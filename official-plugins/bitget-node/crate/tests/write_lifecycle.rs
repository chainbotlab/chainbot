use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::{
    extract::State,
    routing::post,
    Json, Router,
};
use bitget_node_official_plugin::handle_request_json;
use serde_json::{json, Value};
use tokio::net::TcpListener;

#[derive(Clone, Default)]
struct RequestCapture {
    entries: Arc<Mutex<Vec<(String, Value)>>>,
}

impl RequestCapture {
    fn push(&self, path: &str, body: Value) {
        self.entries.lock().expect("capture lock").push((path.to_string(), body));
    }

    fn take(&self) -> Vec<(String, Value)> {
        self.entries.lock().expect("capture lock").clone()
    }
}

#[tokio::test]
#[serial_test::serial]
async fn bitget_place_order_submit_only_returns_submitted_state() {
    let _loopback = LoopbackGuard::set();
    let server = TestBitgetServer::spawn().await;
    let response = handle_request_json(
        &json!({
            "contract_version": "1.0.0",
            "plugin_id": "bitget-node",
            "node_id": "node-1",
            "operation": "bitget_place_order",
            "input": {
                "product_line": "spot",
                "symbol": "BTCUSDT",
                "side": "buy",
                "type": "market",
                "quantity": "0.1",
                "confirmation_mode": "submit_only",
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
    assert_eq!(payload["result_state"], "submitted");
    assert_eq!(payload["output"]["status"], "submitted");

    let requests = server.capture.take();
    assert_eq!(requests[0].0, "/api/v2/spot/trade/place-order");
    assert_eq!(requests[0].1["symbol"], "BTCUSDT");
    assert_eq!(requests[0].1["size"], "0.1");
}

#[tokio::test]
#[serial_test::serial]
async fn bitget_cancel_order_uses_spot_cancel_order_endpoint() {
    let _loopback = LoopbackGuard::set();
    let server = TestBitgetServer::spawn().await;
    let response = handle_request_json(
        &json!({
            "contract_version": "1.0.0",
            "plugin_id": "bitget-node",
            "node_id": "node-3",
            "operation": "bitget_cancel_order",
            "input": {
                "product_line": "spot",
                "symbol": "BTCUSDT",
                "order_id": "42",
                "base_url": server.base_url,
                "confirmation_mode": "submit_only"
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
    assert_eq!(payload["output"]["transaction_id"], "42");

    let requests = server.capture.take();
    assert_eq!(requests[0].0, "/api/v2/spot/trade/cancel-order");
    assert_eq!(requests[0].1["orderId"], "42");
}

#[tokio::test]
#[serial_test::serial]
async fn bitget_cancel_all_orders_uses_spot_cancel_symbol_order_endpoint() {
    let _loopback = LoopbackGuard::set();
    let server = TestBitgetServer::spawn().await;
    let response = handle_request_json(
        &json!({
            "contract_version": "1.0.0",
            "plugin_id": "bitget-node",
            "node_id": "node-4",
            "operation": "bitget_cancel_all_orders",
            "input": {
                "product_line": "spot",
                "symbol": "BTCUSDT",
                "base_url": server.base_url,
                "confirmation_mode": "submit_only"
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
    assert_eq!(payload["output"]["transaction_id"], "BTCUSDT");

    let requests = server.capture.take();
    assert_eq!(requests[0].0, "/api/v2/spot/trade/cancel-symbol-order");
    assert_eq!(requests[0].1["symbol"], "BTCUSDT");
}

#[tokio::test]
#[serial_test::serial]
async fn bitget_futures_place_order_includes_product_type_body() {
    let _loopback = LoopbackGuard::set();
    let server = TestBitgetServer::spawn().await;
    let response = handle_request_json(
        &json!({
            "contract_version": "1.0.0",
            "plugin_id": "bitget-node",
            "node_id": "node-5",
            "operation": "bitget_place_order",
            "input": {
                "product_line": "futures",
                "product_type": "COIN-FUTURES",
                "symbol": "BTCUSD",
                "side": "buy",
                "type": "limit",
                "quantity": "1",
                "price": "50000",
                "margin_coin": "BTC",
                "base_url": server.base_url,
                "confirmation_mode": "submit_only"
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

    let requests = server.capture.take();
    assert_eq!(requests[0].0, "/api/v2/mix/order/place-order");
    assert_eq!(requests[0].1["productType"], "COIN-FUTURES");
    assert_eq!(requests[0].1["marginCoin"], "BTC");
}

#[tokio::test]
async fn bitget_get_positions_rejects_spot_product_line() {
    let response = handle_request_json(
        &json!({
            "contract_version": "1.0.0",
            "plugin_id": "bitget-node",
            "node_id": "node-2",
            "operation": "bitget_get_positions",
            "input": {"product_line": "spot", "base_url": "http://127.0.0.1:1"},
            "activation": {"secrets": {"api_key": "x", "api_secret": "y", "passphrase": "z"}}
        })
        .to_string(),
    )
    .await
    .expect("request should return a failure response");

    let payload: Value = serde_json::from_str(&response).expect("response should decode");
    assert_eq!(payload["success"], false);
    assert!(payload["error"].as_str().unwrap_or_default().contains("positions are unavailable for spot"));
}

#[tokio::test]
#[serial_test::serial]
async fn bitget_place_order_safe_confirms_terminal_status() {
    let _loopback = LoopbackGuard::set();
    let server = TestBitgetServer::spawn().await;
    let response = handle_request_json(
        &json!({
            "contract_version": "1.0.0",
            "plugin_id": "bitget-node",
            "node_id": "node-6",
            "operation": "bitget_place_order",
            "input": {
                "product_line": "spot",
                "symbol": "BTCUSDT",
                "side": "buy",
                "type": "market",
                "quantity": "0.1",
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
    assert_eq!(payload["result_state"], "settled");
    assert_eq!(payload["output"]["status"], "confirmed");
    assert_eq!(payload["output"]["metadata"]["terminal"], true);
}

#[tokio::test]
#[serial_test::serial]
async fn bitget_cancel_all_orders_safe_settles_when_no_open_orders_remain() {
    let _loopback = LoopbackGuard::set();
    let server = TestBitgetServer::spawn().await;
    let response = handle_request_json(
        &json!({
            "contract_version": "1.0.0",
            "plugin_id": "bitget-node",
            "node_id": "node-7",
            "operation": "bitget_cancel_all_orders",
            "input": {
                "product_line": "spot",
                "symbol": "BTCUSDT",
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
    assert_eq!(payload["result_state"], "settled");
    assert_eq!(payload["output"]["status"], "confirmed");
}

struct TestBitgetServer {
    base_url: String,
    capture: RequestCapture,
}

impl TestBitgetServer {
    async fn spawn() -> Self {
        let capture = RequestCapture::default();
        let app = Router::new()
            .route(
                "/api/v2/spot/trade/place-order",
                post({
                    let capture = capture.clone();
                    move |State(capture): State<RequestCapture>, Json(body): Json<Value>| async move {
                        capture.push("/api/v2/spot/trade/place-order", body);
                        Json(json!({"data": {"orderId": "42"}}))
                    }
                }),
            )
            .route(
                "/api/v2/spot/trade/orderInfo",
                post({
                    let capture = capture.clone();
                    move |State(capture): State<RequestCapture>, Json(body): Json<Value>| async move {
                        capture.push("/api/v2/spot/trade/orderInfo", body);
                        Json(json!({"data": [{"orderId": "42", "status": "filled"}]}))
                    }
                }),
            )
            .route(
                "/api/v2/spot/trade/cancel-order",
                post({
                    let capture = capture.clone();
                    move |State(capture): State<RequestCapture>, Json(body): Json<Value>| async move {
                        capture.push("/api/v2/spot/trade/cancel-order", body);
                        Json(json!({"data": {"orderId": "42"}}))
                    }
                }),
            )
            .route(
                "/api/v2/spot/trade/cancel-symbol-order",
                post({
                    let capture = capture.clone();
                    move |State(capture): State<RequestCapture>, Json(body): Json<Value>| async move {
                        capture.push("/api/v2/spot/trade/cancel-symbol-order", body);
                        Json(json!({"data": {"symbol": "BTCUSDT"}}))
                    }
                }),
            )
            .route(
                "/api/v2/spot/trade/unfilled-orders",
                post({
                    let capture = capture.clone();
                    move |State(capture): State<RequestCapture>, Json(body): Json<Value>| async move {
                        capture.push("/api/v2/spot/trade/unfilled-orders", body);
                        Json(json!({"data": []}))
                    }
                }),
            )
            .route(
                "/api/v2/mix/order/place-order",
                post({
                    let capture = capture.clone();
                    move |State(capture): State<RequestCapture>, Json(body): Json<Value>| async move {
                        capture.push("/api/v2/mix/order/place-order", body);
                        Json(json!({"data": {"orderId": "f-99"}}))
                    }
                }),
            )
            .route(
                "/api/v2/spot/trade/orderInfo",
                axum::routing::get(|| async { Json(json!({"data": [{"orderId": "42", "status": "filled"}]})) }),
            )
            .route(
                "/api/v2/spot/trade/unfilled-orders",
                axum::routing::get(|| async { Json(json!({"data": []})) }),
            )
            .with_state(capture.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener should bind");
        let address: SocketAddr = listener.local_addr().expect("listener should have addr");
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("server should run");
        });
        Self { base_url: format!("http://{address}"), capture }
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
