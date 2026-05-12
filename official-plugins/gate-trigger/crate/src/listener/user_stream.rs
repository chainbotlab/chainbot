use std::io::Write;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use hmac::{Hmac, Mac};
use serde::Serialize;
use serde_json::Value;
use sha2::Sha512;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};

use crate::contract::TriggerStartCommand;

use super::market_stream::{handle_socket_message, user_subscribe_request};
use super::normalize::current_time_secs;
use super::policy::{normalize_base_url, validate_destination_policy};
use super::types::{channel_from_params, payload_from_params, validate_user_channel, MessageOutcome, SocketLoopOutcome, DEFAULT_SPOT_WS_URL};

type HmacSha512 = Hmac<Sha512>;

pub async fn run_user_stream_listener(command: &TriggerStartCommand, stdout: &mut impl Write) -> Result<(), String> {
    let runtime = UserStreamRuntime::from_command(command)?;
    let mut emitted_ready = false;
    let mut reconnect_attempt = 0u32;

    loop {
        let request = runtime
            .endpoint
            .clone()
            .into_client_request()
            .map_err(|error| error.to_string())?;

        let (mut socket, _) = match connect_async(request).await {
            Ok(value) => value,
            Err(error) => {
                reconnect_attempt = reconnect_attempt.saturating_add(1);
                if !emitted_ready && reconnect_attempt >= 5 {
                    return Err(format!(
                        "user stream websocket failed to connect after 5 attempts: {error}"
                    ));
                }
                tokio::time::sleep(reconnect_backoff(reconnect_attempt)).await;
                continue;
            }
        };
        reconnect_attempt = 0;

        let request = runtime.subscription_request()?;
        socket
            .send(Message::Text(request.to_string().into()))
            .await
            .map_err(|error| error.to_string())?;

        let outcome = loop {
            let Some(message) = socket.next().await else {
                break SocketLoopOutcome::Reconnect;
            };
            let message = message.map_err(|error| error.to_string())?;
            match handle_socket_message(command, stdout, &mut socket, message, &mut emitted_ready).await? {
                MessageOutcome::Continue => {}
                MessageOutcome::Reconnect => break SocketLoopOutcome::Reconnect,
            }
        };

        if matches!(outcome, SocketLoopOutcome::Reconnect) {
            reconnect_attempt = reconnect_attempt.saturating_add(1);
            tokio::time::sleep(reconnect_backoff(reconnect_attempt)).await;
            continue;
        }
    }
}

fn reconnect_backoff(attempt: u32) -> Duration {
    let capped = attempt.min(5);
    Duration::from_secs(1u64 << capped)
}

struct UserStreamRuntime {
    endpoint: String,
    channel: String,
    payload: Option<Value>,
    api_key: String,
    api_secret: String,
}

impl UserStreamRuntime {
    fn from_command(command: &TriggerStartCommand) -> Result<Self, String> {
        let activation = command
            .activation
            .as_ref()
            .ok_or_else(|| String::from("activation is required for gate_spot_user_stream"))?;

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

        let channel = channel_from_params(&command.params)?;
        validate_user_channel(&channel)?;
        let payload = payload_from_params(&channel, &command.params)?;

        let endpoint = command
            .params
            .get("endpoint")
            .and_then(Value::as_str)
            .map(normalize_base_url)
            .unwrap_or_else(|| String::from(DEFAULT_SPOT_WS_URL.trim_end_matches('/')));

        validate_destination_policy(&endpoint, &activation.allowed_origins, true)?;

        Ok(Self {
            endpoint,
            channel,
            payload,
            api_key,
            api_secret,
        })
    }

    fn subscription_request(&self) -> Result<Value, String> {
        let timestamp = current_time_secs()?;
        let auth = sign_auth(&self.channel, "subscribe", timestamp, &self.api_key, &self.api_secret)?;
        user_subscribe_request(&self.channel, self.payload.clone(), serde_json::to_value(auth).map_err(|error| error.to_string())?)
    }
}

#[derive(Debug, Serialize)]
struct GateAuth<'a> {
    method: &'static str,
    #[serde(rename = "KEY")]
    key: &'a str,
    #[serde(rename = "SIGN")]
    sign: String,
}

fn sign_auth<'a>(channel: &str, event: &str, timestamp: i64, api_key: &'a str, api_secret: &str) -> Result<GateAuth<'a>, String> {
    let payload = format!("channel={channel}&event={event}&time={timestamp}");
    let mut mac = HmacSha512::new_from_slice(api_secret.as_bytes()).map_err(|error| error.to_string())?;
    mac.update(payload.as_bytes());
    let sign = hex::encode(mac.finalize().into_bytes());
    Ok(GateAuth {
        method: "api_key",
        key: api_key,
        sign,
    })
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
}
