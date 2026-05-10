use std::net::SocketAddr;

use axum::{extract::State, routing::{delete, get, post}, Json, Router};
use binance_node_official_plugin::handle_request_json;
use serde_json::{json, Value};
use tokio::net::TcpListener;

#[tokio::test]
#[serial_test::serial]
async fn binance_get_server_time_returns_server_time_field() {
    let _loopback = LoopbackGuard::set();
    let server = TestBinanceServer::spawn().await;
    let response = handle_request_json(&json!({
        "contract_version": "1.0.0",
        "plugin_id": "binance-node",
        "node_id": "node-1",
        "operation": "binance_get_server_time",
        "input": {
            "product_line": "spot",
            "base_url": server.base_url
        }
    })
    .to_string())
    .await
    .expect("request should succeed");

    let payload: Value = serde_json::from_str(&response).expect("response should decode");
    assert_eq!(payload["success"], true);
    assert_eq!(payload["output"]["server_time"], 1234567890i64);
}

#[tokio::test]
#[serial_test::serial]
async fn binance_get_balances_extracts_spot_balances() {
    let _loopback = LoopbackGuard::set();
    let server = TestBinanceServer::spawn().await;
    let response = handle_request_json(&json!({
        "contract_version": "1.0.0",
        "plugin_id": "binance-node",
        "node_id": "node-2",
        "operation": "binance_get_balances",
        "input": {
            "product_line": "spot",
            "base_url": server.base_url,
            "recv_window": "5000"
        },
        "activation": {
            "allowed_origins": [server.origin()],
            "secrets": {
                "api_key": "test-key",
                "api_secret": "test-secret"
            }
        }
    })
    .to_string())
    .await
    .expect("request should succeed");

    let payload: Value = serde_json::from_str(&response).expect("response should decode");
    assert_eq!(payload["success"], true);
    assert_eq!(payload["output"]["balances"][0]["asset"], "BTC");
}

#[derive(Clone)]
struct BinanceState;

struct TestBinanceServer {
    base_url: String,
}

impl TestBinanceServer {
    async fn spawn() -> Self {
        let app = Router::new()
            .route("/api/v3/time", get(handle_get))
            .route("/api/v3/account", get(handle_get))
            .route("/api/v3/order", post(handle_write).delete(handle_write))
            .route("/api/v3/openOrders", delete(handle_write))
            .route("/api/v3/userDataStream", post(handle_user_stream).put(handle_user_stream).delete(handle_user_stream))
            .with_state(BinanceState);
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let address: SocketAddr = listener.local_addr().expect("listener should have addr");
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("server should run");
        });
        Self {
            base_url: format!("http://{address}"),
        }
    }

    fn origin(&self) -> String {
        self.base_url.clone()
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

async fn handle_get(State(_state): State<BinanceState>, uri: axum::http::Uri) -> Json<Value> {
    match uri.path() {
        "/api/v3/time" => Json(json!({"serverTime": 1234567890i64})),
        "/api/v3/account" => Json(json!({
            "balances": [
                {"asset": "BTC", "free": "1.0", "locked": "0.0"},
                {"asset": "USDT", "free": "12.5", "locked": "0.0"}
            ]
        })),
        _ => Json(json!({})),
    }
}

async fn handle_write(State(_state): State<BinanceState>) -> Json<Value> {
    Json(json!({"orderId": 42, "clientOrderId": "abc-123", "status": "NEW"}))
}

async fn handle_user_stream(State(_state): State<BinanceState>) -> Json<Value> {
    Json(json!({"listenKey": "listen-key-1"}))
}
