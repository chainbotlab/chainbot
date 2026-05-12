use std::net::SocketAddr;

use axum::{
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    routing::get,
    Router,
};
use bitget_trigger_official_plugin::{contract::parse_start_command, listener::run_listener_with_writer};
use futures_util::{FutureExt, SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::TcpListener;

#[tokio::test]
#[serial_test::serial]
async fn bitget_market_mock_listener_emits_ready_and_event() {
    let command = serde_json::json!({
        "type": "start",
        "protocol_version": "2.0.0",
        "trigger_id": "bitget-market-trigger",
        "source": "bitget_market_stream",
        "params": {"endpoint": "mock://bitget-market", "product_line": "spot", "stream": "ticker"},
        "activation": {"allowed_origins": ["https://ws.bitget.com:443"]},
        "heartbeat_interval_ms": 1000,
        "shutdown_grace_ms": 1000
    });

    let mut stdout = Vec::new();
    run_listener_with_writer(parse_start_command(&command.to_string()).expect("valid start command"), &mut stdout)
        .await
        .expect("mock bitget market listener should succeed");

    let frames = output_lines(&stdout);
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[0]["type"], "ready");
    assert_eq!(frames[1]["type"], "event");
    assert_eq!(frames[1]["payload"]["exchange"], "bitget");
}

#[tokio::test]
#[serial_test::serial]
async fn bitget_user_mock_listener_emits_ready_and_event() {
    let command = serde_json::json!({
        "type": "start",
        "protocol_version": "2.0.0",
        "trigger_id": "bitget-user-trigger",
        "source": "bitget_user_stream",
        "params": {"endpoint": "mock://bitget-user", "product_line": "futures", "stream": "user_data"},
        "activation": {"allowed_origins": ["https://ws.bitget.com:443"]},
        "heartbeat_interval_ms": 1000,
        "shutdown_grace_ms": 1000
    });

    let mut stdout = Vec::new();
    run_listener_with_writer(parse_start_command(&command.to_string()).expect("valid start command"), &mut stdout)
        .await
        .expect("mock bitget user listener should succeed");

    let frames = output_lines(&stdout);
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[0]["type"], "ready");
    assert_eq!(frames[1]["payload"]["exchange"], "bitget");
}

#[tokio::test]
#[serial_test::serial]
async fn live_market_listener_subscribes_and_emits_event() {
    let _loopback = LoopbackGuard::set();
    let server = WsTestServer::spawn_market().await;
    let command = json!({
        "type": "start",
        "protocol_version": "2.0.0",
        "trigger_id": "market-live",
        "source": "bitget_market_stream",
        "params": {
            "endpoint": server.endpoint,
            "instType": "SPOT",
            "channel": "ticker",
            "instId": "BTCUSDT"
        },
        "heartbeat_interval_ms": 1000,
        "shutdown_grace_ms": 1000
    });
    let mut stdout = Vec::new();
    run_listener_with_writer(parse_start_command(&command.to_string()).expect("valid start command"), &mut stdout)
        .await
        .expect("live market listener should succeed");

    let frames = output_lines(&stdout);
    assert_eq!(frames[0]["type"], "ready");
    assert_eq!(frames[1]["type"], "event");
    assert_eq!(frames[1]["payload"]["stream"], "ticker");
    assert_eq!(frames[1]["payload"]["payload"][0]["lastPr"], "50000");
}

#[tokio::test]
#[serial_test::serial]
async fn live_market_listener_works_without_activation_binding() {
    let _loopback = LoopbackGuard::set();
    let server = WsTestServer::spawn_market().await;
    let command = json!({
        "type": "start",
        "protocol_version": "2.0.0",
        "trigger_id": "market-live-no-activation",
        "source": "bitget_market_stream",
        "params": {
            "endpoint": server.endpoint,
            "instType": "SPOT",
            "channel": "ticker",
            "instId": "BTCUSDT"
        },
        "heartbeat_interval_ms": 1000,
        "shutdown_grace_ms": 1000
    });
    let mut stdout = Vec::new();
    run_listener_with_writer(parse_start_command(&command.to_string()).expect("valid start command"), &mut stdout)
        .await
        .expect("public market listener should not require activation");

    let frames = output_lines(&stdout);
    assert_eq!(frames[0]["type"], "ready");
    assert_eq!(frames[1]["type"], "event");
}

#[tokio::test]
#[serial_test::serial]
async fn live_user_listener_logs_in_and_emits_event() {
    let _loopback = LoopbackGuard::set();
    let server = WsTestServer::spawn_user().await;
    let command = json!({
        "type": "start",
        "protocol_version": "2.0.0",
        "trigger_id": "user-live",
        "source": "bitget_user_stream",
        "params": {
            "endpoint": server.endpoint,
            "instType": "USDT-FUTURES",
            "channel": "orders",
            "instId": "default"
        },
        "activation": {
            "allowed_origins": [server.origin],
            "secrets": {"api_key": "k", "api_secret": "s", "passphrase": "p"}
        },
        "heartbeat_interval_ms": 1000,
        "shutdown_grace_ms": 1000
    });
    let mut stdout = Vec::new();
    run_listener_with_writer(parse_start_command(&command.to_string()).expect("valid start command"), &mut stdout)
        .await
        .expect("live user listener should succeed");

    let frames = output_lines(&stdout);
    assert_eq!(frames[0]["type"], "ready");
    assert_eq!(frames[1]["payload"]["listener_kind"], "user_stream");
    assert_eq!(frames[1]["payload"]["payload"][0]["orderId"], "9001");
}

