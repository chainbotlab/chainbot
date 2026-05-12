use std::net::SocketAddr;

use axum::{routing::{get, post}, Json, Router};
use gate_node_official_plugin::handle_request_json;
use serde_json::{json, Value};
use tokio::net::TcpListener;

#[tokio::test]
#[serial_test::serial]
async fn gate_get_server_time_returns_server_time_field() {
    let _loopback = LoopbackGuard::set();
    let server = TestGateServer::spawn().await;
    let response = handle_request_json(&json!({
        "contract_version": "1.0.0",
        "plugin_id": "gate-node",
        "node_id": "node-1",
        "operation": "gate_get_server_time",
        "input": {
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
async fn gate_get_accounts_returns_signed_payload() {
    let _loopback = LoopbackGuard::set();
    let server = TestGateServer::spawn().await;
    let response = handle_request_json(&json!({
        "contract_version": "1.0.0",
        "plugin_id": "gate-node",
        "node_id": "node-2",
        "operation": "gate_get_accounts",
        "input": {
            "base_url": server.base_url
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
    assert_eq!(payload["output"]["accounts"][0]["currency"], "BTC");
}

#[derive(Clone)]
struct GateState;

struct TestGateServer {
    base_url: String,
}

impl TestGateServer {
    async fn spawn() -> Self {
        let app = Router::new()
            .route("/api/v4/spot/time", get(handle_get))
            .route("/api/v4/spot/accounts", get(handle_get))
            .route("/api/v4/spot/orders", post(handle_write).delete(handle_write))
            .route("/api/v4/spot/orders/{order_id}", get(handle_write).delete(handle_write))
            .with_state(GateState);
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

async fn handle_get(uri: axum::http::Uri) -> Json<Value> {
    match uri.path() {
        "/api/v4/spot/time" => Json(json!({"server_time": 1234567890i64})),
        "/api/v4/spot/accounts" => Json(json!([
            {"currency": "BTC", "available": "1.0", "locked": "0.0"}
        ])),
        _ => Json(json!({})),
    }
}

async fn handle_write() -> Json<Value> {
    Json(json!({"id": "42", "status": "open", "currency_pair": "BTC_USDT"}))
}
