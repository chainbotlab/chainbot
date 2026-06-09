use std::net::SocketAddr;

use axum::{extract::State, routing::get, routing::post, Json, Router};
use jupiter_node_official_plugin::handle_request_json;
use serde_json::{json, Value};
use tokio::net::TcpListener;

#[tokio::test]
async fn jupiter_get_quote_maps_quote_response() {
    let server = TestServer::spawn().await;
    let response = handle_request_json(
        &json!({
            "contract_version": "1.0.0",
            "plugin_id": "jupiter-node",
            "node_id": "node-1",
            "operation": "jupiter_get_quote",
            "input": {
                "base_url": server.jupiter_base_url,
                "input_mint": "So11111111111111111111111111111111111111112",
                "output_mint": "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
                "amount": "100000000",
                "slippage_bps": 50,
                "restrict_intermediate_tokens": true
            }
        })
        .to_string(),
    )
    .await
    .expect("request should succeed");

    let payload: Value = serde_json::from_str(&response).expect("response should decode");
    assert_eq!(payload["success"], true);
    assert_eq!(payload["output"]["in_amount"], "100000000");
    assert_eq!(payload["output"]["out_amount"], "17057460");
    assert_eq!(
        payload["output"]["quote_response"]["routePlan"][0]["swapInfo"]["label"],
        "Raydium CLMM"
    );
}

#[tokio::test]
async fn jupiter_build_swap_returns_unsigned_transaction() {
    let server = TestServer::spawn().await;
    let response = handle_request_json(
        &json!({
            "contract_version": "1.0.0",
            "plugin_id": "jupiter-node",
            "node_id": "node-2",
            "operation": "jupiter_build_swap",
            "input": {
                "base_url": server.jupiter_base_url,
                "quote_response": quote_response(),
                "user_public_key": "11111111111111111111111111111111",
                "dynamic_compute_unit_limit": true,
                "prioritization_fee_lamports": {
                    "priorityLevelWithMaxLamports": {
                        "priorityLevel": "veryHigh",
                        "maxLamports": 1000000
                    }
                }
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
    assert_eq!(payload["output"]["swap_transaction"], "AQIDBA==");
    assert_eq!(payload["output"]["last_valid_block_height"], 123456);
}

#[tokio::test]
async fn jupiter_send_swap_transaction_submit_only_returns_signature() {
    let server = TestServer::spawn().await;
    let response = handle_request_json(
        &json!({
            "contract_version": "1.0.0",
            "plugin_id": "jupiter-node",
            "node_id": "node-3",
            "operation": "jupiter_send_swap_transaction",
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
async fn jupiter_get_signature_status_returns_rpc_status() {
    let server = TestServer::spawn().await;
    let response = handle_request_json(
        &json!({
            "contract_version": "1.0.0",
            "plugin_id": "jupiter-node",
            "node_id": "node-4",
            "operation": "jupiter_get_signature_status",
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
    jupiter_base_url: String,
    rpc_url: String,
}

impl TestServer {
    async fn spawn() -> Self {
        let app = Router::new()
            .route("/swap/v1/quote", get(handle_quote))
            .route("/swap/v1/swap", post(handle_swap))
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
            jupiter_base_url: format!("http://{address}/swap/v1"),
            rpc_url: format!("http://{address}/rpc"),
        }
    }
}

async fn handle_quote(State(_state): State<TestState>) -> Json<Value> {
    Json(quote_response())
}

async fn handle_swap(State(_state): State<TestState>, Json(payload): Json<Value>) -> Json<Value> {
    assert_eq!(payload["userPublicKey"], "11111111111111111111111111111111");
    assert_eq!(payload["dynamicComputeUnitLimit"], true);
    Json(json!({
        "swapTransaction": "AQIDBA==",
        "lastValidBlockHeight": 123456,
        "prioritizationFeeLamports": 5000
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

fn quote_response() -> Value {
    json!({
        "inputMint": "So11111111111111111111111111111111111111112",
        "inAmount": "100000000",
        "outputMint": "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
        "outAmount": "17057460",
        "otherAmountThreshold": "16886885",
        "swapMode": "ExactIn",
        "slippageBps": 50,
        "priceImpactPct": "0.0001",
        "routePlan": [{
            "swapInfo": {
                "ammKey": "raydium-pool",
                "label": "Raydium CLMM",
                "inputMint": "So11111111111111111111111111111111111111112",
                "outputMint": "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
                "inAmount": "100000000",
                "outAmount": "17057460"
            },
            "percent": 100
        }]
    })
}
