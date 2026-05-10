use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};

#[tokio::test]
async fn okx_market_mock_listener_emits_ready_then_event() {
    let command = serde_json::json!({
        "type": "start",
        "protocol_version": "2.0.0",
        "trigger_id": "okx-market-mock",
        "source": "okx_market_stream",
        "params": {
            "endpoint": "mock://market",
            "inst_type": "SPOT",
            "channel": "tickers",
            "inst_id": "BTC-USDT"
        },
        "heartbeat_interval_ms": 1000,
        "shutdown_grace_ms": 1000
    });
    let mut stdout = Vec::new();
    okx_trigger_official_plugin::listener::run_listener_with_writer(
        &serde_json::from_value(command).expect("command"),
        &mut stdout,
    )
    .await
    .expect("mock listener success");

    let output = String::from_utf8(stdout).expect("utf8 stdout");
    let frames = output.lines().collect::<Vec<_>>();
    assert_eq!(frames.len(), 2);
    let ready: Value = serde_json::from_str(frames[0]).expect("ready frame");
    let event: Value = serde_json::from_str(frames[1]).expect("event frame");
    assert_eq!(ready["type"], Value::String(String::from("ready")));
    assert_eq!(event["type"], Value::String(String::from("event")));
}

#[tokio::test]
async fn okx_private_mock_listener_emits_ready_then_event() {
    let command = serde_json::json!({
        "type": "start",
        "protocol_version": "2.0.0",
        "trigger_id": "okx-private-mock",
        "source": "okx_private_stream",
        "params": {
            "endpoint": "mock://private",
            "inst_type": "SPOT",
            "channel": "account"
        },
        "heartbeat_interval_ms": 1000,
        "shutdown_grace_ms": 1000
    });
    let mut stdout = Vec::new();
    okx_trigger_official_plugin::listener::run_listener_with_writer(
        &serde_json::from_value(command).expect("command"),
        &mut stdout,
    )
    .await
    .expect("mock listener success");

    let output = String::from_utf8(stdout).expect("utf8 stdout");
    let frames = output.lines().collect::<Vec<_>>();
    assert_eq!(frames.len(), 2);
}

#[tokio::test]
async fn unsupported_source_fails_without_ready() {
    let command = serde_json::json!({
        "type": "start",
        "protocol_version": "2.0.0",
        "trigger_id": "bad-source",
        "source": "okx_unknown_stream",
        "params": {},
        "heartbeat_interval_ms": 1000,
        "shutdown_grace_ms": 1000
    });
    let mut stdout = Vec::new();
    let error = okx_trigger_official_plugin::listener::run_listener_with_writer(
        &serde_json::from_value(command).expect("command"),
        &mut stdout,
    )
    .await
    .expect_err("unsupported source must fail");
    assert!(error.contains("unsupported OKX trigger source"));
    assert!(stdout.is_empty());
}

#[test]
fn insecure_endpoint_fails_validation() {
    let command = serde_json::json!({
        "type": "start",
        "protocol_version": "2.0.0",
        "trigger_id": "bad-endpoint",
        "source": "okx_market_stream",
        "params": {
            "endpoint": "ws://example.com/ws",
            "inst_type": "SPOT",
            "channel": "tickers",
            "inst_id": "BTC-USDT"
        },
        "activation": {
            "allowed_origins": ["https://ws.okx.com:8443"]
        },
        "heartbeat_interval_ms": 1000,
        "shutdown_grace_ms": 1000
    });
    let mut stdout = Vec::new();
    let runtime = tokio::runtime::Runtime::new().expect("runtime");
    let error = runtime
        .block_on(okx_trigger_official_plugin::listener::run_listener_with_writer(
            &serde_json::from_value(command).expect("command"),
            &mut stdout,
        ))
        .expect_err("insecure endpoint must fail");
    assert!(error.contains("destination url must use https or wss") || error.contains("not allowlisted"));
    assert!(stdout.is_empty());
}

#[test]
fn login_request_uses_fresh_epoch_timestamp() {
    let command = serde_json::json!({
        "type": "start",
        "protocol_version": "2.0.0",
        "trigger_id": "okx-private-login",
        "source": "okx_private_stream",
        "params": {
            "inst_type": "SPOT",
            "channel": "account"
        },
        "activation": {
            "secrets": {
                "api_key": "key-1",
                "api_secret": "secret-1",
                "passphrase": "pass-1"
            }
        },
        "heartbeat_interval_ms": 1000,
        "shutdown_grace_ms": 1000
    });
    let request = okx_trigger_official_plugin::contract::build_login_request(
        &serde_json::from_value(command).expect("command"),
    )
    .expect("login request");
    let timestamp = request["args"][0]["timestamp"]
        .as_str()
        .expect("login timestamp");
    let parsed = timestamp.parse::<u64>().expect("epoch seconds");
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("unix epoch")
        .as_secs();
    assert!(now.abs_diff(parsed) < 300, "expected fresh login timestamp, got {timestamp}");
}
