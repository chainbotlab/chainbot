use std::net::SocketAddr;

use axum::{extract::State, routing::post, Json, Router};
use serde_json::{json, Value};
use solana_node_official_plugin::handle_request_json;
use tokio::net::TcpListener;

#[tokio::test]
async fn solana_get_balance_uses_rpc_result() {
    let server = TestRpcServer::spawn().await;
    let response = handle_request_json(
        &json!({
            "contract_version": "1.0.0",
            "plugin_id": "solana-node",
            "node_id": "node-1",
            "operation": "solana_get_balance",
            "input": {
                "endpoint": server.endpoint,
                "address": "11111111111111111111111111111111",
                "commitment": "confirmed"
            }
        })
        .to_string(),
    )
    .await
    .expect("request should succeed");

    let payload: Value = serde_json::from_str(&response).expect("response should decode");
    assert_eq!(payload["success"], true);
    assert_eq!(payload["output"]["balance"], "42");
}

#[tokio::test]
async fn solana_get_token_balance_uses_rpc_result() {
    let server = TestRpcServer::spawn().await;
    let response = handle_request_json(
        &json!({
            "contract_version": "1.0.0",
            "plugin_id": "solana-node",
            "node_id": "node-2",
            "operation": "solana_get_token_balance",
            "input": {
                "endpoint": server.endpoint,
                "token_account": "11111111111111111111111111111111",
                "commitment": "confirmed"
            }
        })
        .to_string(),
    )
    .await
    .expect("request should succeed");

    let payload: Value = serde_json::from_str(&response).expect("response should decode");
    assert_eq!(payload["success"], true);
    assert_eq!(payload["output"]["balance"], "7");
}

#[tokio::test]
async fn solana_raw_read_returns_rpc_result() {
    let server = TestRpcServer::spawn().await;
    let response = handle_request_json(
        &json!({
            "contract_version": "1.0.0",
            "plugin_id": "solana-node",
            "node_id": "node-3",
            "operation": "solana_raw_read",
            "input": {
                "endpoint": server.endpoint,
                "method": "getHealth",
                "params": []
            }
        })
        .to_string(),
    )
    .await
    .expect("request should succeed");

    let payload: Value = serde_json::from_str(&response).expect("response should decode");
    assert_eq!(payload["success"], true);
    assert_eq!(payload["output"]["result"], "ok");
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
        "getBalance" => json!({"value": 42}),
        "getTokenAccountBalance" => json!({"value": {"amount": "7"}}),
        "getHealth" => json!("ok"),
        _ => json!(null),
    };
    Json(json!({"jsonrpc": "2.0", "id": 1, "result": result}))
}
