use std::net::SocketAddr;

use axum::{extract::State, routing::post, Json, Router};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use uniswap_trigger_official_plugin::contract::parse_start_command;
use uniswap_trigger_official_plugin::listener::{poll_once, price_event_frame};

#[tokio::test]
async fn uniswap_price_threshold_poll_emits_triggerable_event() {
    let server = TestRpcServer::spawn().await;
    let command = parse_start_command(
        &json!({
            "type": "start",
            "protocol_version": "2.0.0",
            "trigger_id": "uni-price",
            "source": "uniswap_price_threshold",
            "params": {
                "endpoint": server.endpoint,
                "router": "0x0000000000000000000000000000000000000001",
                "amount_in": "100",
                "threshold_out": "200",
                "comparison": "gte",
                "path": [
                    "0x0000000000000000000000000000000000000002",
                    "0x0000000000000000000000000000000000000003"
                ]
            },
            "heartbeat_interval_ms": 1000,
            "shutdown_grace_ms": 1000
        })
        .to_string(),
    )
    .expect("command should parse");
    let check = poll_once(&reqwest::Client::new(), &command)
        .await
        .expect("poll should succeed");

    assert!(check.triggered);
    assert_eq!(check.amount_out.to_string(), "250");

    let frame = price_event_frame(&command, &check, 1);
    assert_eq!(frame.r#type, "event");
    assert_eq!(frame.payload["listener_kind"], "price_threshold");
    assert_eq!(frame.payload["amount_out"], "250");
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
