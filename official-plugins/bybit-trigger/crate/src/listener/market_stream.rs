use std::io::Write;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message},
};

use crate::contract::{build_event_frame, build_subscription_request, event_message, TriggerStartCommand};

use super::normalize::{
    current_time_ms, event_key, event_stream_name, event_time_ms, is_subscription_ack, is_text_ping,
    normalize_payload, stream_error, text_pong,
};
use super::policy::validate_destination_policy;
use super::types::{
    default_market_ws_url, environment_from_params, product_line_from_params, MessageOutcome,
    SocketLoopOutcome,
};
use super::write_ready;

pub async fn run_market_listener(command: &TriggerStartCommand, stdout: &mut impl Write) -> Result<(), String> {
    let stream_names = requested_stream_names(command)?;
    let endpoint = market_endpoint(command)?;
    let subscription_request = build_subscription_request(command)?;
    let mut emitted_ready = false;

    loop {
        let request = endpoint
            .clone()
            .into_client_request()
            .map_err(|error| error.to_string())?;
        let (mut socket, _) = connect_async(request).await.map_err(|error| error.to_string())?;

        socket
            .send(Message::Text(subscription_request.to_string().into()))
            .await
            .map_err(|error| error.to_string())?;

        match consume_socket_events(command, stdout, &mut socket, Some(stream_names.clone()), &mut emitted_ready).await? {
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
    requested_streams: Option<Vec<String>>,
    emitted_ready: &mut bool,
) -> Result<SocketLoopOutcome, String> {
    while let Some(message) = socket.next().await {
        let message = message.map_err(|error| error.to_string())?;
        match handle_socket_message(command, stdout, socket, message, requested_streams.clone(), emitted_ready).await? {
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
    requested_streams: Option<Vec<String>>,
    emitted_ready: &mut bool,
) -> Result<MessageOutcome, String> {
    match message {
        Message::Text(text) => {
            let payload: Value = serde_json::from_str(&text).map_err(|error| error.to_string())?;
            if is_text_ping(&payload) {
                socket
                    .send(Message::Text(text_pong().to_string().into()))
                    .await
                    .map_err(|error| error.to_string())?;
                return Ok(MessageOutcome::Continue);
            }
            if is_subscription_ack(&payload) {
                if !*emitted_ready {
                    write_ready(stdout)?;
                    *emitted_ready = true;
                }
                return Ok(MessageOutcome::Continue);
            }
            if let Some((code, message)) = stream_error(&payload) {
                return Err(match code {
                    Some(code) => format!("bybit stream error [{code}] {message}"),
                    None => format!("bybit stream error {message}"),
                });
            }

            let occurred_at_ms = event_time_ms(&payload).unwrap_or(current_time_ms()?);
            let stream = event_stream_name(command, &payload, requested_streams.as_deref());
            let key = event_key(command, &stream, &payload)?;
            let checkpoint = format!("{}:{}:{}", command.source, stream, key);
            let normalized = normalize_payload(command, &stream, payload, &key);
            if !*emitted_ready {
                write_ready(stdout)?;
                *emitted_ready = true;
            }
            writeln!(
                stdout,
                "{}",
                event_message(build_event_frame(checkpoint, key, occurred_at_ms, normalized))
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
        .get("args")
        .and_then(Value::as_array)
        .ok_or_else(|| String::from("market subscription args are missing"))
        .and_then(|items| {
            let values = items
                .iter()
                .map(|item| {
                    item.as_str()
                        .map(ToOwned::to_owned)
                        .ok_or_else(|| String::from("subscription args must be strings"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            if values.is_empty() {
                return Err(String::from("subscription args must not be empty"));
            }
            Ok(values)
        })
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
    Ok(default_market_ws_url(product_line, environment))
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    use axum::{
        extract::{ws::{Message as AxumMessage, WebSocket}, State, WebSocketUpgrade},
        response::IntoResponse,
        routing::get,
        Router,
    };
    use serial_test::serial;
    use tokio::net::TcpListener;

    use crate::contract::TriggerStartCommand;

    #[tokio::test]
    #[serial]
    async fn market_listener_reconnects_after_close() {
        let _loopback = LoopbackGuard::set();
        let attempts = Arc::new(AtomicUsize::new(0));
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind test websocket");
        let address = listener.local_addr().expect("local addr");
        let app = Router::new()
            .route("/", get(websocket_handler))
            .with_state(attempts.clone());

        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve websocket app");
        });

        let command: TriggerStartCommand = serde_json::from_value(serde_json::json!({
            "type": "start",
            "protocol_version": "2.0.0",
            "trigger_id": "market-reconnect-check",
            "source": "bybit_market_stream",
            "params": {
                "endpoint": format!("ws://{address}"),
                "product_line": "spot",
                "stream": "publicTrade.BTCUSDT"
            },
            "activation": {
                "allowed_origins": [format!("http://{address}")]
            },
            "heartbeat_interval_ms": 1000,
            "shutdown_grace_ms": 1000
        }))
        .expect("valid trigger command");

        let task = tokio::spawn(async move {
            let mut stdout = Vec::new();
            run_market_listener(&command, &mut stdout).await
        });

        tokio::time::timeout(Duration::from_secs(5), async {
            while attempts.load(Ordering::SeqCst) < 2 {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("listener should reconnect after close");

        task.abort();
        let _ = task.await;
        server.abort();
        let _ = server.await;

        assert!(attempts.load(Ordering::SeqCst) >= 2);
    }

    #[tokio::test]
    #[serial]
    async fn market_listener_rejects_error_frames_instead_of_emitting_events() {
        let _loopback = LoopbackGuard::set();
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind test websocket");
        let address = listener.local_addr().expect("local addr");
        let app = Router::new().route("/", get(error_websocket_handler));

        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve websocket app");
        });

        let command: TriggerStartCommand = serde_json::from_value(serde_json::json!({
            "type": "start",
            "protocol_version": "2.0.0",
            "trigger_id": "market-error-check",
            "source": "bybit_market_stream",
            "params": {
                "endpoint": format!("ws://{address}"),
                "product_line": "spot",
                "stream": "publicTrade.BTCUSDT"
            },
            "activation": {
                "allowed_origins": [format!("http://{address}")]
            },
            "heartbeat_interval_ms": 1000,
            "shutdown_grace_ms": 1000
        }))
        .expect("valid trigger command");

        let mut stdout = Vec::new();
        let error = run_market_listener(&command, &mut stdout)
            .await
            .expect_err("error frame should fail listener");

        server.abort();
        let _ = server.await;

        assert!(error.contains("bybit stream error"));
        assert!(String::from_utf8(stdout).expect("utf8 stdout").trim().is_empty());
    }

    #[test]
    fn market_endpoint_override_requires_allowlisted_origin() {
        let command: TriggerStartCommand = serde_json::from_value(serde_json::json!({
            "type": "start",
            "protocol_version": "2.0.0",
            "trigger_id": "market-endpoint-check",
            "source": "bybit_market_stream",
            "params": {
                "endpoint": "wss://example.com/ws",
                "product_line": "spot",
                "stream": "publicTrade.BTCUSDT"
            },
            "activation": {
                "allowed_origins": ["https://stream.bybit.com:443"]
            },
            "heartbeat_interval_ms": 1000,
            "shutdown_grace_ms": 1000
        }))
        .expect("valid trigger command");

        let error = market_endpoint(&command).expect_err("endpoint should require allowlisted origin");
        assert!(error.contains("not allowlisted"));
    }

    async fn websocket_handler(
        State(attempts): State<Arc<AtomicUsize>>,
        upgrade: WebSocketUpgrade,
    ) -> impl IntoResponse {
        upgrade.on_upgrade(move |socket| websocket_session(socket, attempts))
    }

    async fn error_websocket_handler(upgrade: WebSocketUpgrade) -> impl IntoResponse {
        upgrade.on_upgrade(error_websocket_session)
    }

    async fn websocket_session(mut socket: WebSocket, attempts: Arc<AtomicUsize>) {
        let attempt = attempts.fetch_add(1, Ordering::SeqCst) + 1;
        let _ = socket.recv().await;
        if attempt == 1 {
            let _ = socket.close().await;
            return;
        }

        let _ = socket
            .send(AxumMessage::Text(
                serde_json::json!({"op": "subscribe", "success": true}).to_string().into(),
            ))
            .await;
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    async fn error_websocket_session(mut socket: WebSocket) {
        let _ = socket.recv().await;
        let _ = socket
            .send(AxumMessage::Text(
                serde_json::json!({"success": false, "retCode": 10404, "retMsg": "invalid op"})
                    .to_string()
                    .into(),
            ))
            .await;
        let _ = socket.close().await;
    }

    struct LoopbackGuard;

    impl LoopbackGuard {
        fn set() -> Self {
            unsafe {
                std::env::set_var("CHAINBOT_HTTP_NODE_ALLOW_LOOPBACK_FOR_TESTS", "1");
                std::env::set_var("CHAINBOT_INTERNAL_ALLOW_TEST_DESTINATIONS", "1");
            }
            Self
        }
    }

    impl Drop for LoopbackGuard {
        fn drop(&mut self) {
            unsafe {
                std::env::remove_var("CHAINBOT_HTTP_NODE_ALLOW_LOOPBACK_FOR_TESTS");
                std::env::remove_var("CHAINBOT_INTERNAL_ALLOW_TEST_DESTINATIONS");
            }
        }
    }
}
