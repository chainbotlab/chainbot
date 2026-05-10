use std::io::Write;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message},
};

use crate::contract::{build_event_frame, build_subscription_request, event_message, TriggerStartCommand};

use super::normalize::{current_time_ms, event_key, event_stream_name, event_time_ms, is_subscription_ack, normalize_payload, stream_error};
use super::policy::validate_destination_policy;
use super::types::{environment_from_params, product_line_from_params, Environment, MessageOutcome, ProductLine, SocketLoopOutcome};
use super::write_ready;

pub async fn run_market_listener(command: &TriggerStartCommand, stdout: &mut impl Write) -> Result<(), String> {
    let endpoint = market_endpoint(command)?;
    let subscription_request = build_subscription_request(command)?;
    let mut emitted_ready = false;

    loop {
        let request = endpoint.clone().into_client_request().map_err(|error| error.to_string())?;
        let (mut socket, _) = connect_async(request).await.map_err(|error| error.to_string())?;
        socket
            .send(Message::Text(subscription_request.to_string().into()))
            .await
            .map_err(|error| error.to_string())?;

        match consume_socket_events(command, stdout, &mut socket, &mut emitted_ready).await? {
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
    socket: &mut tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    emitted_ready: &mut bool,
) -> Result<SocketLoopOutcome, String> {
    while let Some(message) = socket.next().await {
        let message = message.map_err(|error| error.to_string())?;
        match handle_socket_message(command, stdout, socket, message, emitted_ready).await? {
            MessageOutcome::Continue => {}
            MessageOutcome::Reconnect => return Ok(SocketLoopOutcome::Reconnect),
        }
    }
    Ok(SocketLoopOutcome::Reconnect)
}

pub async fn handle_socket_message(
    command: &TriggerStartCommand,
    stdout: &mut impl Write,
    socket: &mut tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    message: Message,
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
                    Some(code) => format!("okx stream error [{code}] {message}"),
                    None => format!("okx stream error {message}"),
                });
            }

            let occurred_at_ms = event_time_ms(&payload).unwrap_or(current_time_ms()?);
            let stream = event_stream_name(command, &payload);
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
                event_message(build_event_frame(checkpoint, event_key, occurred_at_ms, normalized))
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

fn market_endpoint(command: &TriggerStartCommand) -> Result<String, String> {
    if let Some(endpoint) = command.params.get("endpoint").and_then(Value::as_str) {
        let allowed_origins = command
            .activation
            .as_ref()
            .map(|activation| activation.allowed_origins.as_slice())
            .unwrap_or(&[]);
        validate_destination_policy(endpoint, allowed_origins, true)?;
        return Ok(endpoint.to_owned());
    }

    let product_line = product_line_from_params(&command.params)?;
    let environment = environment_from_params(&command.params)?;
    Ok(default_market_ws_url(product_line, environment).to_owned())
}

fn default_market_ws_url(product_line: ProductLine, environment: Environment) -> &'static str {
    super::types::default_market_ws_url(product_line, environment)
}
