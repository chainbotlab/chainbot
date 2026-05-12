use axum::{http::StatusCode, routing::{get, post}, Json, Router};
use gate_node_official_plugin::handle_request_json;
use serde_json::{json, Value};
use std::net::SocketAddr;
use tokio::net::TcpListener;

#[tokio::test]
#[serial_test::serial]
async fn gate_place_order_submit_only_returns_submitted_state() {
    let _loopback = LoopbackGuard::set();
    let server = TestGateServer::spawn().await;
    let response = handle_request_json(&json!({
        "contract_version": "1.0.0",
        "plugin_id": "gate-node",
        "node_id": "node-1",
        "operation": "gate_place_order",
        "input": {
            "currency_pair": "BTC_USDT",
            "side": "buy",
            "amount": "1",
            "price": "100",
            "confirmation_mode": "submit_only",
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
    assert_eq!(payload["result_state"], "submitted");
    assert_eq!(payload["output"]["status"], "submitted");
}

#[tokio::test]
#[serial_test::serial]
async fn gate_place_order_safe_returns_settled_for_terminal_order() {
    let _loopback = LoopbackGuard::set();
    let server = TestGateServer::spawn_with_query_status("closed").await;
    let response = handle_request_json(&json!({
        "contract_version": "1.0.0",
        "plugin_id": "gate-node",
        "node_id": "node-2",
        "operation": "gate_place_order",
        "input": {
            "currency_pair": "BTC_USDT",
            "side": "buy",
            "amount": "1",
            "price": "100",
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
    assert_eq!(payload["result_state"], "settled");
    assert_eq!(payload["output"]["status"], "confirmed");
}

#[tokio::test]
#[serial_test::serial]
async fn gate_signed_requests_require_allowlisted_origin() {
    let _loopback = LoopbackGuard::set();
    let server = TestGateServer::spawn().await;
    let response = handle_request_json(&json!({
        "contract_version": "1.0.0",
        "plugin_id": "gate-node",
        "node_id": "node-3",
        "operation": "gate_get_accounts",
        "input": {
            "base_url": server.base_url
        },
        "activation": {
            "allowed_origins": ["https://api.gateio.ws"],
            "secrets": {
                "api_key": "test-key",
                "api_secret": "test-secret"
            }
        }
    })
    .to_string())
    .await
    .expect("request should serialize failure response");

    let payload: Value = serde_json::from_str(&response).expect("response should decode");
    assert_eq!(payload["success"], false);
    assert!(payload["error"]
        .as_str()
        .unwrap_or_default()
        .contains("is not allowlisted"));
}

#[tokio::test]
async fn gate_rejects_mismatched_plugin_id() {
    let response = handle_request_json(&json!({
        "contract_version": "1.0.0",
        "plugin_id": "http-node",
        "node_id": "node-4",
        "operation": "gate_get_server_time",
        "input": {}
    })
    .to_string())
    .await
    .expect("request should serialize failure response");

    let payload: Value = serde_json::from_str(&response).expect("response should decode");
    assert_eq!(payload["success"], false);
    assert!(payload["error"].as_str().unwrap_or_default().contains("plugin_id must be gate-node"));
}

struct TestGateServer {
    base_url: String,
}

impl TestGateServer {
    async fn spawn() -> Self {
        Self::spawn_with_query_status("open").await
    }

    async fn spawn_with_query_status(query_status: &'static str) -> Self {
        let app = Router::new()
            .route("/api/v4/spot/orders", post(handle_place_order).delete(handle_cancel_all))
            .route("/api/v4/spot/orders/{order_id}", get(handle_get_order).delete(handle_place_order))
            .route("/api/v4/spot/accounts", get(handle_accounts))
            .with_state(query_status);
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

async fn handle_place_order() -> Json<Value> {
    Json(json!({
        "id": "777",
        "text": "client-777",
        "status": "open",
        "currency_pair": "BTC_USDT"
    }))
}

async fn handle_get_order(axum::extract::State(query_status): axum::extract::State<&'static str>) -> impl axum::response::IntoResponse {
    if query_status == "error" {
        return (StatusCode::BAD_GATEWAY, Json(json!({"label": "SERVER_ERROR", "message": "upstream unavailable"})));
    }
    (
        StatusCode::OK,
        Json(json!({
            "id": "777",
            "text": "client-777",
            "status": query_status,
            "currency_pair": "BTC_USDT"
        })),
    )
}

async fn handle_cancel_all() -> Json<Value> {
    Json(json!([
        {"id": "777", "status": "closed", "currency_pair": "BTC_USDT"}
    ]))
}

async fn handle_accounts() -> Json<Value> {
    Json(json!([{"currency": "BTC", "available": "1"}]))
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
