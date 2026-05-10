use serde_json::Value;

#[derive(Debug, Clone, Copy)]
pub enum ProductLine {
    Spot,
    Swap,
    Futures,
    Option,
}

impl ProductLine {
    pub fn parse(input: &str) -> Result<Self, String> {
        match input.trim().to_ascii_lowercase().as_str() {
            "spot" => Ok(Self::Spot),
            "swap" => Ok(Self::Swap),
            "futures" | "future" => Ok(Self::Futures),
            "option" | "options" => Ok(Self::Option),
            other => Err(format!("unsupported inst_type {other}")),
        }
    }

    pub fn as_api_value(self) -> &'static str {
        match self {
            Self::Spot => "SPOT",
            Self::Swap => "SWAP",
            Self::Futures => "FUTURES",
            Self::Option => "OPTION",
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
    let raw = params.get("inst_type").and_then(Value::as_str).unwrap_or("SPOT");
    ProductLine::parse(raw)
}

pub fn environment_from_params(params: &std::collections::BTreeMap<String, Value>) -> Result<Environment, String> {
    let raw = params
        .get("environment")
        .and_then(Value::as_str)
        .unwrap_or("mainnet");
    Environment::parse(raw)
}

pub fn default_market_ws_url(_product_line: ProductLine, environment: Environment) -> &'static str {
    match environment {
        Environment::Mainnet => "wss://ws.okx.com:8443/ws/v5/public",
        Environment::Testnet => "wss://wspap.okx.com:8443/ws/v5/public",
    }
}

pub fn default_private_ws_url(_product_line: ProductLine, environment: Environment) -> &'static str {
    match environment {
        Environment::Mainnet => "wss://ws.okx.com:8443/ws/v5/private",
        Environment::Testnet => "wss://wspap.okx.com:8443/ws/v5/private",
    }
}

pub enum MessageOutcome {
    Continue,
    Reconnect,
}

pub enum SocketLoopOutcome {
    Reconnect,
}
