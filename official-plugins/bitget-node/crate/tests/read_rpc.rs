use std::net::SocketAddr;

use axum::{
    extract::Query,
    routing::get,
    Json, Router,
};
use bitget_node_official_plugin::handle_request_json;
use serde_json::{json, Value};
use tokio::net::TcpListener;

#[tokio::test]
#[serial_test::serial]
async fn bitget_get_server_time_returns_server_time_field() {
    let _loopback = LoopbackGuard::set();
    let server = TestBitgetServer::spawn().await;
    let response = handle_request_json(
        &json!({
            "contract_version": "1.0.0",
            "plugin_id": "bitget-node",
            "node_id": "node-1",
            "operation": "bitget_get_server_time",
            "input": {"product_line": "spot", "base_url": server.base_url}
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
async fn bitget_get_ticker_uses_futures_bulk_endpoint_without_symbol() {
    let _loopback = LoopbackGuard::set();
    let server = TestBitgetServer::spawn().await;
    let response = handle_request_json(
        &json!({
            "contract_version": "1.0.0",
            "plugin_id": "bitget-node",
            "node_id": "node-4",
            "operation": "bitget_get_ticker",
            "input": {"product_line": "futures", "product_type": "USDT-FUTURES", "base_url": server.base_url}
        })
        .to_string(),
    )
    .await
    .expect("request should succeed");

    let payload: Value = serde_json::from_str(&response).expect("response should decode");
    assert_eq!(payload["success"], true);
    assert_eq!(payload["output"]["ticker"][0]["symbol"], "BTCUSDT");
}

#[tokio::test]
#[serial_test::serial]
async fn bitget_get_depth_uses_futures_merge_depth_endpoint() {
    let _loopback = LoopbackGuard::set();
    let server = TestBitgetServer::spawn().await;
    let response = handle_request_json(
        &json!({
            "contract_version": "1.0.0",
            "plugin_id": "bitget-node",
            "node_id": "node-5",
            "operation": "bitget_get_depth",
            "input": {
                "product_line": "futures",
                "product_type": "USDT-FUTURES",
                "symbol": "BTCUSDT",
                "limit": "5",
                "precision": "scale0",
                "base_url": server.base_url
            }
        })
        .to_string(),
    )
    .await
    .expect("request should succeed");

    let payload: Value = serde_json::from_str(&response).expect("response should decode");
    assert_eq!(payload["success"], true);
    assert_eq!(payload["output"]["depth"]["bids"][0][0], "50000");
}

#[tokio::test]
#[serial_test::serial]
async fn bitget_get_order_uses_spot_order_info_endpoint() {
    let _loopback = LoopbackGuard::set();
    let server = TestBitgetServer::spawn().await;
    let response = handle_request_json(
        &json!({
            "contract_version": "1.0.0",
            "plugin_id": "bitget-node",
            "node_id": "node-6",
            "operation": "bitget_get_order",
            "input": {
                "product_line": "spot",
                "order_id": "42",
                "base_url": server.base_url
            },
            "activation": {
                "allowed_origins": [server.origin()],
                "secrets": {"api_key": "x", "api_secret": "y", "passphrase": "z"}
            }
        })
        .to_string(),
    )
    .await
    .expect("request should succeed");

    let payload: Value = serde_json::from_str(&response).expect("response should decode");
    assert_eq!(payload["success"], true);
    assert_eq!(payload["output"]["order"][0]["orderId"], "42");
}

#[tokio::test]
#[serial_test::serial]
async fn bitget_signed_requests_require_allowlisted_origin() {
    let _loopback = LoopbackGuard::set();
    let server = TestBitgetServer::spawn().await;
    let response = handle_request_json(
        &json!({
            "contract_version": "1.0.0",
            "plugin_id": "bitget-node",
            "node_id": "node-2",
            "operation": "bitget_get_balances",
            "input": {"product_line": "spot", "base_url": server.base_url},
            "activation": {
                "allowed_origins": ["https://api.bitget.com"],
                "secrets": {"api_key": "x", "api_secret": "y", "passphrase": "z"}
            }
        })
        .to_string(),
    )
    .await
    .expect("request should serialize failure response");

    let payload: Value = serde_json::from_str(&response).expect("response should decode");
    assert_eq!(payload["success"], false);
    assert!(payload["error"].as_str().unwrap_or_default().contains("not allowlisted"));
}

#[tokio::test]
async fn bitget_rejects_mismatched_plugin_id() {
    let response = handle_request_json(
        &json!({
            "contract_version": "1.0.0",
            "plugin_id": "binance-node",
            "node_id": "node-3",
            "operation": "bitget_get_server_time",
            "input": {"product_line": "spot"}
        })
        .to_string(),
    )
    .await
    .expect("request should serialize failure response");

    let payload: Value = serde_json::from_str(&response).expect("response should decode");
    assert_eq!(payload["success"], false);
    assert!(payload["error"].as_str().unwrap_or_default().contains("plugin_id must be bitget-node"));
}

struct TestBitgetServer {
    base_url: String,
}

impl TestBitgetServer {
    async fn spawn() -> Self {
        let app = Router::new()
            .route(
                "/api/v2/public/time",
                get(|| async { Json(json!({"data": {"serverTime": "1234567890"}})) }),
            )
            .route(
                "/api/v2/mix/market/tickers",
                get(|Query(query): Query<std::collections::HashMap<String, String>>| async move {
                    assert_eq!(query.get("productType").map(String::as_str), Some("USDT-FUTURES"));
                    Json(json!({"data": [{"symbol": "BTCUSDT", "lastPr": "50000"}]}))
                }),
            )
            .route(
                "/api/v2/mix/market/merge-depth",
                get(|Query(query): Query<std::collections::HashMap<String, String>>| async move {
                    assert_eq!(query.get("symbol").map(String::as_str), Some("BTCUSDT"));
                    assert_eq!(query.get("productType").map(String::as_str), Some("USDT-FUTURES"));
                    assert_eq!(query.get("limit").map(String::as_str), Some("5"));
                    assert_eq!(query.get("precision").map(String::as_str), Some("scale0"));
                    Json(json!({"data": {"bids": [["50000", "1"]], "asks": [["50010", "2"]]}}))
                }),
            )
            .route(
                "/api/v2/spot/trade/orderInfo",
                get(|Query(query): Query<std::collections::HashMap<String, String>>| async move {
                    assert_eq!(query.get("orderId").map(String::as_str), Some("42"));
                    Json(json!({"data": [{"orderId": "42", "status": "filled"}]}))
                }),
            );
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener should bind");
        let address: SocketAddr = listener.local_addr().expect("listener should have addr");
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("server should run");
        });
        Self { base_url: format!("http://{address}") }
    }

    fn origin(&self) -> String { self.base_url.clone() }
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
