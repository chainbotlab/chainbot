use std::io::Write;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use hmac::{Hmac, Mac};
use serde_json::Value;
use sha2::Sha256;
use tokio::time::{interval, MissedTickBehavior};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};

use crate::contract::{build_subscription_request, TriggerStartCommand};

use super::market_stream::handle_socket_message;
use super::normalize::{current_time_ms, is_text_ping, stream_error, text_pong};
use super::policy::{normalize_base_url, validate_destination_policy};
use super::types::{default_private_ws_url, environment_from_params, MessageOutcome, SocketLoopOutcome};
use super::write_ready;

type HmacSha256 = Hmac<Sha256>;

pub async fn run_user_stream_listener(command: &TriggerStartCommand, stdout: &mut impl Write) -> Result<(), String> {
    let runtime = UserStreamRuntime::from_command(command)?;
    let subscription_request = build_subscription_request(command)?;
    let mut emitted_ready = false;
    let mut reconnect_attempt = 0u32;
    const MAX_STARTUP_RECONNECT_ATTEMPTS: u32 = 5;

    loop {
        let request = runtime
            .ws_base_url
            .clone()
            .into_client_request()
            .map_err(|error| error.to_string())?;
        let (mut socket, _) = match connect_async(request).await {
            Ok(value) => value,
            Err(error) => {
                reconnect_attempt = reconnect_attempt.saturating_add(1);
                if !emitted_ready && reconnect_attempt >= MAX_STARTUP_RECONNECT_ATTEMPTS {
                    return Err(format!(
                        "user stream websocket failed to connect after {MAX_STARTUP_RECONNECT_ATTEMPTS} attempts: {error}"
                    ));
                }
                tokio::time::sleep(reconnect_backoff(reconnect_attempt)).await;
                continue;
            }
        };
        reconnect_attempt = 0;

        authenticate_and_subscribe(&runtime, &mut socket, &subscription_request, &mut emitted_ready, stdout).await?;

        let mut ping_interval = interval(Duration::from_secs(20));
        ping_interval.set_missed_tick_behavior(MissedTickBehavior::Delay);

        let outcome = loop {
            tokio::select! {
                _ = ping_interval.tick() => {
                    socket
                        .send(Message::Text(text_pong().to_string().into()))
                        .await
                        .map_err(|error| error.to_string())?;
                }
                message = socket.next() => {
                    let Some(message) = message else {
                        break SocketLoopOutcome::Reconnect;
                    };
                    let message = message.map_err(|error| error.to_string())?;
                    match &message {
                        Message::Text(text) => {
                            let payload: Value = serde_json::from_str(text).map_err(|error| error.to_string())?;
                            if is_text_ping(&payload) {
                                socket
                                    .send(Message::Text(text_pong().to_string().into()))
                                    .await
                                    .map_err(|error| error.to_string())?;
                                continue;
                            }
                        }
                        Message::Ping(payload) => {
                            socket
                                .send(Message::Pong(payload.clone()))
                                .await
                                .map_err(|error| error.to_string())?;
                            continue;
                        }
                        Message::Close(_) => break SocketLoopOutcome::Reconnect,
                        _ => {}
                    }

                    match handle_socket_message(command, stdout, &mut socket, message, None, &mut emitted_ready).await? {
                        MessageOutcome::Continue => {}
                        MessageOutcome::Reconnect => break SocketLoopOutcome::Reconnect,
                    }
                }
            }
        };

        if matches!(outcome, SocketLoopOutcome::Reconnect) {
            reconnect_attempt = reconnect_attempt.saturating_add(1);
            tokio::time::sleep(reconnect_backoff(reconnect_attempt)).await;
            continue;
        }
    }
}

async fn authenticate_and_subscribe(
    runtime: &UserStreamRuntime,
    socket: &mut tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    subscription_request: &Value,
    emitted_ready: &mut bool,
    stdout: &mut impl Write,
) -> Result<(), String> {
    let expires = current_time_ms()? + 5_000;
    let auth = serde_json::json!({
        "op": "auth",
        "args": [runtime.api_key, expires, runtime.sign(expires)?],
    });
    socket
        .send(Message::Text(auth.to_string().into()))
        .await
        .map_err(|error| error.to_string())?;
    wait_for_ack(socket, "auth").await?;

    socket
        .send(Message::Text(subscription_request.to_string().into()))
        .await
        .map_err(|error| error.to_string())?;
    wait_for_ack(socket, "subscribe").await?;

    if !*emitted_ready {
        write_ready(stdout)?;
        *emitted_ready = true;
    }
    Ok(())
}

