use axum::{extract::State, http::HeaderMap, routing::get, Router};
use serde_json::Value;
use serial_test::serial;
use std::sync::{Arc, Mutex};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};
use tokio::net::TcpListener;

#[tokio::test]
#[serial]
async fn okx_get_server_time_returns_public_time() {
    let _guard = LoopbackGuard::set();
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind listener");
    let address = listener.local_addr().expect("local addr");
    let headers = Arc::new(Mutex::new(Vec::<(String, String)>::new()));
    let app = Router::new()
        .route("/api/v5/public/time", get(public_time))
        .route("/api/v5/account/balance", get(private_balance))
        .with_state(headers.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve app");
    });

    let request = serde_json::json!({
        "contract_version": "1.0.0",
        "plugin_id": "okx-node",
        "node_id": "okx-time",
        "operation": "okx_get_server_time",
        "input": {
            "inst_type": "spot",
            "base_url": format!("http://{address}")
        }
    });

    let response = okx_node_official_plugin::handle_request_json(&request.to_string())
        .await
        .expect("successful response");
    let payload: Value = serde_json::from_str(&response).expect("json response");
    assert_eq!(payload["success"], Value::Bool(true));
    assert_eq!(payload["output"]["server_time"], Value::String(String::from("1234567890")));

    server.abort();
    let _ = server.await;
}

#[tokio::test]
#[serial]
async fn okx_get_account_balance_signs_with_required_headers() {
    let _guard = LoopbackGuard::set();
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind listener");
    let address = listener.local_addr().expect("local addr");
    let headers = Arc::new(Mutex::new(Vec::<(String, String)>::new()));
    let app = Router::new()
        .route("/api/v5/public/time", get(public_time))
        .route("/api/v5/account/balance", get(private_balance))
        .with_state(headers.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve app");
    });

    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "node.execute",
        "params": {
            "contract_version": "1.0.0",
            "plugin_id": "okx-node",
            "node_id": "okx-balance",
            "operation": "okx_get_account_balance",
            "input": {
                "inst_type": "spot",
                "base_url": format!("http://{address}"),
                "ccy": "BTC"
            },
            "activation": {
                "allowed_origins": [format!("http://{address}")],
                "secrets": {
                    "api_key": "key-1",
                    "api_secret": "secret-1",
                    "passphrase": "pass-1"
                }
            }
        }
    });

    let response = okx_node_official_plugin::handle_request_json(&request.to_string())
        .await
        .expect("successful response");
    let payload: Value = serde_json::from_str(&response).expect("json response");
    assert_eq!(
        payload["result"]["output"]["balances"][0]["details"][0]["ccy"],
        Value::String(String::from("BTC"))
    );

    let captured = headers.lock().expect("lock headers").clone();
    assert!(captured.iter().any(|(name, _)| name == "ok-access-key"));
    assert!(captured.iter().any(|(name, _)| name == "ok-access-sign"));
    assert!(captured.iter().any(|(name, _)| name == "ok-access-timestamp"));
    assert!(captured.iter().any(|(name, _)| name == "ok-access-passphrase"));
    let timestamp = captured
        .iter()
        .find(|(name, _)| name == "ok-access-timestamp")
        .map(|(_, value)| value.as_str())
        .expect("timestamp header");
    let parsed = OffsetDateTime::parse(timestamp, &Rfc3339).expect("parse timestamp");
    let delta_seconds = (OffsetDateTime::now_utc().unix_timestamp() - parsed.unix_timestamp()).abs();
    assert!(delta_seconds < 300, "expected fresh OKX timestamp, got {timestamp}");

    server.abort();
    let _ = server.await;
}

async fn public_time() -> axum::Json<Value> {
    axum::Json(serde_json::json!({"code":"0","data":[{"ts":"1234567890"}]}))
}

async fn private_balance(
    State(headers): State<Arc<Mutex<Vec<(String, String)>>>>,
    request_headers: HeaderMap,
) -> axum::Json<Value> {
    let mut captured = headers.lock().expect("lock headers");
    captured.extend(request_headers.iter().map(|(name, value)| {
        (
            name.as_str().to_owned(),
            value.to_str().unwrap_or("<non-utf8>").to_owned(),
        )
    }));
    axum::Json(serde_json::json!({
        "code":"0",
        "data":[{"details":[{"ccy":"BTC","availBal":"1.25"}]}]
    }))
}

struct LoopbackGuard;

impl LoopbackGuard {
    fn set() -> Self {
        unsafe {
            std::env::set_var("CHAINBOT_HTTP_NODE_ALLOW_LOOPBACK_FOR_TESTS", "1");
            std::env::set_var("CHAINBOT_INTERNAL_ALLOW_TEST_DESTINATIONS", "1");
        }
        Self
    }
}

impl Drop for LoopbackGuard {
    fn drop(&mut self) {
        unsafe {
            std::env::remove_var("CHAINBOT_HTTP_NODE_ALLOW_LOOPBACK_FOR_TESTS");
            std::env::remove_var("CHAINBOT_INTERNAL_ALLOW_TEST_DESTINATIONS");
        }
    }
}
