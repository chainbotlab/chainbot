#![cfg(feature = "live-qualification")]

use axum::{
    extract::State,
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    http::HeaderMap,
    response::Response,
    routing::get,
    Router,
};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use solana_trigger_official_plugin::run_from_stdin;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio_tungstenite::{connect_async, tungstenite::client::IntoClientRequest};

#[derive(Clone)]
struct CaptureProxyState {
    provider_endpoint: String,
}

#[tokio::test]
#[ignore = "live provider qualification is opt-in and not part of default correctness gates"]
async fn live_qualification_subscribes_against_real_provider() {
    let endpoint = std::env::var("CHAINBOT_SOLANA_TRIGGER_LIVE_ENDPOINT")
        .expect("CHAINBOT_SOLANA_TRIGGER_LIVE_ENDPOINT must be set for live qualification");
    let allowed_origin = std::env::var("CHAINBOT_SOLANA_TRIGGER_LIVE_ALLOWED_ORIGIN")
        .unwrap_or_else(|_| endpoint.clone());
    let rpc_token = std::env::var("CHAINBOT_SOLANA_TRIGGER_LIVE_RPC_TOKEN").ok();

    let mut activation = json!({"allowed_origins": [allowed_origin]});
    if let Some(token) = rpc_token {
        activation["secrets"] = json!({"rpc_token": token});
    }

    let command = json!({
        "type": "start",
        "protocol_version": "2.0.0",
        "trigger_id": "solana-live-qualification",
        "source": "solana_account",
        "params": {
            "endpoint": live_capture_endpoint(endpoint.clone()).await,
            "account": "Example1111111111111111111111111111111111111",
            "commitment": "confirmed"
        },
        "activation": activation,
        "heartbeat_interval_ms": 1000,
        "shutdown_grace_ms": 1000
    });

    run_from_stdin(&command.to_string())
        .await
        .expect("live qualification should complete");
}

async fn live_capture_endpoint(provider_endpoint: String) -> String {
    let app = Router::new()
        .route("/", get(ws_capture_handler))
        .with_state(Arc::new(CaptureProxyState { provider_endpoint }));
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener should bind");
    let address: SocketAddr = listener.local_addr().expect("listener should have addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("ws server should run");
    });
    format!("ws://{address}")
}

async fn ws_capture_handler(
    State(state): State<Arc<CaptureProxyState>>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Response {
    let auth_header = headers.get(http::header::AUTHORIZATION).cloned();
    ws.on_upgrade(move |socket| handle_socket(socket, state, auth_header))
}

async fn handle_socket(
    mut socket: WebSocket,
    state: Arc<CaptureProxyState>,
    auth_header: Option<http::HeaderValue>,
) {
    if let Some(Ok(Message::Text(text))) = socket.next().await {
        let payload: Value = serde_json::from_str(&text).expect("request should decode");
        assert_eq!(payload["method"], "accountSubscribe");

        let mut request = state
            .provider_endpoint
            .as_str()
            .into_client_request()
            .expect("provider endpoint should build request");
        if let Some(value) = auth_header {
            request
                .headers_mut()
                .insert(http::header::AUTHORIZATION, value);
        }
        let (mut upstream, _) = connect_async(request)
            .await
            .expect("proxy should connect to provider endpoint");
        upstream
            .send(tokio_tungstenite::tungstenite::Message::Text(text.clone()))
            .await
            .expect("proxy should forward subscription request");
        let upstream_ack = upstream
            .next()
            .await
            .expect("provider should answer subscription")
            .expect("provider ack should succeed");
        let tokio_tungstenite::tungstenite::Message::Text(ack_text) = upstream_ack else {
            panic!("provider ack should be text");
        };
        let ack_payload: Value = serde_json::from_str(&ack_text).expect("ack should decode");
        let subscription_id = ack_payload["result"]
            .as_i64()
            .expect("ack should include numeric subscription id");
        socket
            .send(Message::Text(
                ack_text.into(),
            ))
            .await
            .expect("subscription ack should send");
        socket
            .send(Message::Text(
                json!({
                    "jsonrpc": "2.0",
                    "method": "accountNotification",
                    "params": {
                        "subscription": subscription_id,
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
