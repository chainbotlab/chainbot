use std::io::Write;
use std::time::Duration;

use futures_util::StreamExt;
use reqwest::{Client, Method, Response};
use serde_json::{json, Value};
use tokio::time::{interval, MissedTickBehavior};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

use crate::contract::TriggerStartCommand;

use super::market_stream::handle_socket_message;
use super::normalize::listen_key_expired;
use super::policy::{normalize_base_url, validate_destination_policy};
use super::types::{default_rest_base_url, default_user_ws_base, environment_from_params, product_line_from_params, ProductLine, SocketLoopOutcome};
use super::write_ready;

pub async fn run_user_stream_listener(command: &TriggerStartCommand, stdout: &mut impl Write) -> Result<(), String> {
    let runtime = UserStreamRuntime::from_command(command)?;
    let mut listen_key = runtime.create_user_stream().await?;
    let mut emitted_ready = false;
    let mut reconnect_attempt = 0u32;
    const MAX_STARTUP_RECONNECT_ATTEMPTS: u32 = 5;

    loop {
        let endpoint = runtime.websocket_endpoint(&listen_key);
        let request = endpoint.into_client_request().map_err(|error| error.to_string())?;
        let (mut socket, _) = match connect_async(request).await {
            Ok(value) => value,
            Err(error) => {
                if listen_key_expired(&error.to_string()) {
                    let _ = runtime.close_user_stream(&listen_key).await;
                    listen_key = runtime.create_user_stream().await?;
                }
                reconnect_attempt = reconnect_attempt.saturating_add(1);
                if !emitted_ready && reconnect_attempt >= MAX_STARTUP_RECONNECT_ATTEMPTS {
                    let _ = runtime.close_user_stream(&listen_key).await;
                    return Err(format!(
                        "user stream websocket failed to connect after {MAX_STARTUP_RECONNECT_ATTEMPTS} attempts: {error}"
                    ));
                }
                tokio::time::sleep(reconnect_backoff(reconnect_attempt)).await;
                continue;
            }
        };
        reconnect_attempt = 0;

        if !emitted_ready {
            write_ready(stdout)?;
            emitted_ready = true;
        }

        let mut keepalive = interval(Duration::from_secs(30 * 60));
        keepalive.set_missed_tick_behavior(MissedTickBehavior::Delay);

        let outcome = loop {
            tokio::select! {
                _ = keepalive.tick() => {
                    match runtime.keepalive_user_stream(&listen_key).await {
                        Ok(Some(updated)) if updated != listen_key => {
                            listen_key = updated;
                            break SocketLoopOutcome::Reconnect;
                        }
                        Ok(_) => {}
                        Err(error) if listen_key_expired(&error) => {
                            let _ = runtime.close_user_stream(&listen_key).await;
                            listen_key = runtime.create_user_stream().await?;
                            break SocketLoopOutcome::Reconnect;
                        }
                        Err(error) => return Err(error),
                    }
                }
                message = socket.next() => {
                    let Some(message) = message else {
                        break SocketLoopOutcome::Reconnect;
                    };
                    let message = message.map_err(|error| error.to_string())?;
                    match handle_socket_message(command, stdout, &mut socket, message, None, &mut emitted_ready).await? {
                        super::types::MessageOutcome::Continue => {}
                        super::types::MessageOutcome::Reconnect => break SocketLoopOutcome::Reconnect,
                    }
                }
            }
        };

        if matches!(outcome, SocketLoopOutcome::Reconnect) {
            reconnect_attempt = reconnect_attempt.saturating_add(1);
            tokio::time::sleep(reconnect_backoff(reconnect_attempt)).await;
            continue;
        }
        break;
    }

    let _ = runtime.close_user_stream(&listen_key).await;
    Ok(())
}

