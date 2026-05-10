use std::io::Write;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::contract::{build_event_frame, event_message, source_channel, TriggerStartCommand};

pub fn emit_mock_event(command: &TriggerStartCommand, stdout: &mut impl Write) -> Result<(), String> {
    let channel = source_channel(command.source.as_str())?;
    let coin = command
        .params
        .get("coin")
        .and_then(Value::as_str)
        .unwrap_or(channel)
        .to_owned();
    let event_id = format!("{}:{}:event", channel, command.trigger_id);
    let payload = json!({
        "exchange": "hyperliquid",
        "listener_kind": "market_stream",
        "channel": channel,
        "coin": coin,
        "event_id": event_id,
        "payload": {
            "mock": true,
            "source": command.source,
            "trigger_id": command.trigger_id,
        },
    });
    let occurred_at_ms = current_time_ms()?;
    writeln!(
        stdout,
        "{}",
        event_message(build_event_frame(
            format!("{}:mock", command.trigger_id),
            format!("{}:event", command.source),
            occurred_at_ms,
            payload,
        ))
    )
    .map_err(|error| error.to_string())?;
    stdout.flush().map_err(|error| error.to_string())
}

pub fn is_subscription_ack(payload: &Value, expected_channel: &str) -> bool {
    payload
        .get("channel")
        .and_then(Value::as_str)
        == Some("subscriptionResponse")
        && payload
            .get("data")
            .and_then(|data| data.get("subscription"))
            .and_then(|subscription| subscription.get("type"))
            .and_then(Value::as_str)
            == Some(expected_channel)
}

pub fn stream_error(payload: &Value) -> Option<String> {
    payload
        .get("error")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| {
            payload
                .get("data")
                .and_then(|data| data.get("error"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
}

pub fn event_time_ms(payload: &Value) -> Option<i64> {
    let data = payload.get("data")?;
    match payload.get("channel").and_then(Value::as_str) {
        Some("trades") => data
            .as_array()
            .and_then(|items| items.first())
            .and_then(|item| item.get("time"))
            .and_then(Value::as_i64),
        Some("l2Book") => data.get("time").and_then(Value::as_i64),
        _ => None,
    }
}

pub fn event_key(command: &TriggerStartCommand, payload: &Value) -> Result<String, String> {
    let channel = source_channel(command.source.as_str())?;
    let coin = event_coin(command, payload).unwrap_or_else(|| String::from("unknown"));
    let discriminator = match channel {
        "trades" => payload
            .get("data")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .and_then(|item| item.get("tid"))
            .and_then(Value::as_i64)
            .map(|value| value.to_string()),
        "l2Book" => payload
            .get("data")
            .and_then(|data| data.get("time"))
            .and_then(Value::as_i64)
            .map(|value| value.to_string()),
        _ => None,
    }
    .unwrap_or_else(|| event_time_ms(payload).unwrap_or(0).to_string());

    Ok(format!("market_stream:{channel}:{coin}:{discriminator}"))
}

pub fn checkpoint(command: &TriggerStartCommand, event_key: &str, payload: &Value) -> Result<String, String> {
    let channel = source_channel(command.source.as_str())?;
    let coin = event_coin(command, payload).unwrap_or_else(|| String::from("unknown"));
    Ok(format!("{}:{channel}:{coin}:{event_key}", command.source))
}

pub fn normalize_payload(command: &TriggerStartCommand, payload: Value, event_key: &str) -> Result<Value, String> {
    let channel = source_channel(command.source.as_str())?;
    let coin = event_coin(command, &payload).unwrap_or_else(|| String::from("unknown"));
    let inner_payload = payload.get("data").cloned().unwrap_or(payload);
    Ok(json!({
        "exchange": "hyperliquid",
        "listener_kind": "market_stream",
        "channel": channel,
        "coin": coin,
        "event_id": event_key,
        "payload": inner_payload,
    }))
}

fn event_coin(command: &TriggerStartCommand, payload: &Value) -> Option<String> {
    let data = payload.get("data");
    data.and_then(|value| match value {
        Value::Object(_) => value.get("coin").and_then(Value::as_str).map(ToOwned::to_owned),
        Value::Array(items) => items.first().and_then(|item| item.get("coin")).and_then(Value::as_str).map(ToOwned::to_owned),
        _ => None,
    })
    .or_else(|| {
        command
            .params
            .get("coin")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
    })
}

pub fn current_time_ms() -> Result<i64, String> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?;
    i64::try_from(duration.as_millis()).map_err(|error| error.to_string())
}
