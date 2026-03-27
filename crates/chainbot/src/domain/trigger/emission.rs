//! [INPUT]
//! Trigger definitions and staged trigger-event records produced by ingress or external trigger runtimes.
//!
//! [OUTPUT]
//! Defines normalized `TriggerEmission` values and helpers for rebuilding emissions from persisted staged records.
//!
//! [ROLE]
//! Models the domain event payload shape consumed by trigger acceptance logic.

use serde::{Deserialize, Serialize};

use crate::domain::state::StagedTriggerEventRecord;

use super::contract::TriggerDefinition;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TriggerEmission {
    pub event_id: String,
    pub occurred_at_ms: i64,
    #[serde(default)]
    pub checkpoint: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub payload: serde_json::Value,
    #[serde(default)]
    pub dedup_key: Option<String>,
    #[serde(default)]
    pub dedup_window_ms: Option<i64>,
    #[serde(default)]
    pub cooldown_key: Option<String>,
    #[serde(default)]
    pub cooldown_ms: Option<i64>,
}

pub(crate) fn trigger_emission_from_staged_record(
    record: &StagedTriggerEventRecord,
) -> TriggerEmission {
    TriggerEmission {
        event_id: record.event_id.clone(),
        occurred_at_ms: record.occurred_at_ms,
        checkpoint: record.checkpoint.clone(),
        source: Some(record.source.clone()),
        payload: record.payload.clone(),
        dedup_key: record.dedup_key.clone(),
        dedup_window_ms: record.dedup_window_ms,
        cooldown_key: record.cooldown_key.clone(),
        cooldown_ms: record.cooldown_ms,
    }
}

pub(crate) fn map_trigger_payload(
    definition: &TriggerDefinition,
    payload: &serde_json::Value,
) -> serde_json::Value {
    if definition.input_mapping.is_empty() {
        return payload.clone();
    }

    let mut mapped = serde_json::Map::new();
    for (target, selector) in &definition.input_mapping {
        if let Some(value) = select_payload_value(payload, selector) {
            mapped.insert(target.clone(), value.clone());
        }
    }
    serde_json::Value::Object(mapped)
}

pub(crate) fn select_payload_value<'a>(
    payload: &'a serde_json::Value,
    selector: &str,
) -> Option<&'a serde_json::Value> {
    if selector == "payload" {
        return Some(payload);
    }
    let remainder = selector.strip_prefix("payload.")?;

    let mut current = payload;
    for segment in remainder.split('.') {
        current = current.get(segment)?;
    }
    Some(current)
}
