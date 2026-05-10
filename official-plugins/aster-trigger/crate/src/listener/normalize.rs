use std::io::Write;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::contract::{build_event_frame, event_message, TriggerStartCommand};

pub fn emit_mock_event(command: &TriggerStartCommand, stdout: &mut impl Write) -> Result<(), String> {
    let listener_kind = listener_kind(command.source.as_str()).to_owned();
    let stream = command
        .params
        .get("stream")
        .and_then(Value::as_str)
        .unwrap_or(listener_kind.as_str())
        .to_owned();
    let event_id = format!("{}:{}:event", listener_kind, command.trigger_id);
    let payload = json!({
        "exchange": "aster",
        "listener_kind": listener_kind,
        "stream": stream,
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

pub fn listener_kind(source: &str) -> &'static str {
    match source {
        "aster_market_stream" => "market_stream",
        _ => "event_stream",
    }
}

pub fn event_stream_name(
    command: &TriggerStartCommand,
    payload: &Value,
    requested_streams: Option<&[String]>,
) -> String {
    if let Some(stream) = payload.get("stream").and_then(Value::as_str) {
        return stream.to_owned();
    }

    if let Some(stream) = command.params.get("stream").and_then(Value::as_str) {
        return stream.to_lowercase();
    }

    if let Some(streams) = requested_streams {
        if let Some(first) = streams.first() {
            return first.clone();
        }
    }

    String::from("market_stream")
}

pub fn event_time_ms(payload: &Value) -> Option<i64> {
    let event = payload.get("data").unwrap_or(payload);
    event
        .get("E")
        .and_then(Value::as_i64)
        .or_else(|| event.get("T").and_then(Value::as_i64))
}

pub fn event_key(command: &TriggerStartCommand, stream: &str, payload: &Value) -> Result<String, String> {
    let event = payload.get("data").unwrap_or(payload);
    let discriminator = first_string(event, &["subscriptionId", "i", "c", "u", "t", "a", "s"])
        .or_else(|| first_number(event, &["i", "u", "t", "E", "T"]))
        .or_else(|| event.get("e").and_then(Value::as_str).map(ToOwned::to_owned))
        .unwrap_or_else(|| String::from("event"));
    let occurred_at = event_time_ms(payload).unwrap_or(current_time_ms()?);
    Ok(format!(
        "{}:{}:{}",
        listener_kind(command.source.as_str()),
        stream,
        discriminator_or_time(discriminator, occurred_at)
    ))
}

fn discriminator_or_time(discriminator: String, occurred_at_ms: i64) -> String {
    if discriminator == "event" {
        occurred_at_ms.to_string()
    } else {
        discriminator
    }
}

pub fn normalize_payload(command: &TriggerStartCommand, stream: &str, payload: Value, event_key: &str) -> Value {
    let listener_kind = listener_kind(command.source.as_str());
    let inner_payload = payload.get("data").cloned().unwrap_or(payload);
    json!({
        "exchange": "aster",
        "listener_kind": listener_kind,
        "stream": stream,
        "event_id": event_key,
        "payload": inner_payload,
    })
}

pub fn is_subscription_ack(payload: &Value) -> bool {
    payload.get("id").is_some() && payload.get("result").is_some() && payload.get("stream").is_none()
}

pub fn stream_error(payload: &Value) -> Option<(Option<i64>, String)> {
    let code = payload.get("code").and_then(Value::as_i64);
    let message = payload
        .get("msg")
        .or_else(|| payload.get("message"))
        .and_then(Value::as_str)
        .map(str::to_owned);

    match (code, message) {
        (Some(code), Some(message)) => Some((Some(code), message)),
        (Some(code), None) => Some((Some(code), String::from("stream request failed"))),
        (None, Some(message)) => Some((None, message)),
        (None, None) => None,
    }
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

pub fn current_time_ms() -> Result<i64, String> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?;
    i64::try_from(duration.as_millis()).map_err(|error| error.to_string())
}
