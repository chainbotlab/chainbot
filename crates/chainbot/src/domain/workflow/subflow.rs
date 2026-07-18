//! [INPUT]
//! Runtime variable namespaces, variable references, and contract errors for subflow linkage validation.
//!
//! [OUTPUT]
//! Defines subflow import or export contracts plus validation helpers for parent-child workflow boundaries.
//!
//! [ROLE]
//! Models domain-level subflow data movement between parent and child workflows.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::errors::ContractError;

use super::variables::{RuntimeVariableNamespace, RuntimeVariableNamespaces, VariableReference};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubflowImport {
    pub child_key: String,
    pub source: VariableReference,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubflowExport {
    pub child_key: String,
    pub parent_key: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubflowContract {
    pub workflow_id: String,
    #[serde(default)]
    pub imports: Vec<SubflowImport>,
    #[serde(default)]
    pub exports: Vec<SubflowExport>,
}

impl SubflowContract {
    pub fn validate(
        &self,
        workflow_id: &str,
        node_id: &str,
        is_subflow_node: bool,
    ) -> Result<(), ContractError> {
        if !is_subflow_node {
            return Err(ContractError::UnexpectedSubflowContract {
                workflow_id: workflow_id.to_owned(),
                node_id: node_id.to_owned(),
            });
        }

        if self.workflow_id.trim().is_empty() {
            return Err(ContractError::InvalidSubflowContract {
                workflow_id: workflow_id.to_owned(),
                node_id: node_id.to_owned(),
                detail: "subflow workflow_id cannot be empty".to_owned(),
            });
        }

        let mut child_import_keys = BTreeSet::new();
        for import in &self.imports {
            if import.child_key.trim().is_empty() {
                return Err(ContractError::InvalidSubflowContract {
                    workflow_id: workflow_id.to_owned(),
                    node_id: node_id.to_owned(),
                    detail: "subflow import child_key cannot be empty".to_owned(),
                });
            }

            if !import.source.validate() {
                return Err(ContractError::InvalidVariableReference {
                    workflow_id: workflow_id.to_owned(),
                    node_id: node_id.to_owned(),
                    context: "subflow.imports",
                    namespace: import.source.namespace.as_str().to_owned(),
                    key: import.source.key.clone(),
                });
            }

            match import.source.namespace {
                RuntimeVariableNamespace::SubflowInput
                | RuntimeVariableNamespace::SubflowOutput => {
                    return Err(ContractError::InvalidVariableReference {
                        workflow_id: workflow_id.to_owned(),
                        node_id: node_id.to_owned(),
                        context: "subflow.imports",
                        namespace: import.source.namespace.as_str().to_owned(),
                        key: import.source.key.clone(),
                    });
                }
                RuntimeVariableNamespace::CliArgs
                | RuntimeVariableNamespace::ManualInvocationInput
                | RuntimeVariableNamespace::TriggerPayloadMapping
                | RuntimeVariableNamespace::WorkflowDefaults
                | RuntimeVariableNamespace::ConfigDefaults
                | RuntimeVariableNamespace::NodeOutputs
                | RuntimeVariableNamespace::RunScoped => {}
            }

            if !child_import_keys.insert(import.child_key.clone()) {
                return Err(ContractError::InvalidSubflowContract {
                    workflow_id: workflow_id.to_owned(),
                    node_id: node_id.to_owned(),
                    detail: format!(
                        "subflow import child_key {} is duplicated",
                        import.child_key
                    ),
                });
            }
        }

        let mut parent_export_keys = BTreeSet::new();
        for export in &self.exports {
            if export.child_key.trim().is_empty() || export.parent_key.trim().is_empty() {
                return Err(ContractError::InvalidSubflowContract {
                    workflow_id: workflow_id.to_owned(),
                    node_id: node_id.to_owned(),
                    detail: "subflow exports cannot contain empty child_key or parent_key"
                        .to_owned(),
                });
            }

            if !parent_export_keys.insert(export.parent_key.clone()) {
                return Err(ContractError::InvalidSubflowContract {
                    workflow_id: workflow_id.to_owned(),
                    node_id: node_id.to_owned(),
                    detail: format!(
                        "subflow export parent_key {} is duplicated",
                        export.parent_key
                    ),
                });
            }
        }

        Ok(())
    }

    pub fn try_build_child_inputs(
        &self,
        namespaces: &RuntimeVariableNamespaces,
    ) -> Result<BTreeMap<String, serde_json::Value>, ContractError> {
        let mut child_inputs = BTreeMap::new();
        for import in &self.imports {
            if let Some(value) = namespaces.try_resolve(&import.source)? {
                child_inputs.insert(import.child_key.clone(), value.clone());
            }
        }
        Ok(child_inputs)
    }

    pub fn collect_exports(
        &self,
        child_outputs: &BTreeMap<String, serde_json::Value>,
    ) -> BTreeMap<String, serde_json::Value> {
        let mut exports = BTreeMap::new();
        for export in &self.exports {
            if let Some(value) = child_outputs.get(&export.child_key) {
                exports.insert(export.parent_key.clone(), value.clone());
            }
        }
        exports
    }
}
