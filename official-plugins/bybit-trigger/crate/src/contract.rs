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
        "bybit_market_stream" => {
            let args = market_stream_args(command)?;
            Ok(json!({
                "op": "subscribe",
                "args": args,
            }))
        }
        "bybit_user_stream" => Ok(json!({
            "op": "subscribe",
            "args": user_stream_topics(command)?,
        })),
        other => Err(format!("unsupported Bybit trigger source {other}")),
    }
}

fn market_stream_args(command: &TriggerStartCommand) -> Result<Vec<String>, String> {
    if let Some(stream) = command.params.get("stream").and_then(Value::as_str) {
        let value = stream.trim();
        if value.is_empty() {
            return Err(String::from("params.stream must not be empty"));
        }
        return Ok(vec![value.to_owned()]);
    }

    if let Some(streams) = command.params.get("streams") {
        if let Some(raw) = streams.as_str() {
            let parsed = raw
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>();
            if parsed.is_empty() {
                return Err(String::from("params.streams must not be empty"));
            }
            return Ok(parsed);
        }

        if let Some(items) = streams.as_array() {
            let mut parsed = Vec::new();
            for item in items {
                let stream = item
                    .as_str()
                    .ok_or_else(|| String::from("params.streams entries must be strings"))?;
                let normalized = stream.trim();
                if normalized.is_empty() {
                    return Err(String::from("params.streams entries must not be empty"));
                }
                parsed.push(normalized.to_owned());
            }
            if parsed.is_empty() {
                return Err(String::from("params.streams must not be empty"));
            }
            return Ok(parsed);
        }
    }

    Err(String::from("params.stream or params.streams is required"))
}

pub fn user_stream_topics(command: &TriggerStartCommand) -> Result<Vec<String>, String> {
    let topics = if let Some(topic) = command.params.get("topic").and_then(Value::as_str) {
        vec![topic.trim().to_owned()]
    } else if let Some(topics) = command.params.get("topics") {
        parse_topic_values(topics)?
    } else if let Some(stream) = command.params.get("stream").and_then(Value::as_str) {
        vec![stream.trim().to_owned()]
    } else if let Some(streams) = command.params.get("streams") {
        parse_topic_values(streams)?
    } else {
        vec![default_user_topic(command)?]
    };

    let mut normalized = Vec::new();
    for topic in topics {
        let topic = topic.trim();
        if topic.is_empty() {
            return Err(String::from("user stream topics must not be empty"));
        }
        normalized.push(topic.to_owned());
    }
    if normalized.is_empty() {
        return Err(String::from("user stream topics must not be empty"));
    }

    let has_all_in_one = normalized.iter().any(|topic| topic == "order");
    let has_categorized = normalized.iter().any(|topic| topic.starts_with("order."));
    if has_all_in_one && has_categorized {
        return Err(String::from(
            "cannot mix all-in-one topic `order` with categorized `order.{category}` topics",
        ));
    }

    Ok(normalized)
}

fn parse_topic_values(value: &Value) -> Result<Vec<String>, String> {
    if let Some(raw) = value.as_str() {
        let parsed = raw
            .split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>();
        if parsed.is_empty() {
            return Err(String::from("topic list must not be empty"));
        }
        return Ok(parsed);
    }

    if let Some(items) = value.as_array() {
        let mut parsed = Vec::new();
        for item in items {
            let topic = item
                .as_str()
                .ok_or_else(|| String::from("topic entries must be strings"))?;
            let topic = topic.trim();
            if topic.is_empty() {
                return Err(String::from("topic entries must not be empty"));
            }
            parsed.push(topic.to_owned());
        }
        if parsed.is_empty() {
            return Err(String::from("topic list must not be empty"));
        }
        return Ok(parsed);
    }

    Err(String::from("topics must be a string, comma-separated string, or string array"))
}

fn default_user_topic(command: &TriggerStartCommand) -> Result<String, String> {
    if let Some(category) = command.params.get("category").and_then(Value::as_str) {
        let category = category.trim().to_ascii_lowercase();
        if category.is_empty() {
            return Err(String::from("params.category must not be empty"));
        }
        return Ok(format!("order.{category}"));
    }

    if let Some(product_line) = command.params.get("product_line").and_then(Value::as_str) {
        let category = product_line.trim().to_ascii_lowercase();
        if category == "spot" || category == "linear" || category == "inverse" || category == "option" {
            return Ok(format!("order.{category}"));
        }
        return Err(format!("unsupported product_line {category}"));
    }

    Ok(String::from("order"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(params: Value) -> TriggerStartCommand {
        serde_json::from_value(json!({
            "type": "start",
            "protocol_version": "2.0.0",
            "trigger_id": "bybit-test",
            "source": "bybit_user_stream",
            "params": params,
            "heartbeat_interval_ms": 1000,
            "shutdown_grace_ms": 1000
        }))
        .expect("valid command")
    }

    #[test]
    fn user_topics_reject_mixed_all_in_one_and_categorized_topics() {
        let command = command(json!({ "topics": ["order", "order.spot"] }));
        let error = user_stream_topics(&command).expect_err("mixed topics should fail");
        assert!(error.contains("cannot mix"));
    }
}
