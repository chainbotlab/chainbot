#[cfg(feature = "live-qualification")]
#[tokio::test]
#[ignore = "opt-in live qualification only"]
async fn live_qualification_placeholder() {
    let _ = std::env::var("CHAINBOT_HTTP_NODE_LIVE_URL");
}
