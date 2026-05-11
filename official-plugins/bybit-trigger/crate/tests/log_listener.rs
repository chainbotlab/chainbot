use bybit_trigger_official_plugin::{contract::parse_start_command, listener::run_listener_with_writer};
use futures_util::FutureExt;
use serde_json::Value;

#[tokio::test]
#[serial_test::serial]
async fn bybit_market_mock_listener_emits_event_frame() {
    let command = serde_json::json!({
        "type": "start",
        "protocol_version": "2.0.0",
        "trigger_id": "bybit-market-trigger",
        "source": "bybit_market_stream",
        "params": {
            "endpoint": "mock://bybit-market",
            "product_line": "spot",
            "stream": "publicTrade.BTCUSDT"
        },
        "heartbeat_interval_ms": 1000,
        "shutdown_grace_ms": 1000
    });

    let mut stdout = Vec::new();
    run_listener_with_writer(parse_start_command(&command.to_string()).expect("valid start command"), &mut stdout)
        .await
        .expect("mock bybit market listener should succeed");

    let frames = output_lines(&stdout);
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[0]["type"], "ready");
    assert_eq!(frames[1]["type"], "event");
    assert_eq!(frames[1]["payload"]["exchange"], "bybit");
    assert_eq!(frames[1]["payload"]["listener_kind"], "market_stream");
    assert_eq!(frames[1]["payload"]["stream"], "publicTrade.BTCUSDT");
}

#[tokio::test]
#[serial_test::serial]
async fn bybit_user_mock_listener_emits_event_frame() {
    let command = serde_json::json!({
        "type": "start",
        "protocol_version": "2.0.0",
        "trigger_id": "bybit-user-trigger",
        "source": "bybit_user_stream",
        "params": {
            "endpoint": "mock://bybit-user",
            "product_line": "linear",
            "stream": "order.linear"
        },
        "activation": {
            "allowed_origins": ["https://stream.bybit.com"]
        },
        "heartbeat_interval_ms": 1000,
        "shutdown_grace_ms": 1000
    });

    let mut stdout = Vec::new();
    run_listener_with_writer(parse_start_command(&command.to_string()).expect("valid start command"), &mut stdout)
        .await
        .expect("mock bybit user listener should succeed");

    let frames = output_lines(&stdout);
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[0]["type"], "ready");
    assert_eq!(frames[1]["type"], "event");
    assert_eq!(frames[1]["payload"]["listener_kind"], "user_stream");
}

#[tokio::test]
#[serial_test::serial]
async fn unsupported_source_does_not_emit_ready() {
    let command = serde_json::json!({
        "type": "start",
        "protocol_version": "2.0.0",
        "trigger_id": "bybit-unsupported-trigger",
        "source": "bybit_unknown_stream",
        "params": {},
        "heartbeat_interval_ms": 1000,
        "shutdown_grace_ms": 1000
    });

    let mut stdout = Vec::new();
    let error = run_listener_with_writer(parse_start_command(&command.to_string()).expect("valid start command"), &mut stdout)
        .await
        .expect_err("unsupported source should fail");

    assert!(error.contains("unsupported Bybit trigger source"));
    assert!(stdout.is_empty());
}

#[tokio::test]
#[serial_test::serial]
async fn unsupported_protocol_version_does_not_emit_ready() {
    let command = serde_json::json!({
        "type": "start",
        "protocol_version": "3.0.0",
        "trigger_id": "bybit-unsupported-protocol",
        "source": "bybit_market_stream",
        "params": {
            "endpoint": "mock://bybit-market",
            "product_line": "spot",
            "stream": "publicTrade.BTCUSDT"
        },
        "heartbeat_interval_ms": 1000,
        "shutdown_grace_ms": 1000
    });

    let error = parse_start_command(&command.to_string()).expect_err("unsupported protocol version should fail");
    assert!(error.contains("unsupported protocol_version"));
}

#[test]
fn insecure_market_endpoint_scheme_is_rejected() {
    let command = serde_json::json!({
        "type": "start",
        "protocol_version": "2.0.0",
        "trigger_id": "bybit-insecure-endpoint",
        "source": "bybit_market_stream",
        "params": {
            "endpoint": "ws://example.com/ws",
            "product_line": "spot",
            "stream": "publicTrade.BTCUSDT"
        },
        "activation": {
            "allowed_origins": ["https://example.com:443"]
        },
        "heartbeat_interval_ms": 1000,
        "shutdown_grace_ms": 1000
    });

    let mut stdout = Vec::new();
    let error = run_listener_with_writer(parse_start_command(&command.to_string()).expect("valid start command"), &mut stdout)
        .now_or_never()
        .expect("future should resolve immediately")
        .expect_err("insecure scheme should fail");

    assert!(error.contains("destination url must use https or wss"));
    assert!(stdout.is_empty());
}

fn output_lines(buffer: &[u8]) -> Vec<Value> {
    String::from_utf8(buffer.to_vec())
        .expect("utf8 stdout")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("json frame"))
        .collect()
}
