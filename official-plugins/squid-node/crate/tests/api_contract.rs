use axum::{routing::get, routing::post, Json, Router};
use serial_test::serial;
use serde_json::{json, Value};
use squid_node_official_plugin::handle_request_json;
use tokio::net::TcpListener;

#[tokio::test]
#[serial]
async fn squid_route_returns_transaction_request() {
    let _loopback = LoopbackGuard::set();
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind listener");
    let address = listener.local_addr().expect("local addr");
    let app = Router::new()
        .route("/route", post(route_handler))
        .route("/status", get(status_handler));
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve app");
    });

    let response = handle_request_json(
        &json!({
            "contract_version": "1.0.0",
            "plugin_id": "squid-node",
            "node_id": "node-1",
            "operation": "squid_get_route",
            "input": {
                "base_url": format!("http://{address}"),
                "fromAddress": "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "fromChain": "56",
                "fromToken": "0x55d398326f99059fF775485246999027B3197955",
                "fromAmount": "1000000000000000",
                "toChain": "42161",
                "toToken": "0xaf88d065e77c8cC2239327C5EDb3A432268e5831",
                "toAddress": "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
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
    assert_eq!(payload["output"]["route_response"]["route"]["transactionRequest"]["target"], json!("0x1111111111111111111111111111111111111111"));
}

async fn route_handler() -> Json<Value> {
    Json(json!({
        "route": {
            "quoteId": "quote-1",
            "transactionRequest": {
                "target": "0x1111111111111111111111111111111111111111",
                "data": "0x1234",
                "value": "0"
            }
        }
    }))
}

async fn status_handler() -> Json<Value> {
    Json(json!({"squidTransactionStatus": "success"}))
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
