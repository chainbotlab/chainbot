use std::net::SocketAddr;

use axum::{routing::get, Json, Router};
use aster_node_official_plugin::handle_request_json;
use serde_json::{json, Value};
use tokio::net::TcpListener;

#[tokio::test]
#[serial_test::serial]
async fn aster_get_server_time_returns_server_time_field() {
    let _loopback = LoopbackGuard::set();
    let server = TestAsterServer::spawn(json!({"serverTime": 1234567890i64})).await;
    let response = handle_request_json(
        &json!({
            "contract_version": "1.0.0",
            "plugin_id": "aster-node",
            "node_id": "node-1",
            "operation": "aster_get_server_time",
            "input": {
                "base_url": server.base_url
            }
        })
        .to_string(),
    )
    .await
    .expect("request should succeed");

    let payload: Value = serde_json::from_str(&response).expect("response should decode");
    assert_eq!(payload["success"], true);
    assert_eq!(payload["output"]["server_time"], 1234567890i64);
}

#[tokio::test]
#[serial_test::serial]
async fn aster_get_server_time_requires_server_time_field() {
    let _loopback = LoopbackGuard::set();
    let server = TestAsterServer::spawn(json!({"ok": true})).await;
    let response = handle_request_json(
        &json!({
            "contract_version": "1.0.0",
            "plugin_id": "aster-node",
            "node_id": "node-2",
            "operation": "aster_get_server_time",
            "input": {
                "base_url": server.base_url
            }
        })
        .to_string(),
    )
    .await
    .expect("request should return failure response");

    let payload: Value = serde_json::from_str(&response).expect("response should decode");
    assert_eq!(payload["success"], false);
    assert!(payload["error"]
        .as_str()
        .unwrap_or_default()
        .contains("serverTime"));
}

#[tokio::test]
async fn aster_get_server_time_rejects_non_https_base_url_outside_test_loopback() {
    let response = handle_request_json(
        &json!({
            "contract_version": "1.0.0",
            "plugin_id": "aster-node",
            "node_id": "node-3",
            "operation": "aster_get_server_time",
            "input": {
                "base_url": "http://example.com"
            }
        })
        .to_string(),
    )
    .await
    .expect("request should return failure response");

    let payload: Value = serde_json::from_str(&response).expect("response should decode");
    assert_eq!(payload["success"], false);
    assert!(payload["error"]
        .as_str()
        .unwrap_or_default()
        .contains("request url must use https"));
}

#[derive(Clone)]
struct AsterState {
    response: Value,
}

struct TestAsterServer {
    base_url: String,
}

impl TestAsterServer {
    async fn spawn(response: Value) -> Self {
        let app = Router::new()
            .route("/fapi/v3/time", get(handle_time))
            .with_state(AsterState { response });
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

async fn handle_time(axum::extract::State(state): axum::extract::State<AsterState>) -> Json<Value> {
    Json(state.response)
}
