use std::ffi::OsString;
use std::sync::{Mutex, MutexGuard};

use axum::{routing::get, Json, Router};
use serial_test::serial;
use serde_json::{json, Value};
use across_node_official_plugin::handle_request_json;
use tokio::net::TcpListener;

#[tokio::test(flavor = "current_thread")]
#[serial]
async fn across_swap_approval_returns_swap_transaction() {
    let _loopback = LoopbackGuard::set();
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind listener");
    let address = listener.local_addr().expect("local addr");
    let app = Router::new().route("/swap/approval", get(swap_approval_handler));
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve app");
    });

    let response = handle_request_json(
        &json!({
            "contract_version": "1.0.0",
            "plugin_id": "across-node",
            "node_id": "node-1",
            "operation": "across_get_swap_approval",
            "input": {
                "base_url": format!("http://{address}"),
                "tradeType": "minOutput",
                "amount": "1000000",
                "inputToken": "0xaf88d065e77c8cC2239327C5EDb3A432268e5831",
                "outputToken": "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913",
                "originChainId": 42161,
                "destinationChainId": 8453,
                "depositor": "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "integratorId": "0xdead"
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
    assert_eq!(payload["output"]["swap_approval_response"]["swapTx"]["to"], json!("0x1111111111111111111111111111111111111111"));
}

async fn swap_approval_handler() -> Json<Value> {
    Json(json!({
        "crossSwapType": "bridgeableToBridgeable",
        "approvalTxns": [],
        "swapTx": {
            "chainId": 42161,
            "to": "0x1111111111111111111111111111111111111111",
            "data": "0x1234",
            "value": "0"
        },
        "expectedFillTime": 2
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
