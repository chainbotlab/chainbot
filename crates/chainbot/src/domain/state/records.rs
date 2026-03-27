//! [INPUT]
//! Serde-backed runtime-state record types and shared run-status semantics.
//!
//! [OUTPUT]
//! Defines persisted record structures for runs, workflow logs, trigger events, snapshots, checkpoints, and ingress inbox rows.
//!
//! [ROLE]
//! Provides the domain record catalog consumed by runtime-state storage adapters.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::RunStatus;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunRecordSummary {
    pub schema_version: String,
    pub run_id: String,
    pub workflow_id: String,
    pub status: RunStatus,
    pub started_at_ms: i64,
    pub finished_at_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowRuntimeLogEntry {
    pub run_id: String,
    pub sequence: u64,
    pub event: String,
    pub message: String,
    pub occurred_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerEventRecord {
    #[serde(default = "super::default_schema_version")]
    pub schema_version: String,
    pub run_id: String,
    pub sequence: u64,
    pub trigger_id: String,
    #[serde(default)]
    pub workflow_id: String,
    pub event_id: String,
    #[serde(default)]
    pub checkpoint: Option<String>,
    pub source: String,
    pub accepted_at_ms: i64,
    #[serde(default)]
    pub payload: serde_json::Value,
    #[serde(default)]
    pub dedup_key: Option<String>,
    #[serde(default)]
    pub dedup_expires_at_ms: Option<i64>,
    #[serde(default)]
    pub cooldown_key: Option<String>,
    #[serde(default)]
    pub cooldown_expires_at_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IngressInboxRecord {
    #[serde(default = "super::default_schema_version")]
    pub schema_version: String,
    pub inbox_id: String,
    pub trigger_id: String,
    pub workflow_id: String,
    pub transport_kind: String,
    pub ingress_event_id: String,
    pub source: String,
    pub route_path: String,
    #[serde(default)]
    pub http_method: Option<String>,
    pub received_at_ms: i64,
    #[serde(default)]
    pub payload: serde_json::Value,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub remote_addr: Option<String>,
    #[serde(default)]
    pub processed_at_ms: Option<i64>,
    #[serde(default)]
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StagedTriggerEventRecord {
    #[serde(default = "super::default_schema_version")]
    pub schema_version: String,
    pub staging_id: String,
    pub trigger_id: String,
    pub workflow_id: String,
    pub event_id: String,
    pub source: String,
    pub occurred_at_ms: i64,
    pub staged_at_ms: i64,
    #[serde(default)]
    pub checkpoint: Option<String>,
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
    #[serde(default)]
    pub accepted_at_ms: Option<i64>,
    #[serde(default)]
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerCheckpointRecord {
    #[serde(default = "super::default_schema_version")]
    pub schema_version: String,
    pub trigger_id: String,
    pub checkpoint: String,
    pub acked_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerTokenSnapshot {
    pub key: String,
    pub expires_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerSnapshotRecord {
    #[serde(default = "super::default_schema_version")]
    pub schema_version: String,
    pub trigger_id: String,
    #[serde(default)]
    pub last_event_id: Option<String>,
    #[serde(default)]
    pub last_accepted_at_ms: Option<i64>,
    #[serde(default)]
    pub last_sequence: u64,
    #[serde(default)]
    pub accepted_event_ids: BTreeSet<String>,
    #[serde(default)]
    pub dedup_tokens: Vec<TriggerTokenSnapshot>,
    #[serde(default)]
    pub cooldown_tokens: Vec<TriggerTokenSnapshot>,
}