fn reconnect_backoff(attempt: u32) -> Duration {
    let capped = attempt.min(5);
    Duration::from_secs(1u64 << capped)
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

struct UserStreamRuntime {
    client: Client,
    product_line: ProductLine,
    rest_base_url: String,
    ws_base_url: String,
    api_key: String,
}

impl UserStreamRuntime {
    fn from_command(command: &TriggerStartCommand) -> Result<Self, String> {
        let product_line = product_line_from_params(&command.params)?;
        let environment = environment_from_params(&command.params)?;
        let activation = command
            .activation
            .as_ref()
            .ok_or_else(|| String::from("activation is required for binance_user_stream"))?;
        let api_key = activation
            .secrets
            .get("api_key")
            .cloned()
            .ok_or_else(|| String::from("activation secret api_key is required"))?;

        let rest_base_url = command
            .params
            .get("base_url")
            .and_then(Value::as_str)
            .map(normalize_base_url)
            .unwrap_or_else(|| String::from(default_rest_base_url(product_line, environment)));
        let ws_base_url = command
            .params
            .get("ws_base_url")
            .or_else(|| command.params.get("endpoint"))
            .and_then(Value::as_str)
            .map(normalize_base_url)
            .unwrap_or_else(|| String::from(default_user_ws_base(product_line, environment)));

        validate_destination_policy(&rest_base_url, &activation.allowed_origins, true)?;
        validate_destination_policy(&ws_base_url, &activation.allowed_origins, true)?;

        Ok(Self {
            client: Client::new(),
            product_line,
            rest_base_url,
            ws_base_url,
            api_key,
        })
    }

    async fn create_user_stream(&self) -> Result<String, String> {
        let payload = self
            .send_api_key_request(Method::POST, self.product_line.user_stream_path(), Vec::new())
            .await?;
        payload
            .get("listenKey")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .ok_or_else(|| String::from("listenKey missing from Binance response"))
    }

    async fn keepalive_user_stream(&self, listen_key: &str) -> Result<Option<String>, String> {
        let payload = self
            .send_api_key_request(
                Method::PUT,
                self.product_line.user_stream_path(),
                vec![(String::from("listenKey"), String::from(listen_key))],
            )
            .await?;
        Ok(payload
            .get("listenKey")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned))
    }

    async fn close_user_stream(&self, listen_key: &str) -> Result<(), String> {
        let _ = self
            .send_api_key_request(
                Method::DELETE,
                self.product_line.user_stream_path(),
                vec![(String::from("listenKey"), String::from(listen_key))],
            )
            .await?;
        Ok(())
    }

    fn websocket_endpoint(&self, listen_key: &str) -> String {
        let base = self.ws_base_url.trim_end_matches('/');
        if base.contains("{listenKey}") {
            return base.replace("{listenKey}", listen_key);
        }
        format!("{base}/{listen_key}")
    }

    async fn send_api_key_request(
        &self,
        method: Method,
        path: &str,
        query: Vec<(String, String)>,
    ) -> Result<Value, String> {
        let url = format!("{}{}", self.rest_base_url.trim_end_matches('/'), path);
        let request = self
            .client
            .request(method, url)
            .header("X-MBX-APIKEY", &self.api_key)
            .query(&query);
        let response = request.send().await.map_err(|error| error.to_string())?;
        parse_json_response(response).await
    }
}

async fn parse_json_response(response: Response) -> Result<Value, String> {
    let status = response.status();
    let body = response.text().await.map_err(|error| error.to_string())?;
    if body.trim().is_empty() {
        if status.is_success() {
            return Ok(json!({}));
        }
        return Err(format!("http {}: empty response body", status.as_u16()));
    }

    let payload: Value = serde_json::from_str(&body).map_err(|error| error.to_string())?;
    if status.is_success() {
        return Ok(payload);
    }

    let code = payload.get("code").and_then(Value::as_i64);
    let message = payload
        .get("msg")
        .and_then(Value::as_str)
        .unwrap_or("request failed");
    match code {
        Some(code) => Err(format!("http {}: [{}] {}", status.as_u16(), code, message)),
        None => Err(format!("http {}: {}", status.as_u16(), message)),
    }
}
