//! [INPUT]
//! Scheduled node state snapshots and runtime variable namespaces captured during workflow execution.
//!
//! [OUTPUT]
//! Defines immutable workflow run report types describing status, node outputs, failures, and schedule waves.
//!
//! [ROLE]
//! Models the domain-level reporting surface for completed or failed workflow runs.

use std::collections::BTreeMap;

use super::{RuntimeVariableNamespaces, ScheduledNodeState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowRunStatus {
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowRunReport {
    pub run_id: String,
    pub workflow_id: String,
    pub status: WorkflowRunStatus,
    pub node_states: BTreeMap<String, ScheduledNodeState>,
    pub node_outputs: BTreeMap<String, BTreeMap<String, serde_json::Value>>,
    pub node_failures: BTreeMap<String, String>,
    pub runtime_namespaces: RuntimeVariableNamespaces,
    pub schedule_waves: Vec<Vec<String>>,
}
