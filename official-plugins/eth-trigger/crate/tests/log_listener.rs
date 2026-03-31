use eth_trigger_official_plugin::run_from_stdin;

#[tokio::test]
async fn eth_log_mock_listener_emits_event_frame() {
    let command = serde_json::json!({
        "type": "start",
        "protocol_version": "2.0.0",
        "trigger_id": "eth-log-trigger",
        "source": "eth_log",
        "params": {"endpoint": "mock://eth-log", "network": "sepolia"},
        "heartbeat_interval_ms": 1000,
        "shutdown_grace_ms": 1000
    });

    run_from_stdin(&command.to_string())
        .await
        .expect("mock eth_log listener should succeed");
}
