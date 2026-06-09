use axum::{routing::get, Json, Router};
use rango_node_official_plugin::handle_request_json;
use serial_test::serial;
use serde_json::{json, Value};
use tokio::net::TcpListener;

#[tokio::test]
#[serial]
async fn rango_create_transaction_returns_prepared_step() {
    let _loopback = LoopbackGuard::set();
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind listener");
    let address = listener.local_addr().expect("local addr");
    let app = Router::new().route("/tx/create", get(tx_create_handler));
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve app");
    });

    let response = handle_request_json(
        &json!({
            "contract_version": "1.0.0",
            "plugin_id": "rango-node",
            "node_id": "node-1",
            "operation": "rango_create_transaction",
            "input": {
                "base_url": format!("http://{address}"),
                "requestId": "route-1",
                "step": 1
            },
            "activation": {
                "allowed_origins": [format!("http://{address}")]
            }
        })
        .to_string(),
    )
    .await
    .expect("request should succeed");

    server.abort();
    let _ = server.await;

    let payload: Value = serde_json::from_str(&response).expect("response should decode");
    assert_eq!(payload["success"], json!(true));
    assert_eq!(payload["result_state"], json!("prepared"));
    assert_eq!(payload["output"]["transaction_response"]["transaction"]["to"], json!("0x1111111111111111111111111111111111111111"));
}

async fn tx_create_handler() -> Json<Value> {
    Json(json!({
        "requestId": "route-1",
        "transaction": {
            "to": "0x1111111111111111111111111111111111111111",
            "data": "0x1234",
            "value": "0"
        }
    }))
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
