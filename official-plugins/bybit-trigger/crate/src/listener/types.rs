use std::collections::BTreeMap;

use serde_json::Value;

#[derive(Debug, Clone, Copy)]
pub enum ProductLine {
    Spot,
    Linear,
    Inverse,
    Option,
}

impl ProductLine {
    pub fn parse(input: &str) -> Result<Self, String> {
        match input.trim().to_ascii_lowercase().as_str() {
            "spot" => Ok(Self::Spot),
            "linear" => Ok(Self::Linear),
            "inverse" => Ok(Self::Inverse),
            "option" => Ok(Self::Option),
            other => Err(format!("unsupported product_line {other}")),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Spot => "spot",
            Self::Linear => "linear",
            Self::Inverse => "inverse",
            Self::Option => "option",
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

pub fn product_line_from_params(params: &BTreeMap<String, Value>) -> Result<ProductLine, String> {
    let raw = params
        .get("product_line")
        .and_then(Value::as_str)
        .unwrap_or("spot");
    ProductLine::parse(raw)
}

pub fn environment_from_params(params: &BTreeMap<String, Value>) -> Result<Environment, String> {
    let raw = params
        .get("environment")
        .and_then(Value::as_str)
        .unwrap_or("mainnet");
    Environment::parse(raw)
}

pub fn default_market_ws_url(product_line: ProductLine, environment: Environment) -> String {
    let host = match environment {
        Environment::Mainnet => "wss://stream.bybit.com/v5/public",
        Environment::Testnet => "wss://stream-testnet.bybit.com/v5/public",
    };
    format!("{host}/{}", product_line.as_str())
}

pub fn default_private_ws_url(environment: Environment) -> &'static str {
    match environment {
        Environment::Mainnet => "wss://stream.bybit.com/v5/private",
        Environment::Testnet => "wss://stream-testnet.bybit.com/v5/private",
    }
}

pub enum MessageOutcome {
    Continue,
    Reconnect,
}

pub enum SocketLoopOutcome {
    Reconnect,
}
