use axum::{extract::State, routing::post, Json, Router};
use eth_node_official_plugin::handle_request_json;
use serde_json::{json, Value};
use std::net::SocketAddr;
use tokio::net::TcpListener;

#[tokio::test]
async fn eth_transfer_native_submit_only_returns_submitted_state() {
    let server = TestRpcServer::spawn().await;
    let response = handle_request_json(&json!({
        "contract_version": "1.0.0",
        "plugin_id": "eth-node",
        "node_id": "node-1",
        "operation": "eth_transfer_native",
        "input": {
            "endpoint": server.endpoint,
            "to": "0x0000000000000000000000000000000000000002",
            "amount": "1",
            "confirmation_mode": "submit_only"
        },
        "activation": {
            "secrets": {
                "signer": "0x59c6995e998f97a5a0044976f7ad8f4d1a9f5d3f7e61a821a3d1d0216ee7432b"
            }
        }
    }).to_string()).await.expect("request should succeed");

    let payload: Value = serde_json::from_str(&response).expect("response should decode");
    assert_eq!(payload["success"], true);
    assert_eq!(payload["result_state"], "submitted");
    assert_eq!(payload["output"]["status"], "submitted");
}

#[tokio::test]
async fn eth_raw_write_rejects_unsupported_method() {
    let response = handle_request_json(&json!({
        "contract_version": "1.0.0",
        "plugin_id": "eth-node",
        "node_id": "node-2",
        "operation": "eth_raw_write",
        "input": {
            "endpoint": "http://127.0.0.1:1",
            "method": "eth_sendRawTransaction",
            "params": []
        },
        "activation": {
            "secrets": {
                "signer": "0x59c6995e998f97a5a0044976f7ad8f4d1a9f5d3f7e61a821a3d1d0216ee7432b"
            }
        }
    }).to_string()).await.expect("request should return a failure response");

    let payload: Value = serde_json::from_str(&response).expect("response should decode");
    assert_eq!(payload["success"], false);
    assert!(payload["error"].as_str().unwrap_or_default().contains("eth_sendTransaction"));
}

#[derive(Clone)]
struct RpcState;

struct TestRpcServer {
    endpoint: String,
}

impl TestRpcServer {
    async fn spawn() -> Self {
        let app = Router::new()
            .route("/", post(handle_rpc))
            .with_state(RpcState);
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let address: SocketAddr = listener.local_addr().expect("listener should have addr");
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("server should run");
        });
        Self {
            endpoint: format!("http://{address}"),
        }
    }
}

async fn handle_rpc(State(_state): State<RpcState>, Json(payload): Json<Value>) -> Json<Value> {
    let method = payload["method"].as_str().unwrap_or_default();
    let result = match method {
        "eth_chainId" => json!("0x1"),
        "eth_getTransactionCount" => json!("0x0"),
        "eth_gasPrice" => json!("0x3b9aca00"),
        "eth_maxPriorityFeePerGas" => json!("0x3b9aca00"),
        "eth_feeHistory" => json!({
            "oldestBlock": "0x1",
            "baseFeePerGas": ["0x3b9aca00", "0x3b9aca00"],
            "gasUsedRatio": [0.5],
            "reward": [["0x3b9aca00"]]
        }),
        "eth_estimateGas" => json!("0x5208"),
        "eth_sendRawTransaction" => json!(format!("0x{:064x}", 1)),
        _ => json!(null),
    };
    Json(json!({"jsonrpc": "2.0", "id": 1, "result": result}))
}
