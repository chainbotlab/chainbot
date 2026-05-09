#![cfg(feature = "live-qualification")]

#[tokio::test]
#[ignore = "live provider qualification is opt-in and not part of default correctness gates"]
async fn live_qualification_requires_real_binance_endpoints() {
    let _ = std::env::var("CHAINBOT_BINANCE_TRIGGER_LIVE_ENDPOINT")
        .expect("CHAINBOT_BINANCE_TRIGGER_LIVE_ENDPOINT must be set for live qualification");
}
