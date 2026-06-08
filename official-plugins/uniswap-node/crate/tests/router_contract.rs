use std::net::SocketAddr;

use axum::{extract::State, routing::post, Json, Router};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use uniswap_node_official_plugin::handle_request_json;

#[tokio::test]
async fn uniswap_get_amounts_out_decodes_router_result() {
    let server = TestRpcServer::spawn().await;
    let response = handle_request_json(
        &json!({
            "contract_version": "1.0.0",
            "plugin_id": "uniswap-node",
            "node_id": "node-1",
            "operation": "uniswap_get_amounts_out",
            "input": {
                "endpoint": server.endpoint,
                "router": "0x0000000000000000000000000000000000000001",
                "amount_in": "100",
                "path": [
                    "0x0000000000000000000000000000000000000002",
                    "0x0000000000000000000000000000000000000003"
                ]
            }
        })
        .to_string(),
    )
    .await
    .expect("request should succeed");

    let payload: Value = serde_json::from_str(&response).expect("response should decode");
    assert_eq!(payload["success"], true);
    assert_eq!(payload["output"]["amounts"][0], "100");
    assert_eq!(payload["output"]["amount_out"], "250");
}

#[tokio::test]
async fn uniswap_watch_price_evaluates_threshold() {
    let server = TestRpcServer::spawn().await;
    let response = handle_request_json(
        &json!({
            "contract_version": "1.0.0",
            "plugin_id": "uniswap-node",
            "node_id": "node-2",
            "operation": "uniswap_watch_price",
            "input": {
                "endpoint": server.endpoint,
                "router": "0x0000000000000000000000000000000000000001",
                "amount_in": "100",
                "threshold_out": "200",
                "comparison": "gte",
                "path": [
                    "0x0000000000000000000000000000000000000002",
                    "0x0000000000000000000000000000000000000003"
                ]
            }
        })
        .to_string(),
    )
    .await
    .expect("request should succeed");

    let payload: Value = serde_json::from_str(&response).expect("response should decode");
    assert_eq!(payload["success"], true);
    assert_eq!(payload["output"]["triggered"], true);
    assert_eq!(payload["output"]["amount_out"], "250");
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
        "eth_call" => json!(encoded_amounts_result(&[100, 250])),
        _ => json!(null),
    };
    Json(json!({"jsonrpc": "2.0", "id": 1, "result": result}))
}

fn encoded_amounts_result(amounts: &[u64]) -> String {
    let mut words = vec![word(32), word(amounts.len() as u64)];
    for amount in amounts {
        words.push(word(*amount));
    }
    format!("0x{}", words.concat())
}

fn word(value: u64) -> String {
    format!("{value:064x}")
}
