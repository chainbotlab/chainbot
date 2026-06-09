use std::net::SocketAddr;

use axum::{extract::State, routing::get, routing::post, Json, Router};
use raydium_node_official_plugin::handle_request_json;
use serde_json::{json, Value};
use tokio::net::TcpListener;

#[tokio::test]
async fn raydium_compute_swap_maps_trade_api_response() {
    let server = TestServer::spawn().await;
    let response = handle_request_json(
        &json!({
            "contract_version": "1.0.0",
            "plugin_id": "raydium-node",
            "node_id": "node-1",
            "operation": "raydium_compute_swap",
            "input": {
                "base_url": server.raydium_base_url,
                "input_mint": "So11111111111111111111111111111111111111112",
                "output_mint": "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
                "amount": "100000000",
                "slippage_bps": 50,
                "tx_version": "V0"
            }
        })
        .to_string(),
    )
    .await
    .expect("request should succeed");

    let payload: Value = serde_json::from_str(&response).expect("response should decode");
    assert_eq!(payload["success"], true);
    assert_eq!(payload["output"]["input_amount"], "100000000");
    assert_eq!(payload["output"]["output_amount"], "17057460");
    assert_eq!(
        payload["output"]["swap_response"]["data"]["routePlan"][0]["poolId"],
        "raydium-pool"
    );
}

#[tokio::test]
async fn raydium_build_swap_returns_unsigned_transactions() {
    let server = TestServer::spawn().await;
    let response = handle_request_json(
        &json!({
            "contract_version": "1.0.0",
            "plugin_id": "raydium-node",
            "node_id": "node-2",
            "operation": "raydium_build_swap",
            "input": {
                "base_url": server.raydium_base_url,
                "swap_response": compute_response(),
                "wallet": "11111111111111111111111111111111",
                "tx_version": "V0",
                "compute_unit_price_micro_lamports": "50000",
                "wrap_sol": true,
                "unwrap_sol": false
            }
        })
        .to_string(),
    )
    .await
    .expect("request should succeed");

    let payload: Value = serde_json::from_str(&response).expect("response should decode");
    assert_eq!(payload["success"], true);
    assert_eq!(payload["result_state"], "prepared");
    assert_eq!(payload["output"]["status"], "prepared");
    assert_eq!(payload["output"]["transactions"][0], "AQIDBA==");
    assert_eq!(payload["output"]["transactions"][1], "BQYHCA==");
}

#[tokio::test]
async fn raydium_send_swap_transaction_submit_only_returns_signature() {
    let server = TestServer::spawn().await;
    let response = handle_request_json(
        &json!({
            "contract_version": "1.0.0",
            "plugin_id": "raydium-node",
            "node_id": "node-3",
            "operation": "raydium_send_swap_transaction",
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
    assert_eq!(payload["result_state"], "submitted");
    assert_eq!(payload["output"]["transaction_id"], "test-signature");
}

#[tokio::test]
async fn raydium_get_signature_status_returns_rpc_status() {
    let server = TestServer::spawn().await;
    let response = handle_request_json(
        &json!({
            "contract_version": "1.0.0",
            "plugin_id": "raydium-node",
            "node_id": "node-4",
            "operation": "raydium_get_signature_status",
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
    assert_eq!(payload["output"]["status"]["confirmation_status"], "confirmed");
}

#[derive(Clone)]
struct TestState;

struct TestServer {
    raydium_base_url: String,
    rpc_url: String,
}

impl TestServer {
    async fn spawn() -> Self {
        let app = Router::new()
            .route("/compute/swap-base-in", get(handle_compute))
            .route("/transaction/swap-base-in", post(handle_build))
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
            raydium_base_url: format!("http://{address}"),
            rpc_url: format!("http://{address}/rpc"),
        }
    }
}

async fn handle_compute(State(_state): State<TestState>) -> Json<Value> {
    Json(compute_response())
}

async fn handle_build(State(_state): State<TestState>, Json(payload): Json<Value>) -> Json<Value> {
    assert_eq!(payload["wallet"], "11111111111111111111111111111111");
    assert_eq!(payload["txVersion"], "V0");
    assert_eq!(payload["computeUnitPriceMicroLamports"], "50000");
    Json(json!({
        "id": "build-id",
        "success": true,
        "version": "V1",
        "data": [
            {"transaction": "AQIDBA=="},
            {"transaction": "BQYHCA=="}
        ]
    }))
}

async fn handle_rpc(State(_state): State<TestState>, Json(payload): Json<Value>) -> Json<Value> {
    let method = payload["method"].as_str().unwrap_or_default();
    let result = match method {
        "sendTransaction" => json!("test-signature"),
        "getSignatureStatuses" => json!({
            "value": [{
                "slot": 99,
                "confirmations": 1,
                "err": null,
                "confirmationStatus": "confirmed"
            }]
        }),
        _ => json!(null),
    };
    Json(json!({"jsonrpc": "2.0", "id": 1, "result": result}))
}

fn compute_response() -> Value {
    json!({
        "id": "quote-id",
        "success": true,
        "version": "V1",
        "data": {
            "swapType": "BaseIn",
            "inputMint": "So11111111111111111111111111111111111111112",
            "inputAmount": "100000000",
            "outputMint": "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
            "outputAmount": "17057460",
            "otherAmountThreshold": "16886885",
            "slippageBps": 50,
            "priceImpactPct": 0.0001,
            "routePlan": [{
                "poolId": "raydium-pool",
                "inputMint": "So11111111111111111111111111111111111111112",
                "outputMint": "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
                "feeAmount": "100",
                "feeMint": "So11111111111111111111111111111111111111112"
            }]
        }
    })
}
