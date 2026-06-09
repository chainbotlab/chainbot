use std::io::{self, Write};
use std::time::{SystemTime, UNIX_EPOCH};

use alloy::primitives::U256;
use reqwest::Client;
use serde_json::{json, Value};
use tokio::time::{sleep, Duration};

use crate::contract::{
    build_event_frame, event_message, ready_message, TriggerEventFrame, TriggerStartCommand,
};
use crate::provider::{
    parse_address, parse_path, parse_u256_dec, quote_amounts_out,
};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PriceCheck {
    pub amount_out: U256,
    pub threshold_out: U256,
    pub comparison: String,
    pub triggered: bool,
}

pub async fn run_listener(command: TriggerStartCommand) -> Result<(), String> {
    validate_command(&command)?;
    let mut stdout = io::stdout().lock();
    writeln!(stdout, "{}", ready_message()).map_err(|error| error.to_string())?;
    stdout.flush().map_err(|error| error.to_string())?;

    let client = http_client()?;
    let max_polls = command
        .params
        .get("max_polls")
        .and_then(Value::as_u64)
        .unwrap_or(1);
    let interval_ms = command
        .params
        .get("interval_ms")
        .and_then(Value::as_u64)
        .unwrap_or(5_000);

    for poll_index in 0..max_polls {
        let check = poll_once(&client, &command).await?;
        if check.triggered {
            let frame = price_event_frame(&command, &check, current_time_ms()?);
            writeln!(stdout, "{}", event_message(frame)).map_err(|error| error.to_string())?;
            stdout.flush().map_err(|error| error.to_string())?;
            break;
        }
        if poll_index + 1 < max_polls {
            sleep(Duration::from_millis(interval_ms)).await;
        }
    }

    Ok(())
}

fn http_client() -> Result<Client, String> {
    Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .connect_timeout(CONNECT_TIMEOUT)
        .build()
        .map_err(|error| error.to_string())
}

fn validate_command(command: &TriggerStartCommand) -> Result<(), String> {
    if command.source != "uniswap_price_threshold" {
        return Err(format!("unsupported Uniswap trigger source {}", command.source));
    }
    let _endpoint = param_string(command, "endpoint")?;
    let _router = parse_address(param_string(command, "router")?, "params.router")?;
    let _amount_in = parse_u256_dec(param_string(command, "amount_in")?, "params.amount_in")?;
    let _threshold_out = parse_u256_dec(param_string(command, "threshold_out")?, "params.threshold_out")?;
    let comparison = command
        .params
        .get("comparison")
        .and_then(Value::as_str)
        .unwrap_or("gte");
    match comparison {
        "gte" | "lte" => {}
        other => return Err(format!("comparison must be gte or lte, got {other}")),
    }
    command
        .params
        .get("path")
        .ok_or_else(|| String::from("params.path is required"))
        .and_then(parse_path)?;
    Ok(())
}

pub async fn poll_once(client: &Client, command: &TriggerStartCommand) -> Result<PriceCheck, String> {
    if command.source != "uniswap_price_threshold" {
        return Err(format!("unsupported Uniswap trigger source {}", command.source));
    }
    let endpoint = param_string(command, "endpoint")?;
    let router = parse_address(param_string(command, "router")?, "params.router")?;
    let amount_in = parse_u256_dec(param_string(command, "amount_in")?, "params.amount_in")?;
    let threshold_out = parse_u256_dec(param_string(command, "threshold_out")?, "params.threshold_out")?;
    let comparison = command
        .params
        .get("comparison")
        .and_then(Value::as_str)
        .unwrap_or("gte");
    let block_tag = command
        .params
        .get("block_tag")
        .and_then(Value::as_str)
        .unwrap_or("latest");
    let path = command
        .params
        .get("path")
        .ok_or_else(|| String::from("params.path is required"))
        .and_then(parse_path)?;
    let amounts = quote_amounts_out(&client, endpoint, router, amount_in, &path, block_tag).await?;
    let amount_out = amounts
        .last()
        .copied()
        .ok_or_else(|| String::from("router returned no output amount"))?;
    let triggered = evaluate(amount_out, threshold_out, comparison)?;
    Ok(PriceCheck {
        amount_out,
        threshold_out,
        comparison: comparison.to_owned(),
        triggered,
    })
}

pub fn price_event_frame(
    command: &TriggerStartCommand,
    check: &PriceCheck,
    occurred_at_ms: i64,
) -> TriggerEventFrame {
    let router = command.params.get("router").cloned().unwrap_or(Value::Null);
    let path = command.params.get("path").cloned().unwrap_or(Value::Null);
    let amount_in = command.params.get("amount_in").cloned().unwrap_or(Value::Null);
    let event_key = format!(
        "uniswap_price_threshold:{}:{}:{}",
        router.as_str().unwrap_or("router"),
        check.comparison,
        check.threshold_out
    );
    let payload = json!({
        "chain": command.params.get("chain").cloned().unwrap_or_else(|| json!("evm")),
        "listener_kind": "price_threshold",
        "router": router,
        "path": path,
        "amount_in": amount_in,
        "amount_out": check.amount_out.to_string(),
        "threshold_out": check.threshold_out.to_string(),
        "comparison": check.comparison,
        "triggered": check.triggered,
    });
    build_event_frame(
        format!("{}:{}", command.trigger_id, check.amount_out),
        event_key,
        occurred_at_ms,
        payload,
    )
}

fn evaluate(amount_out: U256, threshold_out: U256, comparison: &str) -> Result<bool, String> {
    match comparison {
        "gte" => Ok(amount_out >= threshold_out),
        "lte" => Ok(amount_out <= threshold_out),
        other => Err(format!("comparison must be gte or lte, got {other}")),
    }
}

fn param_string<'a>(command: &'a TriggerStartCommand, key: &str) -> Result<&'a str, String> {
    command
        .params
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("params.{key} is required"))
}

fn current_time_ms() -> Result<i64, String> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?;
    i64::try_from(duration.as_millis()).map_err(|error| error.to_string())
}
