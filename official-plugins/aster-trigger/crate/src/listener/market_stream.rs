use std::io::Write;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message},
};

use crate::contract::{
    build_event_frame, build_subscription_request, event_message, TriggerStartCommand,
};

use super::normalize::{
    current_time_ms, event_key, event_stream_name, event_time_ms, is_subscription_ack,
    normalize_payload, stream_error,
};
use super::policy::validate_destination_policy;
use super::types::{MessageOutcome, SocketLoopOutcome, DEFAULT_MARKET_WS_BASE_URL};
use super::write_ready;

pub async fn run_market_listener(
    command: &TriggerStartCommand,
    stdout: &mut impl Write,
) -> Result<(), String> {
    let stream_names = requested_stream_names(command)?;
    let endpoint = market_endpoint(command, &stream_names)?;
    let subscription_request = if has_endpoint_override(command) {
        build_subscription_request(command)?
    } else {
        Value::Null
    };
    let mut emitted_ready = false;

    loop {
        let request = endpoint
            .clone()
            .into_client_request()
            .map_err(|error| error.to_string())?;
        let (mut socket, _) = connect_async(request)
            .await
            .map_err(|error| error.to_string())?;

        if !subscription_request.is_null() {
            socket
                .send(Message::Text(subscription_request.to_string().into()))
                .await
                .map_err(|error| error.to_string())?;
        }

        match consume_socket_events(
            command,
            stdout,
            &mut socket,
            Some(stream_names.clone()),
            &mut emitted_ready,
        )
        .await?
        {
            SocketLoopOutcome::Reconnect => {
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            }
        }
    }
}

async fn consume_socket_events(
    command: &TriggerStartCommand,
    stdout: &mut impl Write,
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    requested_streams: Option<Vec<String>>,
    emitted_ready: &mut bool,
) -> Result<SocketLoopOutcome, String> {
    while let Some(message) = socket.next().await {
        let message = message.map_err(|error| error.to_string())?;
        match handle_socket_message(
            command,
            stdout,
            socket,
            message,
            requested_streams.clone(),
            emitted_ready,
        )
        .await?
        {
            MessageOutcome::Continue => {}
            MessageOutcome::Reconnect => return Ok(SocketLoopOutcome::Reconnect),
        }
    }
    Ok(SocketLoopOutcome::Reconnect)
}

async fn handle_socket_message(
    command: &TriggerStartCommand,
    stdout: &mut impl Write,
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    message: Message,
    requested_streams: Option<Vec<String>>,
    emitted_ready: &mut bool,
) -> Result<MessageOutcome, String> {
    match message {
        Message::Text(text) => {
            let payload: Value = serde_json::from_str(&text).map_err(|error| error.to_string())?;
            if is_subscription_ack(&payload) {
                if !*emitted_ready {
                    write_ready(stdout)?;
                    *emitted_ready = true;
                }
                return Ok(MessageOutcome::Continue);
            }
            if let Some((code, message)) = stream_error(&payload) {
                return Err(match code {
                    Some(code) => format!("aster stream error [{code}] {message}"),
                    None => format!("aster stream error {message}"),
                });
            }

            let occurred_at_ms = event_time_ms(&payload).unwrap_or(current_time_ms()?);
            let stream = event_stream_name(command, &payload, requested_streams.as_deref());
            let event_key = event_key(command, &stream, &payload)?;
            let checkpoint = format!("{}:{}:{}", command.source, stream, event_key);
            let normalized = normalize_payload(command, &stream, payload, &event_key);
            if !*emitted_ready {
                write_ready(stdout)?;
                *emitted_ready = true;
            }
            writeln!(
                stdout,
                "{}",
                event_message(build_event_frame(
                    checkpoint,
                    event_key,
                    occurred_at_ms,
                    normalized,
                ))
            )
            .map_err(|error| error.to_string())?;
            stdout.flush().map_err(|error| error.to_string())?;
            Ok(MessageOutcome::Continue)
        }
        Message::Ping(payload) => {
            socket
                .send(Message::Pong(payload))
                .await
                .map_err(|error| error.to_string())?;
            Ok(MessageOutcome::Continue)
        }
        Message::Close(_) => Ok(MessageOutcome::Reconnect),
        _ => Ok(MessageOutcome::Continue),
    }
}

fn requested_stream_names(command: &TriggerStartCommand) -> Result<Vec<String>, String> {
    build_subscription_request(command)?
        .get("params")
        .and_then(Value::as_array)
        .ok_or_else(|| String::from("market subscription params are missing"))
        .and_then(|items| {
            let values = items
                .iter()
                .map(|item| {
                    item.as_str()
                        .map(ToOwned::to_owned)
                        .ok_or_else(|| String::from("subscription params must be strings"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            if values.is_empty() {
                return Err(String::from("subscription params must not be empty"));
            }
            Ok(values)
        })
}

fn market_endpoint(command: &TriggerStartCommand, stream_names: &[String]) -> Result<String, String> {
    if let Some(endpoint) = command.params.get("endpoint").and_then(Value::as_str) {
        let allowed_origins = command
            .activation
            .as_ref()
            .map(|activation| activation.allowed_origins.as_slice())
            .unwrap_or(&[]);
        validate_destination_policy(endpoint, allowed_origins, true)?;
        return Ok(endpoint.to_owned());
    }

    Ok(default_market_ws_url(stream_names))
}

fn default_market_ws_url(stream_names: &[String]) -> String {
    if stream_names.len() == 1 {
        format!("{DEFAULT_MARKET_WS_BASE_URL}/ws/{}", stream_names[0])
    } else {
        format!(
            "{DEFAULT_MARKET_WS_BASE_URL}/stream?streams={}",
            stream_names.join("/")
        )
    }
}

fn has_endpoint_override(command: &TriggerStartCommand) -> bool {
    command.params.get("endpoint").and_then(Value::as_str).is_some()
}
