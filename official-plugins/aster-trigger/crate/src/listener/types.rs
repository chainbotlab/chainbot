pub const DEFAULT_MARKET_WS_BASE_URL: &str = "wss://fstream.asterdex.com";

pub enum MessageOutcome {
    Continue,
    Reconnect,
}

pub enum SocketLoopOutcome {
    Reconnect,
}
