//! [INPUT]
//! Persisted run and trigger record summaries plus schema-version validation helpers.
//!
//! [OUTPUT]
//! Defines runtime-state enums and validation routines for persisted run, trigger, and checkpoint records.
//!
//! [ROLE]
//! Holds schema-level domain validation for ChainBot runtime-state records.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::errors::{assert_supported_major, ContractError};

use super::{RunRecordSummary, TriggerEventRecord, TriggerSnapshotRecord, TriggerTokenSnapshot};

pub const CURRENT_SCHEMA_MAJOR: u64 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
}

impl RunRecordSummary {
    pub fn validate(&self) -> Result<(), ContractError> {
        assert_supported_major(
            "run_record_summary.schema_version",
            &self.schema_version,
            CURRENT_SCHEMA_MAJOR,
        )
    }
}

impl TriggerSnapshotRecord {
    pub fn new(trigger_id: impl Into<String>) -> Self {
        Self {
            schema_version: default_schema_version(),
            trigger_id: trigger_id.into(),
            last_event_id: None,
            last_accepted_at_ms: None,
            last_sequence: 0,
            accepted_event_ids: BTreeSet::new(),
            dedup_tokens: Vec::new(),
            cooldown_tokens: Vec::new(),
        }
    }

    pub fn apply_record(&mut self, record: &TriggerEventRecord) {
        self.last_event_id = Some(record.event_id.clone());
        self.last_accepted_at_ms = Some(record.accepted_at_ms);
        self.last_sequence = self.last_sequence.max(record.sequence);
        self.accepted_event_ids.insert(record.event_id.clone());
        retain_unexpired_tokens(&mut self.dedup_tokens, record.accepted_at_ms);
        retain_unexpired_tokens(&mut self.cooldown_tokens, record.accepted_at_ms);
        upsert_token_snapshot(
            &mut self.dedup_tokens,
            record.dedup_key.as_deref(),
            record.dedup_expires_at_ms,
            record.accepted_at_ms,
        );
        upsert_token_snapshot(
            &mut self.cooldown_tokens,
            record.cooldown_key.as_deref(),
            record.cooldown_expires_at_ms,
            record.accepted_at_ms,
        );
    }
}

pub fn default_schema_version() -> String {
    String::from("1.0.0")
}

pub fn accepted_trigger_key(trigger_id: &str, event_id: &str) -> String {
    format!("{trigger_id}:::{event_id}")
}

pub fn sanitize_path_component(value: &str) -> String {
    let mut sanitized = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
            sanitized.push(ch);
        } else {
            sanitized.push('_');
        }
    }

    if sanitized.is_empty() {
        sanitized.push('_');
    }

    sanitized
}

pub fn retain_unexpired_tokens(tokens: &mut Vec<TriggerTokenSnapshot>, now_ms: i64) {
    tokens.retain(|token| token.expires_at_ms > now_ms);
}

pub fn upsert_token_snapshot(
    tokens: &mut Vec<TriggerTokenSnapshot>,
    key: Option<&str>,
    expires_at_ms: Option<i64>,
    now_ms: i64,
) {
    let (Some(key), Some(expires_at_ms)) = (key, expires_at_ms) else {
        return;
    };
    if expires_at_ms <= now_ms {
        return;
    }

    if let Some(existing) = tokens.iter_mut().find(|token| token.key == key) {
        existing.expires_at_ms = existing.expires_at_ms.max(expires_at_ms);
        return;
    }

    tokens.push(TriggerTokenSnapshot {
        key: key.to_owned(),
        expires_at_ms,
    });
}