#[tokio::test]
async fn unsupported_source_rejected_before_ready() {
    let command = serde_json::json!({
        "type": "start",
        "protocol_version": "2.0.0",
        "trigger_id": "x",
        "source": "bitget_unknown_stream",
        "params": {},
        "heartbeat_interval_ms": 1000,
        "shutdown_grace_ms": 1000
    });
    let mut stdout = Vec::new();
    let error = run_listener_with_writer(parse_start_command(&command.to_string()).expect("valid start command"), &mut stdout)
        .await
        .expect_err("unsupported source should fail");
    assert!(error.contains("unsupported Bitget trigger source"));
    assert!(stdout.is_empty());
}

#[test]
fn insecure_endpoint_rejected() {
    let command = serde_json::json!({
        "type": "start",
        "protocol_version": "2.0.0",
        "trigger_id": "x",
        "source": "bitget_market_stream",
        "params": {"endpoint": "ws://example.com/ws", "stream": "ticker"},
        "activation": {"allowed_origins": ["https://example.com:443"]},
        "heartbeat_interval_ms": 1000,
        "shutdown_grace_ms": 1000
    });
    let mut stdout = Vec::new();
    let error = run_listener_with_writer(parse_start_command(&command.to_string()).expect("valid start command"), &mut stdout)
        .now_or_never()
        .expect("future should resolve immediately")
        .expect_err("insecure scheme should fail");
    assert!(error.contains("destination url must use https or wss"));
    assert!(stdout.is_empty());
}

struct WsTestServer {
    endpoint: String,
    origin: String,
}

impl WsTestServer {
    async fn spawn_market() -> Self {
        let app = Router::new().route(
            "/ws",
            get(|ws: WebSocketUpgrade| async move { ws.on_upgrade(handle_market_socket) }),
        );
        Self::serve(app).await
    }

    async fn spawn_user() -> Self {
        let app = Router::new().route(
            "/ws",
            get(|ws: WebSocketUpgrade| async move { ws.on_upgrade(handle_user_socket) }),
        );
        Self::serve(app).await
    }

    async fn serve(app: Router) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener should bind");
        let address: SocketAddr = listener.local_addr().expect("listener should have addr");
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("server should run");
        });
        Self {
            endpoint: format!("ws://{address}/ws"),
            origin: format!("http://{address}"),
        }
    }
}

async fn handle_market_socket(mut socket: WebSocket) {
    let subscribe = receive_json(&mut socket).await;
    assert_eq!(subscribe["op"], "subscribe");
    socket
        .send(Message::Text(json!({
            "event": "subscribe",
            "arg": {"instType": "SPOT", "channel": "ticker", "instId": "BTCUSDT"}
        }).to_string().into()))
        .await
        .expect("subscribe ack should send");
    socket
        .send(Message::Text(json!({
            "arg": {"instType": "SPOT", "channel": "ticker", "instId": "BTCUSDT"},
            "data": [{"ts": "1710000000000", "lastPr": "50000"}]
        }).to_string().into()))
        .await
        .expect("data frame should send");
    socket.close().await.expect("socket should close");
}

async fn handle_user_socket(mut socket: WebSocket) {
    let login = receive_json(&mut socket).await;
    assert_eq!(login["op"], "login");
    assert_eq!(login["args"][0]["apiKey"], "k");
    assert_eq!(login["args"][0]["passphrase"], "p");
    assert!(login["args"][0]["sign"].as_str().unwrap_or_default().len() > 10);
    socket
        .send(Message::Text(json!({"event": "login", "code": "0", "msg": ""}).to_string().into()))
        .await
        .expect("login ack should send");
    let subscribe = receive_json(&mut socket).await;
    assert_eq!(subscribe["op"], "subscribe");
    socket
        .send(Message::Text(json!({
            "event": "subscribe",
            "arg": {"instType": "USDT-FUTURES", "channel": "orders", "instId": "default"}
        }).to_string().into()))
        .await
        .expect("subscribe ack should send");
    socket
        .send(Message::Text(json!({
            "arg": {"instType": "USDT-FUTURES", "channel": "orders", "instId": "default"},
            "action": "snapshot",
            "data": [{"orderId": "9001", "status": "live", "uTime": "1710000001000"}]
        }).to_string().into()))
        .await
        .expect("data frame should send");
    socket.close().await.expect("socket should close");
}

async fn receive_json(socket: &mut WebSocket) -> Value {
    loop {
        match socket.next().await.expect("socket message").expect("ws result") {
            Message::Text(text) => return serde_json::from_str(&text).expect("json text"),
            Message::Ping(payload) => {
                socket.send(Message::Pong(payload)).await.expect("pong reply");
            }
            _ => {}
        }
    }
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

fn output_lines(buffer: &[u8]) -> Vec<Value> {
    String::from_utf8(buffer.to_vec())
        .expect("utf8 stdout")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("json frame"))
        .collect()
}
