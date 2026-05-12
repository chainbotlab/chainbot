use aster_trigger_official_plugin::{contract::parse_start_command, listener::run_listener_with_writer};
use futures_util::FutureExt;
use serde_json::Value;

#[tokio::test]
#[serial_test::serial]
async fn aster_market_mock_listener_emits_event_frame() {
    let _loopback = LoopbackGuard::set();
    let command = serde_json::json!({
        "type": "start",
        "protocol_version": "2.0.0",
        "trigger_id": "aster-market-trigger",
        "source": "aster_market_stream",
        "params": {
            "endpoint": "mock://aster-market",
            "stream": "btcusdt@trade"
        },
        "heartbeat_interval_ms": 1000,
        "shutdown_grace_ms": 1000
    });

    let mut stdout = Vec::new();
    run_listener_with_writer(
        parse_start_command(&command.to_string()).expect("valid start command"),
        &mut stdout,
    )
    .await
    .expect("mock aster market listener should succeed");

    let frames = output_lines(&stdout);
    assert_eq!(frames.len(), 2, "mock market path should emit ready + event");
    assert_eq!(frames[0]["type"], "ready");
    assert_eq!(frames[0]["protocol_version"], "2.0.0");
    assert_eq!(frames[1]["type"], "event");
    assert_eq!(frames[1]["dedup_key"], frames[1]["event_key"]);
    assert_eq!(frames[1]["dedup_window_ms"], 60_000);
    assert_eq!(frames[1]["payload"]["exchange"], "aster");
    assert_eq!(frames[1]["payload"]["listener_kind"], "market_stream");
    assert_eq!(frames[1]["payload"]["stream"], "btcusdt@trade");
}

#[tokio::test]
#[serial_test::serial]
async fn unsupported_source_does_not_emit_ready() {
    let command = serde_json::json!({
        "type": "start",
        "protocol_version": "2.0.0",
        "trigger_id": "aster-unsupported-trigger",
        "source": "aster_unknown_stream",
        "params": {},
        "heartbeat_interval_ms": 1000,
        "shutdown_grace_ms": 1000
    });

    let mut stdout = Vec::new();
    let error = run_listener_with_writer(
        parse_start_command(&command.to_string()).expect("valid start command"),
        &mut stdout,
    )
    .await
    .expect_err("unsupported source should fail");

    assert!(error.contains("unsupported Aster trigger source"));
    assert!(stdout.is_empty(), "startup failure should not emit ready output");
}

#[tokio::test]
#[serial_test::serial]
async fn unsupported_protocol_version_does_not_emit_ready() {
    let command = serde_json::json!({
        "type": "start",
        "protocol_version": "3.0.0",
        "trigger_id": "aster-unsupported-protocol",
        "source": "aster_market_stream",
        "params": {
            "endpoint": "mock://aster-market",
            "stream": "btcusdt@trade"
        },
        "heartbeat_interval_ms": 1000,
        "shutdown_grace_ms": 1000
    });

    let error = parse_start_command(&command.to_string())
        .expect_err("unsupported protocol version should fail");
    assert!(error.contains("unsupported protocol_version"));
}

#[test]
fn insecure_market_endpoint_scheme_is_rejected() {
    let command = serde_json::json!({
        "type": "start",
        "protocol_version": "2.0.0",
        "trigger_id": "aster-insecure-endpoint",
        "source": "aster_market_stream",
        "params": {
            "endpoint": "ws://example.com/ws",
            "stream": "btcusdt@trade"
        },
        "activation": {
            "allowed_origins": ["https://example.com:443"]
        },
        "heartbeat_interval_ms": 1000,
        "shutdown_grace_ms": 1000
    });

    let mut stdout = Vec::new();
    let error = run_listener_with_writer(
        parse_start_command(&command.to_string()).expect("valid start command"),
        &mut stdout,
    )
    .now_or_never()
    .expect("future should resolve immediately")
    .expect_err("insecure scheme should fail");

    assert!(error.contains("destination url must use https or wss"));
    assert!(stdout.is_empty(), "startup failure should not emit ready output");
}

#[test]
fn default_market_endpoint_uses_documented_raw_stream_path() {
    let command = serde_json::json!({
        "type": "start",
        "protocol_version": "2.0.0",
        "trigger_id": "aster-default-endpoint",
        "source": "aster_market_stream",
        "params": {
            "stream": "btcusdt@trade"
        },
        "heartbeat_interval_ms": 1000,
        "shutdown_grace_ms": 1000
    });

    let parsed = parse_start_command(&command.to_string()).expect("valid start command");
    let request = aster_trigger_official_plugin::contract::build_subscription_request(&parsed)
        .expect("subscription request should build");
    let params = request["params"].as_array().expect("params array");
    let stream = params[0].as_str().expect("stream name");
    let endpoint = format!("wss://fstream.asterdex.com/ws/{stream}");

    assert_eq!(endpoint, "wss://fstream.asterdex.com/ws/btcusdt@trade");
}

fn output_lines(buffer: &[u8]) -> Vec<Value> {
    String::from_utf8(buffer.to_vec())
        .expect("utf8 stdout")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("json frame"))
        .collect()
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
