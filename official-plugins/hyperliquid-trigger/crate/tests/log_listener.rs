use futures_util::FutureExt;
use hyperliquid_trigger_official_plugin::{contract::parse_start_command, listener::run_listener_with_writer};
use serde_json::Value;

#[tokio::test]
#[serial_test::serial]
async fn hyperliquid_trades_mock_listener_emits_event_frame() {
    let _loopback = LoopbackGuard::set();
    let command = serde_json::json!({
        "type": "start",
        "protocol_version": "2.0.0",
        "trigger_id": "hyperliquid-trades-trigger",
        "source": "hyperliquid_trades",
        "params": {
            "endpoint": "mock://hyperliquid-trades",
            "coin": "BTC"
        },
        "heartbeat_interval_ms": 1000,
        "shutdown_grace_ms": 1000
    });

    let mut stdout = Vec::new();
    run_listener_with_writer(parse_start_command(&command.to_string()).expect("valid start command"), &mut stdout)
        .await
        .expect("mock hyperliquid trades listener should succeed");

    let frames = output_lines(&stdout);
    assert_eq!(frames.len(), 2, "mock trades path should emit ready + event");
    assert_eq!(frames[0]["type"], "ready");
    assert_eq!(frames[0]["protocol_version"], "2.0.0");
    assert_eq!(frames[1]["type"], "event");
    assert_eq!(frames[1]["dedup_key"], frames[1]["event_key"]);
    assert_eq!(frames[1]["dedup_window_ms"], 60_000);
    assert_eq!(frames[1]["payload"]["exchange"], "hyperliquid");
    assert_eq!(frames[1]["payload"]["listener_kind"], "market_stream");
    assert_eq!(frames[1]["payload"]["channel"], "trades");
    assert_eq!(frames[1]["payload"]["coin"], "BTC");
}

#[tokio::test]
#[serial_test::serial]
async fn hyperliquid_l2_book_mock_listener_emits_event_frame() {
    let _loopback = LoopbackGuard::set();
    let command = serde_json::json!({
        "type": "start",
        "protocol_version": "2.0.0",
        "trigger_id": "hyperliquid-l2-book-trigger",
        "source": "hyperliquid_l2_book",
        "params": {
            "endpoint": "mock://hyperliquid-l2-book",
            "coin": "ETH"
        },
        "heartbeat_interval_ms": 1000,
        "shutdown_grace_ms": 1000
    });

    let mut stdout = Vec::new();
    run_listener_with_writer(parse_start_command(&command.to_string()).expect("valid start command"), &mut stdout)
        .await
        .expect("mock hyperliquid l2 book listener should succeed");

    let frames = output_lines(&stdout);
    assert_eq!(frames.len(), 2, "mock l2 book path should emit ready + event");
    assert_eq!(frames[0]["type"], "ready");
    assert_eq!(frames[1]["type"], "event");
    assert_eq!(frames[1]["payload"]["channel"], "l2Book");
    assert_eq!(frames[1]["payload"]["coin"], "ETH");
}

#[tokio::test]
#[serial_test::serial]
async fn unsupported_source_does_not_emit_ready() {
    let command = serde_json::json!({
        "type": "start",
        "protocol_version": "2.0.0",
        "trigger_id": "hyperliquid-unsupported-trigger",
        "source": "hyperliquid_unknown",
        "params": {
            "coin": "BTC"
        },
        "heartbeat_interval_ms": 1000,
        "shutdown_grace_ms": 1000
    });

    let error = parse_start_command(&command.to_string()).expect_err("unsupported source should fail");

    assert!(error.contains("unsupported Hyperliquid trigger source"));
}

#[tokio::test]
#[serial_test::serial]
async fn missing_coin_does_not_emit_ready() {
    let command = serde_json::json!({
        "type": "start",
        "protocol_version": "2.0.0",
        "trigger_id": "hyperliquid-missing-coin",
        "source": "hyperliquid_trades",
        "params": {},
        "heartbeat_interval_ms": 1000,
        "shutdown_grace_ms": 1000
    });

    let error = parse_start_command(&command.to_string()).expect_err("missing coin should fail");
    assert!(error.contains("params.coin is required"));
}

#[test]
fn insecure_market_endpoint_scheme_is_rejected() {
    let command = serde_json::json!({
        "type": "start",
        "protocol_version": "2.0.0",
        "trigger_id": "hyperliquid-insecure-endpoint",
        "source": "hyperliquid_trades",
        "params": {
            "endpoint": "ws://example.com/ws",
            "coin": "BTC"
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

    assert!(error.contains("destination url must use wss"));
    assert!(stdout.is_empty(), "startup failure should not emit ready output");
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
