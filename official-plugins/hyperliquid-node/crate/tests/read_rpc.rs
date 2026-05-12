use axum::{extract::Json, routing::post, Router};
use hyperliquid_node_official_plugin::handle_request_json;
use serial_test::serial;
use serde_json::{json, Value};
use tokio::net::TcpListener;

#[tokio::test]
#[serial]
async fn get_all_mids_reads_info_endpoint() {
    let _loopback = LoopbackGuard::set();
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind listener");
    let address = listener.local_addr().expect("local addr");
    let app = Router::new().route("/info", post(info_handler));
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve app");
    });

    let response = handle_request_json(
        &json!({
            "contract_version": "1.0.0",
            "plugin_id": "hyperliquid-node",
            "node_id": "node-1",
            "operation": "hyperliquid_get_all_mids",
            "input": {
                "base_url": format!("http://{address}"),
                "dex": "perp"
            },
            "activation": {
                "allowed_origins": [format!("http://{address}")]
            }
        })
        .to_string(),
    )
    .await
    .expect("request succeeds");

    server.abort();
    let _ = server.await;

    let payload: Value = serde_json::from_str(&response).expect("json response");
    assert_eq!(payload["success"], json!(true));
    assert_eq!(payload["output"]["all_mids"]["BTC"], json!("64000.1"));
}

#[tokio::test]
#[serial]
async fn get_l2_book_returns_normalized_output() {
    let _loopback = LoopbackGuard::set();
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind listener");
    let address = listener.local_addr().expect("local addr");
    let app = Router::new().route("/info", post(info_handler));
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve app");
    });

    let response = handle_request_json(
        &json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "node.execute",
            "params": {
                "contract_version": "1.0.0",
                "plugin_id": "hyperliquid-node",
                "node_id": "node-1",
                "operation": "hyperliquid_get_l2_book",
                "input": {
                    "base_url": format!("http://{address}"),
                    "coin": "BTC",
                    "nSigFigs": 4,
                    "mantissa": 2
                },
                "activation": {
                    "allowed_origins": [format!("http://{address}")]
                }
            }
        })
        .to_string(),
    )
    .await
    .expect("request succeeds");

    server.abort();
    let _ = server.await;

    let payload: Value = serde_json::from_str(&response).expect("jsonrpc response");
    assert_eq!(payload["result"]["output"]["l2_book"]["coin"], json!("BTC"));
    assert_eq!(payload["result"]["output"]["l2_book"]["levels"][0][0]["px"], json!("64000.0"));
}

#[tokio::test]
#[serial]
async fn get_candle_snapshot_requires_loopback_guard() {
    let response = handle_request_json(
        &json!({
            "contract_version": "1.0.0",
            "plugin_id": "hyperliquid-node",
            "node_id": "node-1",
            "operation": "hyperliquid_get_candle_snapshot",
            "input": {
                "base_url": "http://127.0.0.1:8080",
                "coin": "BTC",
                "interval": "15m",
                "startTime": 1000,
                "endTime": 2000
            },
            "activation": {
                "allowed_origins": ["http://127.0.0.1:8080"]
            }
        })
        .to_string(),
    )
    .await
    .expect("response generated");

    let payload: Value = serde_json::from_str(&response).expect("json response");
    assert_eq!(payload["success"], json!(false));
    let error = payload["error"].as_str().expect("error message");
    assert!(error.contains("blocked"));
}

#[tokio::test]
#[serial]
async fn get_all_mids_rejects_non_allowlisted_base_url() {
    let _loopback = LoopbackGuard::set();
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind listener");
    let address = listener.local_addr().expect("local addr");
    drop(listener);

    let response = handle_request_json(
        &json!({
            "contract_version": "1.0.0",
            "plugin_id": "hyperliquid-node",
            "node_id": "node-1",
            "operation": "hyperliquid_get_all_mids",
            "input": {
                "base_url": format!("http://{address}"),
                "dex": "perp"
            },
            "activation": {
                "allowed_origins": ["https://api.hyperliquid.xyz"]
            }
        })
        .to_string(),
    )
    .await
    .expect("response generated");

    let payload: Value = serde_json::from_str(&response).expect("json response");
    assert_eq!(payload["success"], json!(false));
    let error = payload["error"].as_str().expect("error message");
    assert!(error.contains("not allowlisted"), "unexpected error: {error}");
}

async fn info_handler(Json(body): Json<Value>) -> Json<Value> {
    let response = match body["type"].as_str() {
        Some("allMids") => json!({"BTC": "64000.1", "ETH": "3200.5"}),
        Some("l2Book") => json!({
            "coin": body["coin"],
            "time": 1710000000000i64,
            "levels": [
                [{"px": "64000.0", "sz": "1.5", "n": 3}],
                [{"px": "64001.0", "sz": "1.1", "n": 2}]
            ]
        }),
        Some("candleSnapshot") => json!([
            {
                "t": body["req"]["startTime"],
                "T": body["req"]["endTime"],
                "s": body["req"]["coin"],
                "i": body["req"]["interval"],
                "o": "64000.0",
                "c": "64100.0",
                "h": "64200.0",
                "l": "63900.0",
                "v": "12.5"
            }
        ]),
        other => json!({"unexpected": other}),
    };
    Json(response)
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
