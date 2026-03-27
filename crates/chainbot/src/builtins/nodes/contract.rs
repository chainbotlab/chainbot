//! [INPUT]
//! Workflow runtime variable namespaces, builtin node request metadata, and contract error handling.
//!
//! [OUTPUT]
//! Defines builtin node handler traits plus request and result contracts used by registry-backed dispatch.
//!
//! [ROLE]
//! Provides the canonical contract surface for builtin node execution.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::domain::workflow::RuntimeVariableNamespaces;
use crate::errors::ContractError;

pub trait BuiltinNodeHandler: Send + Sync {
    fn kind(&self) -> &str;

    fn handle(&self, request: &BuiltinNodeRequest) -> Result<BuiltinNodeResult, ContractError>;
}

#[derive(Debug, Clone, PartialEq)]
pub struct BuiltinNodeRequest {
    pub run_id: String,
    pub workflow_id: String,
    pub workflow_package_root: PathBuf,
    pub node_id: String,
    pub operation: String,
    pub inputs: BTreeMap<String, serde_json::Value>,
    pub runtime_namespaces: RuntimeVariableNamespaces,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct BuiltinNodeResult {
    pub outputs: BTreeMap<String, serde_json::Value>,
    pub run_scoped: BTreeMap<String, serde_json::Value>,
    pub subflow_output: BTreeMap<String, serde_json::Value>,
}
