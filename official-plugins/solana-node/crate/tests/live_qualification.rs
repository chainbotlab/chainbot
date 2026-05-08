#![cfg(feature = "live-qualification")]

use serde_json::json;
use solana_node_official_plugin::handle_request_json;

#[tokio::test]
#[ignore = "live provider qualification is opt-in and not part of default correctness gates"]
async fn live_qualification_reads_health_from_real_provider() {
    let endpoint = std::env::var("CHAINBOT_SOLANA_NODE_LIVE_ENDPOINT")
        .expect("CHAINBOT_SOLANA_NODE_LIVE_ENDPOINT must be set for live qualification");

    let response = handle_request_json(
        &json!({
            "contract_version": "1.0.0",
            "plugin_id": "solana-node",
            "node_id": "live-qualification",
            "operation": "solana_raw_read",
            "input": {
                "endpoint": endpoint,
                "method": "getHealth",
                "params": []
            }
        })
        .to_string(),
    )
    .await
    .expect("live qualification request should complete");

    let payload: serde_json::Value =
        serde_json::from_str(&response).expect("response should decode");
    assert_eq!(payload["success"], true);
    assert!(payload["output"]["result"].is_string() || payload["output"]["result"].is_object());
}
