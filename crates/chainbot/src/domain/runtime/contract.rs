//! [INPUT]
//! Runtime scheduling concepts, workflow variable contracts, subflow wiring, and manifest-version validation helpers.
//!
//! [OUTPUT]
//! Defines node and schedule contracts that describe how workflows execute at runtime.
//!
//! [ROLE]
//! Provides backend-agnostic runtime contract types shared across workflow validation and execution planning.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::errors::{assert_required_major, ContractError};
pub use crate::domain::workflow::{
    DependsMode, RuntimeVariableLayers, RuntimeVariableNamespace, RuntimeVariableNamespaces,
    RuntimeVariableSource, SubflowContract, SubflowExport, SubflowImport, VariableBinding,
    VariableReference, WhenCondition, WhenOperator, WorkflowDefinition,
};

pub const CURRENT_API_MAJOR: u64 = 2;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeDefinition {
    #[serde(rename = "manifest_version")]
    pub api_version: String,
    #[serde(rename = "id")]
    pub node_id: String,
    pub kind: String,
    #[serde(rename = "plugin")]
    pub plugin_id: String,
    pub operation: String,
    #[serde(default)]
    pub depends_mode: DependsMode,
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub inputs: Vec<VariableBinding>,
    #[serde(default)]
    pub when: Option<WhenCondition>,
    #[serde(default)]
    pub subflow: Option<SubflowContract>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NormalizedRunRequest {
    pub run_id: String,
    pub workflow_id: String,
    pub cli_args: BTreeMap<String, serde_json::Value>,
    pub manual_invocation_input: BTreeMap<String, serde_json::Value>,
    pub trigger_payload_mapping: BTreeMap<String, serde_json::Value>,
    pub subflow_input: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduledNodeState {
    Pending,
    Blocked,
    Ready,
    Running,
    Succeeded,
    Skipped,
    Failed,
}

impl NodeDefinition {
    pub fn validate(&self) -> Result<(), ContractError> {
        assert_required_major(
            "node.manifest_version",
            &self.api_version,
            CURRENT_API_MAJOR,
        )?;

        for input in &self.inputs {
            if !input.validate() {
                return Err(ContractError::InvalidVariableReference {
                    workflow_id: "<unknown>".to_owned(),
                    node_id: self.node_id.clone(),
                    context: "node.inputs",
                    namespace: input.source.namespace.as_str().to_owned(),
                    key: input.source.key.clone(),
                });
            }
        }

        if let Some(when) = &self.when
            && !when.validate()
        {
            return Err(ContractError::InvalidVariableReference {
                workflow_id: "<unknown>".to_owned(),
                node_id: self.node_id.clone(),
                context: "node.when",
                namespace: when.source.namespace.as_str().to_owned(),
                key: when.source.key.clone(),
            });
        }

        Ok(())
    }
}

impl NormalizedRunRequest {
    pub fn new(run_id: impl Into<String>, workflow_id: impl Into<String>) -> Self {
        Self {
            run_id: run_id.into(),
            workflow_id: workflow_id.into(),
            cli_args: BTreeMap::new(),
            manual_invocation_input: BTreeMap::new(),
            trigger_payload_mapping: BTreeMap::new(),
            subflow_input: BTreeMap::new(),
        }
    }
}

impl ScheduledNodeState {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Skipped | Self::Failed)
    }
}
