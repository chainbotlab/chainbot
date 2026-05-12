use axum::{http::StatusCode, routing::{get, post}, Json, Router};
use bybit_node_official_plugin::handle_request_json;
use serde_json::{json, Value};
use std::net::SocketAddr;
use tokio::net::TcpListener;

#[tokio::test]
#[serial_test::serial]
async fn bybit_place_order_submit_only_returns_submitted() {
    let _loopback = LoopbackGuard::set();
    let server = TestBybitServer::spawn_with_query_status("New").await;
    let payload = call_place_order(&server, Some("submit_only")).await;
    assert_eq!(payload["success"], true);
    assert_eq!(payload["result_state"], "submitted");
    assert_eq!(payload["output"]["status"], "submitted");
}

#[tokio::test]
#[serial_test::serial]
async fn bybit_place_order_safe_returns_submitted_for_non_terminal() {
    let _loopback = LoopbackGuard::set();
    let server = TestBybitServer::spawn_with_query_status("New").await;
    let payload = call_place_order(&server, None).await;
    assert_eq!(payload["success"], true);
    assert_eq!(payload["result_state"], "submitted");
    assert_eq!(payload["output"]["status"], "submitted");
}

#[tokio::test]
#[serial_test::serial]
async fn bybit_place_order_safe_returns_settled_for_terminal() {
    let _loopback = LoopbackGuard::set();
    let server = TestBybitServer::spawn_with_query_status("Filled").await;
    let payload = call_place_order(&server, None).await;
    assert_eq!(payload["success"], true);
    assert_eq!(payload["result_state"], "settled");
    assert_eq!(payload["output"]["status"], "confirmed");
}

#[tokio::test]
#[serial_test::serial]
async fn bybit_place_order_safe_returns_pending_confirmation_when_query_fails() {
    let _loopback = LoopbackGuard::set();
    let server = TestBybitServer::spawn_with_query_status("ERROR").await;
    let payload = call_place_order(&server, None).await;
    assert_eq!(payload["success"], true);
    assert_eq!(payload["result_state"], "submitted");
    assert_eq!(payload["output"]["status"], "pending_confirmation");
}

#[tokio::test]
async fn bybit_get_positions_rejects_spot() {
    let response = handle_request_json(&json!({
        "contract_version": "1.0.0",
        "plugin_id": "bybit-node",
        "node_id": "node-pos",
        "operation": "bybit_get_positions",
        "input": {"product_line": "spot"},
        "activation": {"secrets": {"api_key": "k", "api_secret": "s"}}
    }).to_string())
    .await
    .expect("request should serialize failure response");
    let payload: Value = serde_json::from_str(&response).expect("response should decode");
    assert_eq!(payload["success"], false);
    assert!(payload["error"].as_str().unwrap_or_default().contains("positions are unavailable for spot"));
}

#[tokio::test]
#[serial_test::serial]
async fn bybit_signed_requests_require_allowlisted_origin() {
    let _loopback = LoopbackGuard::set();
    let server = TestBybitServer::spawn_with_query_status("Filled").await;
    let response = handle_request_json(&json!({
        "contract_version": "1.0.0",
        "plugin_id": "bybit-node",
        "node_id": "node-allow",
        "operation": "bybit_get_balances",
        "input": {"product_line": "spot", "base_url": server.base_url},
        "activation": {
            "allowed_origins": ["https://api.bybit.com"],
            "secrets": {"api_key": "k", "api_secret": "s"}
        }
    }).to_string())
    .await
    .expect("request should serialize failure response");
    let payload: Value = serde_json::from_str(&response).expect("response should decode");
    assert_eq!(payload["success"], false);
    assert!(payload["error"].as_str().unwrap_or_default().contains("is not allowlisted"));
}

#[tokio::test]
async fn bybit_rejects_mismatched_plugin_id() {
    let response = handle_request_json(&json!({
        "contract_version": "1.0.0",
        "plugin_id": "binance-node",
        "node_id": "node-bad",
        "operation": "bybit_get_server_time",
        "input": {"product_line": "spot"}
    }).to_string())
    .await
    .expect("request should serialize failure response");
    let payload: Value = serde_json::from_str(&response).expect("response should decode");
    assert_eq!(payload["success"], false);
    assert!(payload["error"].as_str().unwrap_or_default().contains("plugin_id must be bybit-node"));
}

async fn call_place_order(server: &TestBybitServer, confirmation_mode: Option<&str>) -> Value {
    let mut input = json!({
        "product_line": "linear",
        "symbol": "BTCUSDT",
        "side": "Buy",
        "type": "Limit",
        "qty": "1",
        "price": "100",
        "time_in_force": "GTC",
        "base_url": server.base_url
    });
    if let Some(mode) = confirmation_mode {
        input["confirmation_mode"] = Value::String(mode.to_owned());
    }

    let response = handle_request_json(&json!({
        "contract_version": "1.0.0",
        "plugin_id": "bybit-node",
        "node_id": "node-write",
        "operation": "bybit_place_order",
        "input": input,
        "activation": {
            "allowed_origins": [server.origin()],
            "secrets": {"api_key": "test-key", "api_secret": "test-secret"}
        }
    }).to_string())
    .await
    .expect("request should succeed");

    serde_json::from_str(&response).expect("response should decode")
}

struct TestBybitServer {
    base_url: String,
}

impl TestBybitServer {
    async fn spawn_with_query_status(query_status: &'static str) -> Self {
        let app = Router::new()
            .route("/v5/order/create", post(handle_place_order))
            .route("/v5/order/realtime", get(handle_get_order))
            .route("/v5/account/wallet-balance", get(handle_wallet))
            .with_state(query_status);
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

async fn handle_place_order() -> Json<Value> {
    Json(json!({
        "retCode": 0,
        "retMsg": "OK",
        "result": {"orderId": "777", "orderLinkId": "client-777", "orderStatus": "New"}
    }))
}

async fn handle_get_order(axum::extract::State(query_status): axum::extract::State<&'static str>) -> impl axum::response::IntoResponse {
    if query_status == "ERROR" {
        return (StatusCode::BAD_GATEWAY, Json(json!({"retCode": 10001, "retMsg": "upstream unavailable"})));
    }
    (
        StatusCode::OK,
        Json(json!({"retCode": 0, "retMsg": "OK", "result": {"list": [{"orderStatus": query_status, "orderId": "777"}]}})),
    )
}

async fn handle_wallet() -> Json<Value> {
    Json(json!({"retCode": 0, "retMsg": "OK", "result": {"list": [{"coin": []}]}}))
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
