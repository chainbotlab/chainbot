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
    let auth_header = command
        .activation
        .as_ref()
        .and_then(|activation| activation.secrets.get("rpc_token"))
        .cloned();
    let mut request = endpoint
        .into_client_request()
        .map_err(|error| error.to_string())?;
    if let Some(token) = auth_header {
        request.headers_mut().insert(
            http::header::AUTHORIZATION,
            format!("Bearer {token}")
                .parse::<http::HeaderValue>()
                .map_err(|error| error.to_string())?,
        );
    }

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
            if let Some(result) = payload.get("result").and_then(Value::as_str) {
                subscription_id = Some(result.to_owned());
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
        "source": command.source,
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
        "eth_log" => {
            let tx_hash = result.get("transactionHash").and_then(Value::as_str).unwrap_or("tx");
            let log_index = result.get("logIndex").and_then(Value::as_str).unwrap_or("0x0");
            format!("{tx_hash}:{log_index}")
        }
        "eth_new_head" => result
            .get("hash")
            .and_then(Value::as_str)
            .unwrap_or("head")
            .to_owned(),
        "alchemy_mined_tx" => result
            .get("transactionHash")
            .and_then(Value::as_str)
            .unwrap_or("mined")
            .to_owned(),
        _ => String::from("event"),
    }
}

fn normalize_payload(source: &str, result: Value) -> Value {
    match source {
        "eth_log" => json!({
            "chain": "ethereum",
            "listener_kind": "event_log",
            "block_ref": result.get("blockNumber").cloned().unwrap_or(Value::Null),
            "event_id": extract_event_key(source, &result),
            "payload": result,
        }),
        "eth_new_head" => json!({
            "chain": "ethereum",
            "listener_kind": "state_change",
            "block_ref": result.get("number").cloned().unwrap_or(Value::Null),
            "event_id": extract_event_key(source, &result),
            "payload": result,
        }),
        "alchemy_mined_tx" => json!({
            "chain": "ethereum",
            "listener_kind": "alchemy_mined_tx",
            "block_ref": result.get("blockNumber").cloned().unwrap_or(Value::Null),
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
