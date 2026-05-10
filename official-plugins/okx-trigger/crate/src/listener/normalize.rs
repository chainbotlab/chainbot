use std::io::Write;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::contract::{build_event_frame, event_message, TriggerStartCommand};

pub fn emit_mock_event(command: &TriggerStartCommand, stdout: &mut impl Write) -> Result<(), String> {
    let product_line = command
        .params
        .get("inst_type")
        .and_then(Value::as_str)
        .unwrap_or("SPOT")
        .to_owned();
    let listener_kind = listener_kind(command.source.as_str()).to_owned();
    let stream = command
        .params
        .get("channel")
        .and_then(Value::as_str)
        .unwrap_or(listener_kind.as_str())
        .to_owned();
    let event_id = format!("{}:{}:event", listener_kind, command.trigger_id);
    let payload = json!({
        "exchange": "okx",
        "product_line": product_line,
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
        "okx_market_stream" => "market_stream",
        "okx_private_stream" => "private_stream",
        _ => "event_stream",
    }
}

pub fn event_stream_name(command: &TriggerStartCommand, payload: &Value) -> String {
    if let Some(channel) = payload.get("arg").and_then(|arg| arg.get("channel")).and_then(Value::as_str) {
        return channel.to_owned();
    }
    command
        .params
        .get("channel")
        .and_then(Value::as_str)
        .unwrap_or("stream")
        .to_owned()
}

pub fn event_time_ms(payload: &Value) -> Option<i64> {
    payload
        .get("data")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .and_then(|item| {
            item.get("ts")
                .and_then(Value::as_str)
                .and_then(|value| value.parse::<i64>().ok())
                .or_else(|| item.get("pTime").and_then(Value::as_str).and_then(|value| value.parse::<i64>().ok()))
        })
}

pub fn event_key(command: &TriggerStartCommand, stream: &str, payload: &Value) -> Result<String, String> {
    let discriminator = payload
        .get("arg")
        .and_then(|arg| arg.get("instId"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| payload.get("event").and_then(Value::as_str).map(str::to_owned))
        .unwrap_or_else(|| String::from("event"));
    let occurred_at = event_time_ms(payload).unwrap_or(current_time_ms()?);
    Ok(format!(
        "{}:{}:{}",
        listener_kind(command.source.as_str()),
        stream,
        if discriminator == "event" { occurred_at.to_string() } else { discriminator }
    ))
}

pub fn normalize_payload(command: &TriggerStartCommand, stream: &str, payload: Value, event_key: &str) -> Value {
    let product_line = command
        .params
        .get("inst_type")
        .and_then(Value::as_str)
        .unwrap_or("SPOT");
    let listener_kind = listener_kind(command.source.as_str());
    let inner_payload = payload.get("data").cloned().unwrap_or(payload);
    json!({
        "exchange": "okx",
        "product_line": product_line,
        "listener_kind": listener_kind,
        "stream": stream,
        "event_id": event_key,
        "payload": inner_payload,
    })
}

pub fn is_subscription_ack(payload: &Value) -> bool {
    payload.get("event").and_then(Value::as_str) == Some("subscribe")
}

pub fn is_login_ack(payload: &Value) -> bool {
    payload.get("event").and_then(Value::as_str) == Some("login")
        && payload.get("code").and_then(Value::as_str) == Some("0")
}

pub fn stream_error(payload: &Value) -> Option<(Option<i64>, String)> {
    let code = payload
        .get("code")
        .and_then(Value::as_str)
        .and_then(|value| value.parse::<i64>().ok())
        .or_else(|| payload.get("code").and_then(Value::as_i64));
    let message = payload
        .get("msg")
        .or_else(|| payload.get("message"))
        .and_then(Value::as_str)
        .map(str::to_owned);

    match (code, message) {
        (Some(0), Some(_)) if is_login_ack(payload) => None,
        (Some(code), Some(message)) if code != 0 => Some((Some(code), message)),
        (Some(code), None) if code != 0 => Some((Some(code), String::from("stream request failed"))),
        (None, Some(message)) => Some((None, message)),
        _ => None,
    }
}

pub fn current_time_ms() -> Result<i64, String> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?;
    i64::try_from(duration.as_millis()).map_err(|error| error.to_string())
}
