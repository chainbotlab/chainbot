use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone, Deserialize)]
pub struct TriggerStartCommand {
    pub protocol_version: String,
    pub trigger_id: String,
    pub source: String,
    #[serde(default)]
    pub params: BTreeMap<String, Value>,
    #[serde(default)]
    pub resume_checkpoint: Option<String>,
    #[serde(default)]
    pub activation: Option<PluginActivationEnvelope>,
    pub heartbeat_interval_ms: i64,
    pub shutdown_grace_ms: i64,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct PluginActivationEnvelope {
    #[serde(default)]
    pub secrets: BTreeMap<String, String>,
    #[serde(default)]
    pub allowed_origins: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TriggerReady {
    pub r#type: &'static str,
    pub protocol_version: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct TriggerEventFrame {
    pub r#type: &'static str,
    pub checkpoint: String,
    pub event_key: String,
    pub occurred_at_ms: i64,
    pub payload: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dedup_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dedup_window_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cooldown_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cooldown_ms: Option<i64>,
}

pub fn parse_start_command(input: &str) -> Result<TriggerStartCommand, String> {
    let value: Value = serde_json::from_str(input).map_err(|error| error.to_string())?;
    let start = value
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| String::from("missing message type"))?;
    if start != "start" {
        return Err(format!("unsupported host message type {start}"));
    }
    let command: TriggerStartCommand = serde_json::from_value(value).map_err(|error| error.to_string())?;
    if command.protocol_version != "2.0.0" {
        return Err(format!("unsupported protocol_version {}", command.protocol_version));
    }
    Ok(command)
}

pub fn ready_message() -> String {
    serde_json::to_string(&TriggerReady {
        r#type: "ready",
        protocol_version: "2.0.0",
    })
    .unwrap_or_else(|_| String::from("{\"type\":\"ready\",\"protocol_version\":\"2.0.0\"}"))
}

pub fn event_message(frame: TriggerEventFrame) -> String {
    serde_json::to_string(&frame).unwrap_or_else(|_| String::from("{}"))
}

pub fn build_event_frame(
    checkpoint: String,
    event_key: String,
    occurred_at_ms: i64,
    payload: Value,
) -> TriggerEventFrame {
    TriggerEventFrame {
        r#type: "event",
        checkpoint,
        event_key: event_key.clone(),
        occurred_at_ms,
        payload,
        dedup_key: Some(event_key),
        dedup_window_ms: Some(60_000),
        cooldown_key: None,
        cooldown_ms: None,
    }
}

pub fn build_subscription_request(command: &TriggerStartCommand) -> Result<Value, String> {
    match command.source.as_str() {
        "okx_market_stream" | "okx_private_stream" => Ok(json!({
            "op": "subscribe",
            "args": subscription_args(command)?
        })),
        other => Err(format!("unsupported OKX trigger source {other}")),
    }
}

pub fn build_login_request(command: &TriggerStartCommand) -> Result<Value, String> {
    let activation = command.activation.as_ref().ok_or_else(|| String::from("activation is required"))?;
    let api_key = activation
        .secrets
        .get("api_key")
        .ok_or_else(|| String::from("activation api_key is required"))?;
    let api_secret = activation
        .secrets
        .get("api_secret")
        .ok_or_else(|| String::from("activation api_secret is required"))?;
    let passphrase = activation
        .secrets
        .get("passphrase")
        .ok_or_else(|| String::from("activation passphrase is required"))?;
    let timestamp = login_timestamp();
    let signature = sign_login_payload(&timestamp, api_secret)?;
    Ok(json!({
        "op": "login",
        "args": [{
            "apiKey": api_key,
            "passphrase": passphrase,
            "timestamp": timestamp,
            "sign": signature
        }]
    }))
}

fn subscription_args(command: &TriggerStartCommand) -> Result<Vec<Value>, String> {
    let channel = command
        .params
        .get("channel")
        .and_then(Value::as_str)
        .ok_or_else(|| String::from("params.channel is required"))?;
    let inst_type = command
        .params
        .get("inst_type")
        .and_then(Value::as_str)
        .unwrap_or("SPOT");
    let mut arg = serde_json::Map::new();
    arg.insert(String::from("channel"), Value::String(channel.to_owned()));
    arg.insert(String::from("instType"), Value::String(inst_type.to_owned()));
    if let Some(inst_id) = command.params.get("inst_id").and_then(Value::as_str) {
        arg.insert(String::from("instId"), Value::String(inst_id.to_owned()));
    }
    Ok(vec![Value::Object(arg)])
}

fn sign_login_payload(timestamp: &str, api_secret: &str) -> Result<String, String> {
    let mut mac = HmacSha256::new_from_slice(api_secret.as_bytes()).map_err(|error| error.to_string())?;
    mac.update(timestamp.as_bytes());
    mac.update(b"GET");
    mac.update(b"/users/self/verify");
    Ok(base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes()))
}

fn login_timestamp() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs().to_string())
        .unwrap_or_else(|_| String::from("0"))
}
