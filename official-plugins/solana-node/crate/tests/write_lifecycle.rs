use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::{extract::State, routing::post, Json, Router};
use base64::Engine;
use serde_json::{json, Value};
use solana_hash::Hash;
use solana_keypair::Keypair;
use solana_message::Message;
use solana_node_official_plugin::handle_request_json;
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use solana_system_interface::instruction as system_instruction;
use solana_transaction::Transaction;
use tokio::net::TcpListener;

#[tokio::test]
async fn solana_transfer_native_submit_only_returns_submitted_state() {
    let server = TestRpcServer::spawn().await;
    let signer = test_signer();
    let recipient = Pubkey::new_unique();

    let response = handle_request_json(
        &json!({
            "contract_version": "1.0.0",
            "plugin_id": "solana-node",
            "node_id": "node-1",
            "operation": "solana_transfer_native",
            "input": {
                "endpoint": server.endpoint,
                "to": recipient.to_string(),
                "lamports": "15",
                "confirmation_mode": "submit_only"
            },
            "activation": {
                "secrets": {
                    "signer": signer.to_base58_string()
                }
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

    let sent = server.last_transaction();
    assert_eq!(sent.message.account_keys[0], signer.pubkey());
    assert_eq!(sent.message.account_keys[1], recipient);
}

#[tokio::test]
async fn solana_raw_write_signs_managed_message_and_waits_for_status() {
    let server = TestRpcServer::spawn().await;
    let signer = test_signer();
    let recipient = Pubkey::new_unique();
    let message = Message::new(
        &[system_instruction::transfer(&signer.pubkey(), &recipient, 3)],
        Some(&signer.pubkey()),
    );
    let message_base64 = base64::engine::general_purpose::STANDARD
        .encode(bincode::serialize(&message).expect("message should serialize"));

    let response = handle_request_json(
        &json!({
            "contract_version": "1.0.0",
            "plugin_id": "solana-node",
            "node_id": "node-2",
            "operation": "solana_raw_write",
            "input": {
                "endpoint": server.endpoint,
                "method": "sendTransaction",
                "params": [{"message_base64": message_base64}],
                "confirmation_mode": "confirmed"
            },
            "activation": {
                "secrets": {
                    "signer": signer.to_base58_string()
                }
            }
        })
        .to_string(),
    )
    .await
    .expect("request should succeed");

    let payload: Value = serde_json::from_str(&response).expect("response should decode");
    assert_eq!(payload["success"], true);
    assert_eq!(payload["result_state"], "settled");
    assert_eq!(payload["output"]["status"], "settled");
    assert_eq!(payload["output"]["transaction_id"], server.signature());

    let sent = server.last_transaction();
    assert_eq!(sent.message.account_keys[0], signer.pubkey());
    assert_eq!(sent.message.account_keys[1], recipient);
}

#[tokio::test]
async fn solana_raw_write_rejects_unsupported_method() {
    let signer = test_signer();
    let response = handle_request_json(
        &json!({
            "contract_version": "1.0.0",
            "plugin_id": "solana-node",
            "node_id": "node-3",
            "operation": "solana_raw_write",
            "input": {
                "endpoint": "http://127.0.0.1:1",
                "method": "sendRawTransaction",
                "params": []
            },
            "activation": {
                "secrets": {
                    "signer": signer.to_base58_string()
                }
            }
        })
        .to_string(),
    )
    .await
    .expect("request should return a failure response");

    let payload: Value = serde_json::from_str(&response).expect("response should decode");
    assert_eq!(payload["success"], false);
    assert!(payload["error"]
        .as_str()
        .unwrap_or_default()
        .contains("sendTransaction"));
}

#[derive(Clone)]
struct RpcState {
    recorded: Arc<Mutex<Option<Transaction>>>,
    signature: String,
}

struct TestRpcServer {
    endpoint: String,
    state: RpcState,
}

impl TestRpcServer {
    async fn spawn() -> Self {
        let state = RpcState {
            recorded: Arc::new(Mutex::new(None)),
            signature: String::from("5N9wJx7YxQeK6G6dRzQ6pX6d5j8m3U4n2QyT6Q7c8P9r"),
        };
        let app = Router::new()
            .route("/", post(handle_rpc))
            .with_state(state.clone());
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener should bind");
        let address: SocketAddr = listener.local_addr().expect("listener should have addr");
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("server should run");
        });
        Self {
            endpoint: format!("http://{address}"),
            state,
        }
    }

    fn last_transaction(&self) -> Transaction {
        self.state
            .recorded
            .lock()
            .expect("recorded tx lock should work")
            .clone()
            .expect("transaction should be recorded")
    }

    fn signature(&self) -> &str {
        &self.state.signature
    }
}

async fn handle_rpc(State(state): State<RpcState>, Json(payload): Json<Value>) -> Json<Value> {
    let method = payload["method"].as_str().unwrap_or_default();
    let result = match method {
        "getLatestBlockhash" => json!({
            "value": {
                "blockhash": Hash::new_from_array([7; 32]).to_string()
            }
        }),
        "sendTransaction" => {
            let encoded = payload["params"][0]
                .as_str()
                .expect("encoded tx should be present");
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .expect("tx payload should decode");
            let tx: Transaction = bincode::deserialize(&bytes).expect("tx should deserialize");
            *state
                .recorded
                .lock()
                .expect("recorded tx lock should work") = Some(tx);
            json!(state.signature)
        }
        "getSignatureStatuses" => json!({
            "value": [{"err": null}]
        }),
        _ => json!(null),
    };
    Json(json!({"jsonrpc": "2.0", "id": 1, "result": result}))
}

fn test_signer() -> Keypair {
    Keypair::new_from_array([9; 32])
}
