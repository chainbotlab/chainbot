use std::io::Write;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::contract::{build_event_frame, event_message, simple_event_payload, TriggerStartCommand};

use super::types::{channel_from_params, stream_hint};

pub fn emit_mock_event(command: &TriggerStartCommand, stdout: &mut impl Write) -> Result<(), String> {
    let channel = channel_from_params(&command.params).unwrap_or_else(|_| {
        if command.source == "gate_spot_user_stream" {
            String::from("spot.orders")
        } else {
            String::from("spot.trades")
        }
    });
    let listener_kind = listener_kind(command.source.as_str());
    let stream = stream_hint(&channel, &command.params);
    let event_id = format!("{}:{}:mock", listener_kind, stream);
    let payload = simple_event_payload(
        "gate",
        "spot",
        listener_kind,
        &stream,
        &event_id,
        json!({
            "mock": true,
            "source": command.source,
            "trigger_id": command.trigger_id,
            "channel": channel,
        }),
    );
    let occurred_at_ms = current_time_ms()?;

    writeln!(
        stdout,
        "{}",
        event_message(build_event_frame(
            format!("{}:mock", command.trigger_id),
            event_id,
            occurred_at_ms,
            payload,
        ))
    )
    .map_err(|error| error.to_string())?;
    stdout.flush().map_err(|error| error.to_string())
}

pub fn listener_kind(source: &str) -> &'static str {
    match source {
        "gate_spot_market_stream" => "market_stream",
        "gate_spot_user_stream" => "user_stream",
        _ => "event_stream",
    }
}

pub fn event_stream_name(command: &TriggerStartCommand, payload: &Value) -> String {
    let channel = payload
        .get("channel")
        .and_then(Value::as_str)
        .or_else(|| command.params.get("channel").and_then(Value::as_str))
        .unwrap_or("spot.stream");

    if let Some(pair) = first_currency_pair(payload) {
        format!("{channel}:{pair}")
    } else {
        channel.to_owned()
    }
}

pub fn event_time_ms(payload: &Value) -> Option<i64> {
    first_number_or_decimal(payload, &["time_ms"]).or_else(|| {
        let result = payload.get("result")?;
        match result {
            Value::Array(items) => items.first().and_then(item_time_ms),
            Value::Object(_) => item_time_ms(result),
            _ => None,
        }
    })
}

pub fn event_key(command: &TriggerStartCommand, stream: &str, payload: &Value) -> Result<String, String> {
    let discriminator = event_discriminator(payload).unwrap_or_else(|| String::from("event"));
    let occurred_at = event_time_ms(payload).unwrap_or(current_time_ms()?);
    Ok(format!(
        "{}:{}:{}",
        listener_kind(command.source.as_str()),
        stream,
        discriminator_or_time(discriminator, occurred_at)
    ))
}

pub fn normalize_payload(command: &TriggerStartCommand, stream: &str, payload: Value, event_key: &str) -> Value {
    let inner_payload = payload.get("result").cloned().unwrap_or(payload);
    simple_event_payload(
        "gate",
        "spot",
        listener_kind(command.source.as_str()),
        stream,
        event_key,
        inner_payload,
    )
}

pub fn is_subscription_ack(payload: &Value) -> bool {
    matches!(payload.get("event").and_then(Value::as_str), Some("subscribe" | "unsubscribe"))
        && payload.get("error").map(Value::is_null).unwrap_or(true)
}

pub fn stream_error(payload: &Value) -> Option<String> {
    let error = payload.get("error")?;
    if error.is_null() {
        return None;
    }

    if let Some(message) = error.get("message").and_then(Value::as_str) {
        if let Some(label) = error.get("label").and_then(Value::as_str) {
            return Some(format!("{label}: {message}"));
        }
        return Some(message.to_owned());
    }

    if let Some(label) = error.get("label").and_then(Value::as_str) {
        return Some(label.to_owned());
    }

    Some(error.to_string())
}

pub fn current_time_ms() -> Result<i64, String> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?;
    i64::try_from(duration.as_millis()).map_err(|error| error.to_string())
}

pub fn current_time_secs() -> Result<i64, String> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?;
    i64::try_from(duration.as_secs()).map_err(|error| error.to_string())
}

fn event_discriminator(payload: &Value) -> Option<String> {
    let item = first_result_item(payload);
    first_string(item, &["id", "order_id", "id_market", "currency", "change_type", "text"])
        .or_else(|| first_number(item, &["id", "id_market", "order_id"]))
}

fn first_currency_pair(payload: &Value) -> Option<String> {
    let item = first_result_item(payload);
    first_string(item, &["currency_pair", "s"])
}

fn first_result_item(payload: &Value) -> &Value {
    let result = payload.get("result").unwrap_or(payload);
    match result {
        Value::Array(items) => items.first().unwrap_or(result),
        _ => result,
    }
}

fn item_time_ms(item: &Value) -> Option<i64> {
    first_number_or_decimal(item, &["create_time_ms", "timestamp_ms", "t"])
        .or_else(|| first_number(item, &["create_time", "timestamp", "time"]).and_then(|value| value.parse::<i64>().ok()).map(|value| value * 1000))
}

fn first_string(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str).map(ToOwned::to_owned))
}

fn first_number(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        value.get(*key).and_then(|item| match item {
            Value::Number(number) => Some(number.to_string()),
            _ => None,
        })
    })
}

fn first_number_or_decimal(value: &Value, keys: &[&str]) -> Option<i64> {
    keys.iter().find_map(|key| {
        value.get(*key).and_then(|item| match item {
            Value::Number(number) => number.as_i64(),
            Value::String(raw) => parse_decimal_millis(raw),
            _ => None,
        })
    })
}

fn parse_decimal_millis(raw: &str) -> Option<i64> {
    let integer = raw.trim().split('.').next()?;
    integer.parse::<i64>().ok()
}

fn discriminator_or_time(discriminator: String, occurred_at_ms: i64) -> String {
    if discriminator == "event" {
        occurred_at_ms.to_string()
    } else {
        discriminator
    }
}
