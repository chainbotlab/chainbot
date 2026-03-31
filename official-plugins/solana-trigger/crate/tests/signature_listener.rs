use solana_trigger_official_plugin::run_from_stdin;

#[tokio::test]
async fn solana_signature_mock_listener_emits_event_frame() {
    let command = serde_json::json!({
        "type": "start",
        "protocol_version": "2.0.0",
        "trigger_id": "solana-signature-trigger",
        "source": "solana_signature",
        "params": {"endpoint": "mock://solana-signature", "network": "devnet"},
        "heartbeat_interval_ms": 1000,
        "shutdown_grace_ms": 1000
    });

    run_from_stdin(&command.to_string())
        .await
        .expect("mock solana_signature listener should succeed");
}
