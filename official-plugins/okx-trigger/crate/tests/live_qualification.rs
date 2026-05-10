#[cfg(feature = "live-qualification")]
#[test]
fn live_endpoint_configuration_is_present() {
    let endpoint = std::env::var("CHAINBOT_OKX_TRIGGER_LIVE_ENDPOINT")
        .expect("CHAINBOT_OKX_TRIGGER_LIVE_ENDPOINT must be set for live qualification");
    assert!(!endpoint.trim().is_empty(), "live qualification endpoint must not be empty");
}