async fn wait_for_ack(
    socket: &mut tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    op: &str,
) -> Result<(), String> {
    while let Some(message) = socket.next().await {
        let message = message.map_err(|error| error.to_string())?;
        match message {
            Message::Text(text) => {
                let payload: Value = serde_json::from_str(&text).map_err(|error| error.to_string())?;
                if is_text_ping(&payload) {
                    socket
                        .send(Message::Text(text_pong().to_string().into()))
                        .await
                        .map_err(|error| error.to_string())?;
                    continue;
                }
                if payload.get("op").and_then(Value::as_str) == Some(op)
                    && payload.get("success").and_then(Value::as_bool) == Some(true)
                {
                    return Ok(());
                }
                if let Some((code, message)) = stream_error(&payload) {
                    return Err(match code {
                        Some(code) => format!("bybit stream error [{code}] {message}"),
                        None => format!("bybit stream error {message}"),
                    });
                }
            }
            Message::Ping(payload) => {
                socket
                    .send(Message::Pong(payload))
                    .await
                    .map_err(|error| error.to_string())?;
            }
            Message::Close(_) => return Err(format!("bybit {op} closed before acknowledgement")),
            _ => {}
        }
    }

    Err(format!("bybit {op} closed before acknowledgement"))
}

fn reconnect_backoff(attempt: u32) -> Duration {
    let capped = attempt.min(5);
    Duration::from_secs(1u64 << capped)
}

struct UserStreamRuntime {
    ws_base_url: String,
    api_key: String,
    api_secret: String,
}

impl UserStreamRuntime {
    fn from_command(command: &TriggerStartCommand) -> Result<Self, String> {
        let environment = environment_from_params(&command.params)?;
        let activation = command
            .activation
            .as_ref()
            .ok_or_else(|| String::from("activation is required for bybit_user_stream"))?;
        let api_key = activation
            .secrets
            .get("api_key")
            .cloned()
            .ok_or_else(|| String::from("activation secret api_key is required"))?;
        let api_secret = activation
            .secrets
            .get("api_secret")
            .cloned()
            .ok_or_else(|| String::from("activation secret api_secret is required"))?;
        let ws_base_url = command
            .params
            .get("ws_base_url")
            .or_else(|| command.params.get("endpoint"))
            .and_then(Value::as_str)
            .map(normalize_base_url)
            .unwrap_or_else(|| String::from(default_private_ws_url(environment)));

        validate_destination_policy(&ws_base_url, &activation.allowed_origins, true)?;

        Ok(Self {
            ws_base_url,
            api_key,
            api_secret,
        })
    }

    fn sign(&self, expires: i64) -> Result<String, String> {
        let preimage = format!("GET/realtime{expires}");
        let mut mac = HmacSha256::new_from_slice(self.api_secret.as_bytes()).map_err(|error| error.to_string())?;
        mac.update(preimage.as_bytes());
        Ok(hex::encode(mac.finalize().into_bytes()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconnect_backoff_grows_and_caps() {
        assert_eq!(reconnect_backoff(1), Duration::from_secs(2));
        assert_eq!(reconnect_backoff(2), Duration::from_secs(4));
        assert_eq!(reconnect_backoff(5), Duration::from_secs(32));
        assert_eq!(reconnect_backoff(9), Duration::from_secs(32));
    }

    #[test]
    fn auth_signature_matches_bybit_contract_shape() {
        let runtime = UserStreamRuntime {
            ws_base_url: String::from("wss://stream.bybit.com/v5/private"),
            api_key: String::from("key"),
            api_secret: String::from("secret"),
        };
        let signature = runtime.sign(1_700_000_000_000).expect("signature");
        assert_eq!(signature.len(), 64);
    }
}
