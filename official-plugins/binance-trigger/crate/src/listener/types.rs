use serde_json::Value;

#[derive(Debug, Clone, Copy)]
pub enum ProductLine {
    Spot,
    Usdm,
    Coinm,
}

impl ProductLine {
    pub fn parse(input: &str) -> Result<Self, String> {
        match input.trim().to_ascii_lowercase().as_str() {
            "spot" => Ok(Self::Spot),
            "usdm" | "usdsm" | "usd-m" | "usd_m" => Ok(Self::Usdm),
            "coinm" | "coin-m" | "coin_m" => Ok(Self::Coinm),
            other => Err(format!("unsupported product_line {other}")),
        }
    }

    pub fn user_stream_path(self) -> &'static str {
        match self {
            Self::Spot => "/api/v3/userDataStream",
            Self::Usdm => "/fapi/v1/listenKey",
            Self::Coinm => "/dapi/v1/listenKey",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Environment {
    Mainnet,
    Testnet,
}

impl Environment {
    pub fn parse(input: &str) -> Result<Self, String> {
        match input.trim().to_ascii_lowercase().as_str() {
            "mainnet" | "prod" | "production" => Ok(Self::Mainnet),
            "testnet" | "test" | "demo" => Ok(Self::Testnet),
            other => Err(format!("unsupported environment {other}")),
        }
    }
}

pub fn product_line_from_params(params: &std::collections::BTreeMap<String, Value>) -> Result<ProductLine, String> {
    let raw = params
        .get("product_line")
        .and_then(Value::as_str)
        .unwrap_or("spot");
    ProductLine::parse(raw)
}

pub fn environment_from_params(params: &std::collections::BTreeMap<String, Value>) -> Result<Environment, String> {
    let raw = params
        .get("environment")
        .and_then(Value::as_str)
        .unwrap_or("mainnet");
    Environment::parse(raw)
}

pub fn default_market_ws_url(product_line: ProductLine, environment: Environment) -> &'static str {
    match (product_line, environment) {
        (ProductLine::Spot, Environment::Mainnet) => "wss://stream.binance.com:9443/ws",
        (ProductLine::Spot, Environment::Testnet) => "wss://stream.testnet.binance.vision/ws",
        (ProductLine::Usdm, Environment::Mainnet) => "wss://fstream.binance.com/ws",
        (ProductLine::Usdm, Environment::Testnet) => "wss://stream.binancefuture.com/ws",
        (ProductLine::Coinm, Environment::Mainnet) => "wss://dstream.binance.com/ws",
        (ProductLine::Coinm, Environment::Testnet) => "wss://dstream.binancefuture.com/ws",
    }
}

pub fn default_rest_base_url(product_line: ProductLine, environment: Environment) -> &'static str {
    match (product_line, environment) {
        (ProductLine::Spot, Environment::Mainnet) => "https://api.binance.com",
        (ProductLine::Spot, Environment::Testnet) => "https://testnet.binance.vision",
        (ProductLine::Usdm, Environment::Mainnet) => "https://fapi.binance.com",
        (ProductLine::Usdm, Environment::Testnet) => "https://demo-fapi.binance.com",
        (ProductLine::Coinm, Environment::Mainnet) => "https://dapi.binance.com",
        (ProductLine::Coinm, Environment::Testnet) => "https://testnet.binancefuture.com",
    }
}

pub fn default_user_ws_base(product_line: ProductLine, environment: Environment) -> &'static str {
    match (product_line, environment) {
        (ProductLine::Spot, Environment::Mainnet) => "wss://stream.binance.com:9443/ws",
        (ProductLine::Spot, Environment::Testnet) => "wss://stream.testnet.binance.vision/ws",
        (ProductLine::Usdm, Environment::Mainnet) => "wss://fstream.binance.com/private/ws?listenKey={listenKey}",
        (ProductLine::Usdm, Environment::Testnet) => "wss://fstream.binancefuture.com/ws",
        (ProductLine::Coinm, Environment::Mainnet) => "wss://dstream.binance.com/ws",
        (ProductLine::Coinm, Environment::Testnet) => "wss://dstream.binancefuture.com/ws",
    }
}

pub enum MessageOutcome {
    Continue,
    Reconnect,
}

pub enum SocketLoopOutcome {
    Reconnect,
}
