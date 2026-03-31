use solana_trigger_official_plugin::run_from_stdin;

#[tokio::test]
async fn solana_logs_mock_listener_emits_event_frame() {
    let command = serde_json::json!({
        "type": "start",
        "protocol_version": "2.0.0",
        "trigger_id": "solana-logs-trigger",
        "source": "solana_logs",
        "params": {"endpoint": "mock://solana-logs", "network": "devnet"},
        "heartbeat_interval_ms": 1000,
        "shutdown_grace_ms": 1000
    });

    run_from_stdin(&command.to_string())
        .await
        .expect("mock solana_logs listener should succeed");
}
