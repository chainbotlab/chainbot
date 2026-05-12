use std::collections::BTreeMap;

use serde_json::{json, Value};

pub const DEFAULT_SPOT_WS_URL: &str = "wss://api.gateio.ws/ws/v4/";

const PRIVATE_CHANNELS: &[&str] = &["spot.orders", "spot.usertrades", "spot.balances"];

pub enum MessageOutcome {
    Continue,
    Reconnect,
}

pub enum SocketLoopOutcome {
    Reconnect,
}

pub fn channel_from_params(params: &BTreeMap<String, Value>) -> Result<String, String> {
    params
        .get("channel")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| String::from("params.channel is required"))
}

pub fn validate_market_channel(channel: &str) -> Result<(), String> {
    if !channel.starts_with("spot.") {
        return Err(format!("unsupported Gate spot market channel {channel}"));
    }
    if PRIVATE_CHANNELS.contains(&channel) {
        return Err(format!("private Gate channel {channel} requires gate_spot_user_stream"));
    }
    Ok(())
}

pub fn validate_user_channel(channel: &str) -> Result<(), String> {
    if PRIVATE_CHANNELS.contains(&channel) {
        Ok(())
    } else {
        Err(format!("unsupported Gate private channel {channel}"))
    }
}

pub fn payload_from_params(channel: &str, params: &BTreeMap<String, Value>) -> Result<Option<Value>, String> {
    if let Some(payload) = params.get("payload") {
        if payload.is_null() {
            return Ok(None);
        }
        return Ok(Some(normalize_payload_value(payload)?));
    }

    if channel == "spot.balances" {
        return Ok(None);
    }

    let pairs = currency_pairs_from_params(params)?;
    if pairs.is_empty() {
        Err(format!("channel {channel} requires params.payload or currency_pair/currency_pairs"))
    } else {
        Ok(Some(json!(pairs)))
    }
}

pub fn stream_hint(channel: &str, params: &BTreeMap<String, Value>) -> String {
    let pairs = currency_pairs_from_params(params).unwrap_or_default();
    if let Some(first) = pairs.first() {
        format!("{channel}:{first}")
    } else {
        channel.to_owned()
    }
}

fn normalize_payload_value(value: &Value) -> Result<Value, String> {
    match value {
        Value::String(items) => {
            let parsed = items
                .split(',')
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(normalize_currency_pair)
                .collect::<Vec<_>>();
            if parsed.is_empty() {
                return Err(String::from("params.payload must not be empty"));
            }
            Ok(json!(parsed))
        }
        Value::Array(items) => {
            let mut parsed = Vec::new();
            for item in items {
                let pair = item
                    .as_str()
                    .ok_or_else(|| String::from("params.payload array entries must be strings"))?;
                let normalized = normalize_currency_pair(pair);
                if normalized.is_empty() {
                    return Err(String::from("params.payload array entries must not be empty"));
                }
                parsed.push(normalized);
            }
            if parsed.is_empty() {
                return Err(String::from("params.payload must not be empty"));
            }
            Ok(json!(parsed))
        }
        Value::Object(_) => Ok(value.clone()),
        _ => Err(String::from("params.payload must be a string, array, object, or null")),
    }
}

fn currency_pairs_from_params(params: &BTreeMap<String, Value>) -> Result<Vec<String>, String> {
    if let Some(pair) = params.get("currency_pair").and_then(Value::as_str) {
        let normalized = normalize_currency_pair(pair);
        if normalized.is_empty() {
            return Err(String::from("params.currency_pair must not be empty"));
        }
        return Ok(vec![normalized]);
    }

    let Some(raw_pairs) = params.get("currency_pairs") else {
        return Ok(Vec::new());
    };

    match raw_pairs {
        Value::String(items) => {
            let pairs = items
                .split(',')
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(normalize_currency_pair)
                .collect::<Vec<_>>();
            if pairs.is_empty() {
                return Err(String::from("params.currency_pairs must not be empty"));
            }
            Ok(pairs)
        }
        Value::Array(items) => {
            let mut pairs = Vec::new();
            for item in items {
                let pair = item
                    .as_str()
                    .ok_or_else(|| String::from("params.currency_pairs entries must be strings"))?;
                let normalized = normalize_currency_pair(pair);
                if normalized.is_empty() {
                    return Err(String::from("params.currency_pairs entries must not be empty"));
                }
                pairs.push(normalized);
            }
            if pairs.is_empty() {
                return Err(String::from("params.currency_pairs must not be empty"));
            }
            Ok(pairs)
        }
        _ => Err(String::from("params.currency_pairs must be a string or array")),
    }
}

fn normalize_currency_pair(input: &str) -> String {
    input.trim().to_ascii_uppercase()
}
