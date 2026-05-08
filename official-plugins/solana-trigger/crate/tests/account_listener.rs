use axum::{extract::ws::{Message, WebSocket, WebSocketUpgrade}, routing::get, Router};
use futures_util::StreamExt;
use serde_json::{json, Value};
use solana_trigger_official_plugin::run_from_stdin;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;

#[tokio::test]
async fn solana_account_listener_sends_expected_subscription_request() {
    let captured = Arc::new(Mutex::new(Vec::<Value>::new()));
    let endpoint = spawn_ws_server(captured.clone()).await;
    let command = json!({
        "type": "start",
        "protocol_version": "2.0.0",
        "trigger_id": "solana-account-trigger",
        "source": "solana_account",
        "params": {
            "endpoint": endpoint,
            "account": "Example1111111111111111111111111111111111111",
            "commitment": "confirmed"
        },
        "heartbeat_interval_ms": 1000,
        "shutdown_grace_ms": 1000
    });

    run_from_stdin(&command.to_string())
        .await
        .expect("account listener should succeed");

    let requests = captured.lock().expect("capture lock should work");
    assert!(!requests.is_empty());
    assert_eq!(requests[0]["method"], "accountSubscribe");
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
                json!({"jsonrpc": "2.0", "id": 1, "result": 1})
                    .to_string()
                    .into(),
            ))
            .await
            .expect("subscription ack should send");
        socket
            .send(Message::Text(
                json!({
                    "jsonrpc": "2.0",
                    "method": "accountNotification",
                    "params": {
                        "subscription": 1,
                        "result": {
                            "context": {"slot": 10},
                            "value": {"lamports": 1}
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
