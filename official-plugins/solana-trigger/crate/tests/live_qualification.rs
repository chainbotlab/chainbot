#![cfg(feature = "live-qualification")]

#[test]
#[ignore = "live provider qualification is opt-in and not part of default correctness gates"]
fn live_qualification_requires_explicit_endpoint_configuration() {
    let endpoint = std::env::var("CHAINBOT_SOLANA_TRIGGER_LIVE_ENDPOINT")
        .expect("CHAINBOT_SOLANA_TRIGGER_LIVE_ENDPOINT must be set for live qualification");
    assert!(!endpoint.trim().is_empty());
}
