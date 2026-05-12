use std::net::SocketAddr;

use axum::{extract::State, routing::get, Json, Router};
use bybit_node_official_plugin::handle_request_json;
use serde_json::{json, Value};
use tokio::net::TcpListener;

#[tokio::test]
#[serial_test::serial]
async fn bybit_get_server_time_returns_unwrapped_result_field() {
    let _loopback = LoopbackGuard::set();
    let server = TestBybitServer::spawn().await;
    let response = handle_request_json(&json!({
        "contract_version": "1.0.0",
        "plugin_id": "bybit-node",
        "node_id": "node-1",
        "operation": "bybit_get_server_time",
        "input": {"product_line": "spot", "base_url": server.base_url}
    }).to_string())
    .await
    .expect("request should succeed");

    let payload: Value = serde_json::from_str(&response).expect("response should decode");
    assert_eq!(payload["success"], true);
    assert_eq!(payload["output"]["server_time"], "1234567890");
}

#[tokio::test]
#[serial_test::serial]
async fn bybit_get_balances_extracts_coin_array() {
    let _loopback = LoopbackGuard::set();
    let server = TestBybitServer::spawn().await;
    let response = handle_request_json(&json!({
        "contract_version": "1.0.0",
        "plugin_id": "bybit-node",
        "node_id": "node-2",
        "operation": "bybit_get_balances",
        "input": {"product_line": "spot", "base_url": server.base_url},
        "activation": {
            "allowed_origins": [server.origin()],
            "secrets": {"api_key": "test-key", "api_secret": "test-secret"}
        }
    }).to_string())
    .await
    .expect("request should succeed");

    let payload: Value = serde_json::from_str(&response).expect("response should decode");
    assert_eq!(payload["success"], true);
    assert_eq!(payload["output"]["balances"][0]["coin"], "BTC");
}

#[derive(Clone)]
struct BybitState;

struct TestBybitServer {
    base_url: String,
}

impl TestBybitServer {
    async fn spawn() -> Self {
        let app = Router::new()
            .route("/v5/market/time", get(handle_get))
            .route("/v5/account/wallet-balance", get(handle_get))
            .with_state(BybitState);
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener should bind");
        let address: SocketAddr = listener.local_addr().expect("listener should have addr");
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("server should run");
        });
        Self { base_url: format!("http://{address}") }
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

async fn handle_get(State(_state): State<BybitState>, uri: axum::http::Uri) -> Json<Value> {
    match uri.path() {
        "/v5/market/time" => Json(json!({"retCode": 0, "retMsg": "OK", "result": {"timeNano": "1234567890"}})),
        "/v5/account/wallet-balance" => Json(json!({
            "retCode": 0,
            "retMsg": "OK",
            "result": {
                "list": [
                    {
                        "accountType": "UNIFIED",
                        "coin": [
                            {"coin": "BTC", "walletBalance": "1.0"},
                            {"coin": "USDT", "walletBalance": "12.5"}
                        ]
                    }
                ]
            }
        })),
        _ => Json(json!({"retCode": 0, "retMsg": "OK", "result": {}})),
    }
}
