use axum::{extract::ws::{Message, WebSocket, WebSocketUpgrade}, response::Response, routing::get, Router};
use eth_trigger_official_plugin::run_from_stdin;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;

#[tokio::test]
async fn alchemy_mined_tx_listener_sends_expected_subscription_request() {
    let captured = Arc::new(Mutex::new(Vec::<Value>::new()));
    let endpoint = spawn_ws_server(captured.clone()).await;

    let command = json!({
        "type": "start",
        "protocol_version": "2.0.0",
        "trigger_id": "alchemy-trigger",
        "source": "alchemy_mined_tx",
        "params": {
            "endpoint": endpoint,
            "filter": {"addresses": [{"to": ["0x0000000000000000000000000000000000000001"]}]}
        },
        "activation": {"secrets": {"rpc_token": "test-token"}},
        "heartbeat_interval_ms": 1000,
        "shutdown_grace_ms": 1000
    });

    run_from_stdin(&command.to_string())
        .await
        .expect("alchemy listener should succeed");

    let requests = captured.lock().expect("capture lock should work");
    assert!(!requests.is_empty());
    assert_eq!(requests[0]["method"], "eth_subscribe");
    assert_eq!(requests[0]["params"][0], "alchemy_minedTransactions");
}

async fn spawn_ws_server(captured: Arc<Mutex<Vec<Value>>>) -> String {
    let app = Router::new().route(
        "/",
        get(move |ws: WebSocketUpgrade| {
            let captured = captured.clone();
            async move { ws.on_upgrade(move |socket| handle_socket(socket, captured)) }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener should bind");
    let address: SocketAddr = listener.local_addr().expect("listener should have addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("ws server should run");
    });
    format!("ws://{address}")
}

async fn handle_socket(mut socket: WebSocket, captured: Arc<Mutex<Vec<Value>>>) {
    if let Some(Ok(Message::Text(text))) = socket.next().await {
        let payload: Value = serde_json::from_str(&text).expect("request should decode");
        captured.lock().expect("capture lock should work").push(payload);
        socket
            .send(Message::Text(
                json!({"jsonrpc": "2.0", "id": 1, "result": "subscription-1"})
                    .to_string()
                    .into(),
            ))
            .await
            .expect("subscription ack should send");
        socket
            .send(Message::Text(
                json!({
                    "jsonrpc": "2.0",
                    "method": "eth_subscription",
                    "params": {
                        "subscription": "subscription-1",
                        "result": {
                            "transactionHash": "0x01",
                            "blockNumber": "0x10"
                        }
                    }
                })
                .to_string()
                .into(),
            ))
            .await
            .expect("notification should send");
    }
}
