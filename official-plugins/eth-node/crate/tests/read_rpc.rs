use std::net::SocketAddr;

use axum::{extract::State, routing::post, Json, Router};
use eth_node_official_plugin::handle_request_json;
use serde_json::{json, Value};
use tokio::net::TcpListener;

#[tokio::test]
async fn eth_get_balance_uses_rpc_result() {
    let server = TestRpcServer::spawn().await;
    let response = handle_request_json(&json!({
        "contract_version": "1.0.0",
        "plugin_id": "eth-node",
        "node_id": "node-1",
        "operation": "eth_get_balance",
        "input": {
            "endpoint": server.endpoint,
            "address": "0x0000000000000000000000000000000000000001",
            "block_tag": "latest"
        }
    }).to_string()).await.expect("request should succeed");

    let payload: Value = serde_json::from_str(&response).expect("response should decode");
    assert_eq!(payload["success"], true);
    assert_eq!(payload["output"]["balance"], "16");
}

#[tokio::test]
async fn eth_raw_read_returns_rpc_result() {
    let server = TestRpcServer::spawn().await;
    let response = handle_request_json(&json!({
        "contract_version": "1.0.0",
        "plugin_id": "eth-node",
        "node_id": "node-2",
        "operation": "eth_raw_read",
        "input": {
            "endpoint": server.endpoint,
            "method": "web3_clientVersion",
            "params": []
        }
    }).to_string()).await.expect("request should succeed");

    let payload: Value = serde_json::from_str(&response).expect("response should decode");
    assert_eq!(payload["output"]["result"], "chainbot-test-client");
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

async fn handle_rpc(
    State(_state): State<RpcState>,
    Json(payload): Json<Value>,
) -> Json<Value> {
    let method = payload["method"].as_str().unwrap_or_default();
    let result = match method {
        "eth_getBalance" => json!("0x10"),
        "web3_clientVersion" => json!("chainbot-test-client"),
        "eth_call" => json!("0x0"),
        _ => json!(null),
    };
    Json(json!({"jsonrpc": "2.0", "id": 1, "result": result}))
}
