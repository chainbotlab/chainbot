use std::ffi::OsString;
use std::sync::{Mutex, MutexGuard};

use axum::{extract::Json, routing::get, routing::post, Router};
use lifi_node_official_plugin::handle_request_json;
use serial_test::serial;
use serde_json::{json, Value};
use tokio::net::TcpListener;

#[tokio::test(flavor = "current_thread")]
#[serial]
async fn lifi_quote_returns_prepared_response() {
    let _loopback = LoopbackGuard::set();
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind listener");
    let address = listener.local_addr().expect("local addr");
    let app = Router::new()
        .route("/v1/quote", get(quote_handler))
        .route("/v1/advanced/routes", post(routes_handler));
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve app");
    });

    let response = handle_request_json(
        &json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "node.exec.v2",
            "params": {
                "contract_version": "1.0.0",
                "plugin_id": "lifi-node",
                "node_id": "node-1",
                "operation": "lifi_get_quote",
                "input": {
                    "base_url": format!("http://{address}"),
                    "fromChain": 42161,
                    "toChain": 10,
                    "fromToken": "0xaf88d065e77c8cC2239327C5EDb3A432268e5831",
                    "toToken": "0xDA10009cBd5D07dd0CeCc66161FC93D7c9000da1",
                    "fromAmount": "10000000",
                    "fromAddress": "0x552008c0f6870c2f77e5cC1d2eb9bdff03e30Ea0"
                },
                "activation": {
                    "allowed_origins": [format!("http://{address}")]
                }
            }
        })
        .to_string(),
    )
    .await
    .expect("request should succeed");

    server.abort();
    let _ = server.await;

    let payload: Value = serde_json::from_str(&response).expect("response should decode");
    assert_eq!(payload["result"]["result_state"], json!("prepared"));
    assert_eq!(payload["result"]["output"]["quote_response"]["transactionRequest"]["to"], json!("0x1111111111111111111111111111111111111111"));
    assert_eq!(payload["result"]["output"]["metadata"]["provider"], json!("lifi"));
}

async fn quote_handler() -> Json<Value> {
    Json(json!({
        "id": "quote-1",
        "transactionRequest": {
            "to": "0x1111111111111111111111111111111111111111",
            "data": "0x1234",
            "value": "0"
        }
    }))
}

async fn routes_handler(Json(_body): Json<Value>) -> Json<Value> {
    Json(json!({"routes": [{"id": "route-1"}]}))
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
