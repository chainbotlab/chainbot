use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

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
    serde_json::from_value(value).map_err(|error| error.to_string())
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
