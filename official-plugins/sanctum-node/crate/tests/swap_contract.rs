use std::net::SocketAddr;

use axum::{extract::State, routing::get, routing::post, Json, Router};
use sanctum_node_official_plugin::handle_request_json;
use serde_json::{json, Value};
use tokio::net::TcpListener;

#[tokio::test]
async fn sanctum_get_lsts_returns_metadata_list() {
    let server = TestServer::spawn().await;
    let response = handle_request_json(
        &json!({
            "contract_version": "1.0.0",
            "plugin_id": "sanctum-node",
            "node_id": "node-1",
            "operation": "sanctum_get_lsts",
            "input": {"base_url": server.sanctum_base_url}
        })
        .to_string(),
    )
    .await
    .expect("request should succeed");

    let payload: Value = serde_json::from_str(&response).expect("response should decode");
    assert_eq!(payload["success"], true);
    assert_eq!(payload["output"]["lsts"][0]["symbol"], "INF");
}

#[tokio::test]
async fn sanctum_create_swap_order_maps_unsigned_transaction() {
    let server = TestServer::spawn().await;
    let response = handle_request_json(
        &json!({
            "contract_version": "1.0.0",
            "plugin_id": "sanctum-node",
            "node_id": "node-2",
            "operation": "sanctum_create_swap_order",
            "input": {
                "base_url": server.sanctum_base_url,
                "input_mint": "So11111111111111111111111111111111111111112",
                "output_mint": "inf-mint",
                "amount": "100000000",
                "mode": "ExactIn",
                "signer": "11111111111111111111111111111111",
                "slippage_bps": 50
            }
        })
        .to_string(),
    )
    .await
    .expect("request should succeed");

    let payload: Value = serde_json::from_str(&response).expect("response should decode");
    assert_eq!(payload["success"], true);
    assert_eq!(payload["result_state"], "prepared");
    assert_eq!(payload["output"]["swap_transaction"], "AQIDBA==");
    assert_eq!(payload["output"]["input_amount"], "100000000");
    assert_eq!(payload["output"]["output_amount"], "99000000");
}

#[tokio::test]
async fn sanctum_execute_swap_order_returns_signature() {
    let server = TestServer::spawn().await;
    let response = handle_request_json(
        &json!({
            "contract_version": "1.0.0",
            "plugin_id": "sanctum-node",
            "node_id": "node-3",
            "operation": "sanctum_execute_swap_order",
            "input": {
                "base_url": server.sanctum_base_url,
                "signed_transaction": "signed-base64",
                "order_response": order_response()
            }
        })
        .to_string(),
    )
    .await
    .expect("request should succeed");

    let payload: Value = serde_json::from_str(&response).expect("response should decode");
    assert_eq!(payload["success"], true);
    assert_eq!(payload["result_state"], "submitted");
    assert_eq!(payload["output"]["transaction_id"], "sanctum-signature");
}

#[tokio::test]
async fn sanctum_send_swap_transaction_submit_only_returns_signature() {
    let server = TestServer::spawn().await;
    let response = handle_request_json(
        &json!({
            "contract_version": "1.0.0",
            "plugin_id": "sanctum-node",
            "node_id": "node-4",
            "operation": "sanctum_send_swap_transaction",
            "input": {
                "endpoint": server.rpc_url,
                "signed_transaction": "AQIDBA==",
                "confirmation_mode": "submit_only",
                "preflight": true
            }
        })
        .to_string(),
    )
    .await
    .expect("request should succeed");

    let payload: Value = serde_json::from_str(&response).expect("response should decode");
    assert_eq!(payload["success"], true);
    assert_eq!(payload["output"]["transaction_id"], "test-signature");
}

#[tokio::test]
async fn sanctum_send_swap_transaction_finalized_returns_settled() {
    let server = TestServer::spawn().await;
    let response = handle_request_json(
        &json!({
            "contract_version": "1.0.0",
            "plugin_id": "sanctum-node",
            "node_id": "node-5",
            "operation": "sanctum_send_swap_transaction",
            "input": {
                "endpoint": server.rpc_url,
                "signed_transaction": "AQIDBA==",
                "confirmation_mode": "finalized",
                "preflight": true
            }
        })
        .to_string(),
    )
    .await
    .expect("request should succeed");

    let payload: Value = serde_json::from_str(&response).expect("response should decode");
    assert_eq!(payload["success"], true);
    assert_eq!(payload["result_state"], "settled");
    assert_eq!(payload["output"]["transaction_id"], "test-signature");
}

#[tokio::test]
async fn sanctum_get_signature_status_returns_metadata() {
    let server = TestServer::spawn().await;
    let response = handle_request_json(
        &json!({
            "contract_version": "1.0.0",
            "plugin_id": "sanctum-node",
            "node_id": "node-6",
            "operation": "sanctum_get_signature_status",
            "input": {
                "endpoint": server.rpc_url,
                "signature": "test-signature"
            }
        })
        .to_string(),
    )
    .await
    .expect("request should succeed");

    let payload: Value = serde_json::from_str(&response).expect("response should decode");
    assert_eq!(payload["success"], true);
    assert_eq!(payload["output"]["signature"], "test-signature");
    assert_eq!(payload["output"]["status"]["confirmation_status"], "finalized");
    assert_eq!(payload["output"]["metadata"]["provider"], "sanctum");
}

#[derive(Clone)]
struct TestState;

struct TestServer {
    sanctum_base_url: String,
    rpc_url: String,
}

impl TestServer {
    async fn spawn() -> Self {
        let app = Router::new()
            .route("/lsts", get(handle_lsts))
            .route("/swap/token/order", get(handle_order))
            .route("/swap/token/execute", post(handle_execute))
            .route("/rpc", post(handle_rpc))
            .with_state(TestState);
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let address: SocketAddr = listener.local_addr().expect("listener should have addr");
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("server should run");
        });
        Self {
            sanctum_base_url: format!("http://{address}"),
            rpc_url: format!("http://{address}/rpc"),
        }
    }
}

async fn handle_lsts(State(_state): State<TestState>) -> Json<Value> {
    Json(json!([{"symbol": "INF", "mint": "inf-mint"}]))
}

async fn handle_order(State(_state): State<TestState>) -> Json<Value> {
    Json(order_response())
}

async fn handle_execute(State(_state): State<TestState>, Json(payload): Json<Value>) -> Json<Value> {
    assert_eq!(payload["signedTx"], "signed-base64");
    assert_eq!(payload["orderResponse"]["tx"], "AQIDBA==");
    Json(json!({"txSignature": "sanctum-signature"}))
}

async fn handle_rpc(State(_state): State<TestState>, Json(payload): Json<Value>) -> Json<Value> {
    let method = payload["method"].as_str().unwrap_or_default();
    let result = match method {
        "sendTransaction" => json!("test-signature"),
        "getSignatureStatuses" => json!({
            "value": [{
                "slot": 99,
                "confirmations": null,
                "err": null,
                "confirmationStatus": "finalized"
            }]
        }),
        _ => json!(null),
    };
    Json(json!({"jsonrpc": "2.0", "id": 1, "result": result}))
}

fn order_response() -> Value {
    json!({
        "tx": "AQIDBA==",
        "inpAmt": "100000000",
        "outAmt": "99000000",
        "source": "Infinity",
        "feeAmt": "1000",
        "feeMint": "inf-mint"
    })
}
