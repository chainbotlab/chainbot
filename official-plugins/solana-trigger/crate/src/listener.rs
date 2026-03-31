use std::io::{self, Write};
use std::time::{SystemTime, UNIX_EPOCH};

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message},
};

use crate::contract::{
    build_event_frame, build_subscription_request, event_message, ready_message, TriggerStartCommand,
};

pub async fn run_listener(command: TriggerStartCommand) -> Result<(), String> {
    let mut stdout = io::stdout().lock();
    writeln!(stdout, "{}", ready_message()).map_err(|error| error.to_string())?;
    stdout.flush().map_err(|error| error.to_string())?;

    let endpoint = command
        .params
        .get("endpoint")
        .and_then(Value::as_str)
        .ok_or_else(|| String::from("params.endpoint is required"))?;

    if endpoint.starts_with("mock://") {
        return emit_mock_event(&command, &mut stdout);
    }

    let subscription_request = build_subscription_request(&command)?;
    let request = endpoint
        .into_client_request()
        .map_err(|error| error.to_string())?;
    let (mut socket, _) = connect_async(request).await.map_err(|error| error.to_string())?;
    socket
        .send(Message::Text(subscription_request.to_string().into()))
        .await
        .map_err(|error| error.to_string())?;

    let mut subscription_id = None;
    while let Some(message) = socket.next().await {
        let message = message.map_err(|error| error.to_string())?;
        let Message::Text(text) = message else {
            continue;
        };
        let payload: Value = serde_json::from_str(&text).map_err(|error| error.to_string())?;
        if subscription_id.is_none() {
            if let Some(result) = payload.get("result").and_then(Value::as_u64) {
                subscription_id = Some(result.to_string());
                continue;
            }
        }
        if let Some(params) = payload.get("params") {
            let result = params.get("result").cloned().unwrap_or(Value::Null);
            let event_key = extract_event_key(&command.source, &result);
            let checkpoint = format!(
                "{}:{}",
                subscription_id.as_deref().unwrap_or("subscription"),
                event_key
            );
            let occurred_at_ms = current_time_ms()?;
            let normalized = normalize_payload(&command.source, result);
            writeln!(
                stdout,
                "{}",
                event_message(build_event_frame(checkpoint, event_key, occurred_at_ms, normalized))
            )
            .map_err(|error| error.to_string())?;
            stdout.flush().map_err(|error| error.to_string())?;
            break;
        }
    }
    Ok(())
}

fn emit_mock_event(command: &TriggerStartCommand, stdout: &mut impl Write) -> Result<(), String> {
    let occurred_at_ms = current_time_ms()?;
    let payload = json!({
        "chain": "solana",
        "listener_kind": command.source,
        "network": command.params.get("network").cloned().unwrap_or_else(|| json!("mock")),
    });
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

fn extract_event_key(source: &str, result: &Value) -> String {
    match source {
        "solana_logs" => result
            .get("value")
            .and_then(|value| value.get("signature"))
            .and_then(Value::as_str)
            .unwrap_or("logs")
            .to_owned(),
        "solana_account" => result
            .get("context")
            .and_then(|context| context.get("slot"))
            .and_then(Value::as_u64)
            .map(|slot| slot.to_string())
            .unwrap_or_else(|| String::from("account")),
        "solana_signature" => result
            .get("value")
            .and_then(|value| value.get("err"))
            .map(|_| String::from("signature"))
            .unwrap_or_else(|| String::from("signature")),
        _ => String::from("event"),
    }
}

fn normalize_payload(source: &str, result: Value) -> Value {
    match source {
        "solana_logs" => json!({
            "chain": "solana",
            "listener_kind": "event_log",
            "slot_ref": result.get("context").and_then(|context| context.get("slot")).cloned().unwrap_or(Value::Null),
            "event_id": extract_event_key(source, &result),
            "payload": result,
        }),
        "solana_account" => json!({
            "chain": "solana",
            "listener_kind": "state_change",
            "slot_ref": result.get("context").and_then(|context| context.get("slot")).cloned().unwrap_or(Value::Null),
            "event_id": extract_event_key(source, &result),
            "payload": result,
        }),
        "solana_signature" => json!({
            "chain": "solana",
            "listener_kind": "signature",
            "slot_ref": result.get("context").and_then(|context| context.get("slot")).cloned().unwrap_or(Value::Null),
            "event_id": extract_event_key(source, &result),
            "payload": result,
        }),
        _ => result,
    }
}

fn current_time_ms() -> Result<i64, String> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?;
    i64::try_from(duration.as_millis()).map_err(|error| error.to_string())
}
