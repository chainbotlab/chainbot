use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Deserialize)]
pub struct TriggerStartCommand {
    pub protocol_version: String,
    pub trigger_id: String,
    pub source: String,
    #[serde(default)]
    pub params: BTreeMap<String, Value>,
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
    pub dedup_key: Option<String>,
    pub dedup_window_ms: Option<i64>,
}

pub fn parse_start_command(input: &str) -> Result<TriggerStartCommand, String> {
    let value: Value = serde_json::from_str(input).map_err(|error| error.to_string())?;
    if value.get("type").and_then(Value::as_str) != Some("start") {
        return Err(String::from("unsupported host message type"));
    }
    let command: TriggerStartCommand = serde_json::from_value(value).map_err(|error| error.to_string())?;
    if command.protocol_version != "2.0.0" {
        return Err(format!("unsupported protocol_version {}", command.protocol_version));
    }
    Ok(command)
}

pub fn ready_message() -> String {
    serde_json::to_string(&TriggerReady { r#type: "ready", protocol_version: "2.0.0" })
        .unwrap_or_else(|_| String::from("{\"type\":\"ready\",\"protocol_version\":\"2.0.0\"}"))
}

pub fn event_message(frame: TriggerEventFrame) -> String {
    serde_json::to_string(&frame).unwrap_or_else(|_| String::from("{}"))
}

pub fn build_event_frame(checkpoint: String, event_key: String, occurred_at_ms: i64, payload: Value) -> TriggerEventFrame {
    TriggerEventFrame {
        r#type: "event",
        checkpoint,
        event_key: event_key.clone(),
        occurred_at_ms,
        payload,
        dedup_key: Some(event_key),
        dedup_window_ms: Some(60_000),
    }
}

pub fn build_bitget_subscribe_request(command: &TriggerStartCommand) -> Result<Value, String> {
    let inst_type = command.params.get("instType").and_then(Value::as_str).unwrap_or("SPOT");
    let channel = command.params.get("channel").and_then(Value::as_str).unwrap_or("ticker");
    let inst_id = command
        .params
        .get("instId")
        .or_else(|| command.params.get("symbol"))
        .and_then(Value::as_str)
        .ok_or_else(|| String::from("instId or symbol is required"))?;
    Ok(json!({"op": "subscribe", "args": [{"instType": inst_type, "channel": channel, "instId": inst_id}]}))
}
