use std::io::Write;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message},
};

use crate::contract::{
    build_event_frame, build_subscription_request, event_message, source_channel, TriggerStartCommand,
};

use super::normalize::{
    checkpoint, current_time_ms, event_key, event_time_ms, is_subscription_ack, normalize_payload, stream_error,
};
use super::policy::validate_destination_policy;
use super::write_ready;

enum MessageOutcome {
    Continue,
    Reconnect,
}

enum SocketLoopOutcome {
    Reconnect,
}

pub async fn run_market_listener(command: &TriggerStartCommand, stdout: &mut impl Write) -> Result<(), String> {
    let endpoint = market_endpoint(command)?;
    let subscription_request = build_subscription_request(command)?;
    let expected_channel = source_channel(command.source.as_str())?;
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

        match consume_socket_events(command, stdout, &mut socket, expected_channel, &mut emitted_ready).await? {
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
    expected_channel: &str,
    emitted_ready: &mut bool,
) -> Result<SocketLoopOutcome, String> {
    while let Some(message) = socket.next().await {
        let message = message.map_err(|error| error.to_string())?;
        match handle_socket_message(command, stdout, socket, message, expected_channel, emitted_ready).await? {
            MessageOutcome::Continue => {}
            MessageOutcome::Reconnect => return Ok(SocketLoopOutcome::Reconnect),
        }
    }
    Ok(SocketLoopOutcome::Reconnect)
}

async fn handle_socket_message(
    command: &TriggerStartCommand,
    stdout: &mut impl Write,
    socket: &mut tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    message: Message,
    expected_channel: &str,
    emitted_ready: &mut bool,
) -> Result<MessageOutcome, String> {
    match message {
        Message::Text(text) => {
            let payload: Value = serde_json::from_str(&text).map_err(|error| error.to_string())?;
            if is_subscription_ack(&payload, expected_channel) {
                if !*emitted_ready {
                    write_ready(stdout)?;
                    *emitted_ready = true;
                }
                return Ok(MessageOutcome::Continue);
            }
            if let Some(message) = stream_error(&payload) {
                return Err(format!("hyperliquid stream error {message}"));
            }
            let channel = payload
                .get("channel")
                .and_then(Value::as_str)
                .ok_or_else(|| String::from("hyperliquid message missing channel"))?;
            if channel != expected_channel {
                return Ok(MessageOutcome::Continue);
            }

            let occurred_at_ms = event_time_ms(&payload).unwrap_or(current_time_ms()?);
            let event_key = event_key(command, &payload)?;
            let checkpoint = checkpoint(command, &event_key, &payload)?;
            let normalized = normalize_payload(command, payload, &event_key)?;
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

    Ok(String::from("wss://api.hyperliquid.xyz/ws"))
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    use axum::{
        extract::{
            ws::{Message as AxumMessage, WebSocket},
            State, WebSocketUpgrade,
        },
        response::IntoResponse,
        routing::get,
        Router,
    };
    use serial_test::serial;
    use tokio::net::TcpListener;

    #[tokio::test]
    #[serial]
    async fn market_listener_reconnects_after_close() {
        let _loopback = LoopbackGuard::set();
        let attempts = Arc::new(AtomicUsize::new(0));
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind test websocket");
        let address = listener.local_addr().expect("local addr");
        let app = Router::new().route("/", get(websocket_handler)).with_state(attempts.clone());

        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve websocket app");
        });

        let command: TriggerStartCommand = serde_json::from_value(serde_json::json!({
            "type": "start",
            "protocol_version": "2.0.0",
            "trigger_id": "trades-reconnect-check",
            "source": "hyperliquid_trades",
            "params": {
                "endpoint": format!("ws://{address}/"),
                "coin": "BTC"
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

        assert!(attempts.load(Ordering::SeqCst) >= 2, "expected at least two websocket connection attempts");
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
            "trigger_id": "trades-error-check",
            "source": "hyperliquid_trades",
            "params": {
                "endpoint": format!("ws://{address}/"),
                "coin": "BTC"
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

        assert!(
            error.contains("hyperliquid stream error") || error.contains("invalid subscription"),
            "unexpected error: {error}"
        );
        let output = String::from_utf8(stdout).expect("utf8 stdout");
        let frames = output.lines().collect::<Vec<_>>();
        assert!(frames.is_empty(), "error frame before subscription success should not emit ready or event");
    }

    #[test]
    fn market_endpoint_override_requires_allowlisted_origin() {
        let command: TriggerStartCommand = serde_json::from_value(serde_json::json!({
            "type": "start",
            "protocol_version": "2.0.0",
            "trigger_id": "trades-endpoint-check",
            "source": "hyperliquid_trades",
            "params": {
                "endpoint": "wss://example.com/ws",
                "coin": "BTC"
            },
            "activation": {
                "allowed_origins": ["https://api.hyperliquid.xyz"]
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
                serde_json::json!({
                    "channel": "subscriptionResponse",
                    "data": { "subscription": { "type": "trades", "coin": "BTC" } }
                })
                .to_string()
                .into(),
            ))
            .await;
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    async fn error_websocket_session(mut socket: WebSocket) {
        let _ = socket.recv().await;
        let _ = socket
            .send(AxumMessage::Text(
                serde_json::json!({"error": "invalid subscription"}).to_string().into(),
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
