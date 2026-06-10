use std::ffi::OsString;
use std::sync::{Mutex, MutexGuard};

use axum::{routing::get, Json, Router};
use rango_node_official_plugin::handle_request_json;
use serial_test::serial;
use serde_json::{json, Value};
use tokio::net::TcpListener;

#[tokio::test(flavor = "current_thread")]
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

static LOOPBACK_ENV_LOCK: Mutex<()> = Mutex::new(());

struct LoopbackGuard {
    _lock: MutexGuard<'static, ()>,
    previous_loopback: Option<OsString>,
    previous_internal: Option<OsString>,
}

impl LoopbackGuard {
    fn set() -> Self {
        let lock = LOOPBACK_ENV_LOCK.lock().expect("loopback env lock poisoned");
        let previous_loopback = std::env::var_os("CHAINBOT_HTTP_NODE_ALLOW_LOOPBACK_FOR_TESTS");
        let previous_internal = std::env::var_os("CHAINBOT_INTERNAL_ALLOW_TEST_DESTINATIONS");
        unsafe {
            std::env::set_var("CHAINBOT_HTTP_NODE_ALLOW_LOOPBACK_FOR_TESTS", "1");
            std::env::set_var("CHAINBOT_INTERNAL_ALLOW_TEST_DESTINATIONS", "1");
        }
        Self {
            _lock: lock,
            previous_loopback,
            previous_internal,
        }
    }
}

impl Drop for LoopbackGuard {
    fn drop(&mut self) {
        unsafe {
            match self.previous_loopback.take() {
                Some(value) => std::env::set_var("CHAINBOT_HTTP_NODE_ALLOW_LOOPBACK_FOR_TESTS", value),
                None => std::env::remove_var("CHAINBOT_HTTP_NODE_ALLOW_LOOPBACK_FOR_TESTS"),
            }
            match self.previous_internal.take() {
                Some(value) => std::env::set_var("CHAINBOT_INTERNAL_ALLOW_TEST_DESTINATIONS", value),
                None => std::env::remove_var("CHAINBOT_INTERNAL_ALLOW_TEST_DESTINATIONS"),
            }
        }
    }
}
