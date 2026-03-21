//! [INPUT]
//! Shared plugin manifests, external-node protocol payloads, and contract error helpers.
//!
//! [OUTPUT]
//! Defines validated plugin manifest and external-node protocol contract types for ChainBot.
//!
//! [ROLE]
//! Owns the shared plugin contract surface used by config loading, trigger orchestration, builtin external-node dispatch, and tests.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::errors::{assert_required_major, assert_supported_major, ContractError};

pub const CURRENT_API_MAJOR: u64 = 2;
pub const NODE_PLUGIN_CONTRACT_VERSION: &str = "1.0.0";
pub const NODE_PLUGIN_CONTRACT_MAX_MAJOR: u64 = 1;

pub const PLUGIN_KIND_BUILTIN: &str = "builtin";
pub const PLUGIN_KIND_EXTERNAL_NODE: &str = "external_node";
pub const PLUGIN_KIND_EXTERNAL_TRIGGER: &str = "external_trigger";
pub const NODE_PLUGIN_EXECUTE_CAPABILITY: &str = "node:execute";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginKind {
    Builtin,
    ExternalNode,
    ExternalTrigger,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginManifest {
    #[serde(rename = "manifest_version")]
    pub api_version: String,
    pub plugin_id: String,
    pub kind: String,
    pub entrypoint: String,
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub executable: Option<String>,
    #[serde(default)]
    pub input_schema: Vec<String>,
    #[serde(default)]
    pub output_schema: Vec<String>,
    #[serde(skip)]
    pub manifest_path: PathBuf,
}

impl PluginManifest {
    pub fn validate(&self) -> Result<(), ContractError> {
        assert_required_major(
            "plugin.manifest_version",
            &self.api_version,
            CURRENT_API_MAJOR,
        )?;

        validate_non_empty(&self.plugin_id, "plugin.plugin_id", &self.plugin_id)?;
        validate_non_empty(&self.entrypoint, "plugin.entrypoint", &self.plugin_id)?;

        validate_unique_non_empty_list(&self.capabilities, "plugin.capabilities", &self.plugin_id)?;

        match self.kind()? {
            PluginKind::Builtin => {}
            PluginKind::ExternalNode => {
                validate_non_empty(
                    self.executable.as_deref().unwrap_or_default(),
                    "plugin.executable",
                    &self.plugin_id,
                )?;
                validate_unique_non_empty_list(
                    &self.input_schema,
                    "plugin.input_schema",
                    &self.plugin_id,
                )?;
                validate_unique_non_empty_list(
                    &self.output_schema,
                    "plugin.output_schema",
                    &self.plugin_id,
                )?;

                if !self
                    .capabilities
                    .iter()
                    .any(|capability| capability == NODE_PLUGIN_EXECUTE_CAPABILITY)
                {
                    return Err(ContractError::NodePluginCapabilityNotDeclared {
                        plugin_id: self.plugin_id.clone(),
                        capability: NODE_PLUGIN_EXECUTE_CAPABILITY.to_owned(),
                    });
                }
            }
            PluginKind::ExternalTrigger => {
                validate_non_empty(
                    self.executable.as_deref().unwrap_or_default(),
                    "plugin.executable",
                    &self.plugin_id,
                )?;
            }
        }

        Ok(())
    }

    pub fn kind(&self) -> Result<PluginKind, ContractError> {
        match self.kind.as_str() {
            PLUGIN_KIND_BUILTIN => Ok(PluginKind::Builtin),
            PLUGIN_KIND_EXTERNAL_NODE => Ok(PluginKind::ExternalNode),
            PLUGIN_KIND_EXTERNAL_TRIGGER => Ok(PluginKind::ExternalTrigger),
            _ => Err(ContractError::NodePluginInvalidKind {
                plugin_id: self.plugin_id.clone(),
                kind: self.kind.clone(),
            }),
        }
    }

    pub fn from_json_str(input: &str) -> Result<Self, ContractError> {
        let manifest: Self = serde_json::from_str(input)?;
        manifest.validate()?;
        Ok(manifest)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExternalNodePluginRequest {
    pub contract_version: String,
    pub plugin_id: String,
    pub node_id: String,
    pub operation: String,
    #[serde(default)]
    pub requested_capabilities: Vec<String>,
    #[serde(default)]
    pub input: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExternalNodePluginResponse {
    pub contract_version: String,
    pub success: bool,
    #[serde(default)]
    pub output: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub error: Option<String>,
}

impl ExternalNodePluginRequest {
    pub fn validate(&self, manifest: &PluginManifest) -> Result<(), ContractError> {
        assert_supported_major(
            "node_plugin_request.contract_version",
            &self.contract_version,
            NODE_PLUGIN_CONTRACT_MAX_MAJOR,
        )?;

        if self.plugin_id != manifest.plugin_id {
            return Err(ContractError::NodePluginProtocolContractViolation {
                plugin_id: manifest.plugin_id.clone(),
                detail: "request plugin_id does not match manifest plugin_id".to_owned(),
            });
        }

        validate_non_empty(
            &self.node_id,
            "node_plugin_request.node_id",
            &self.plugin_id,
        )?;
        validate_non_empty(
            &self.operation,
            "node_plugin_request.operation",
            &self.plugin_id,
        )?;

        for capability in &self.requested_capabilities {
            if !manifest
                .capabilities
                .iter()
                .any(|declared| declared == capability)
            {
                return Err(ContractError::NodePluginCapabilityNotDeclared {
                    plugin_id: manifest.plugin_id.clone(),
                    capability: capability.clone(),
                });
            }
        }

        validate_input_schema(&manifest.plugin_id, &manifest.input_schema, &self.input)
    }
}

impl ExternalNodePluginResponse {
    pub fn validate(&self, manifest: &PluginManifest) -> Result<(), ContractError> {
        assert_supported_major(
            "node_plugin_response.contract_version",
            &self.contract_version,
            NODE_PLUGIN_CONTRACT_MAX_MAJOR,
        )?;
        if self.success {
            Ok(())
        } else {
            validate_non_empty(
                self.error.as_deref().unwrap_or_default(),
                "node_plugin_response.error",
                &manifest.plugin_id,
            )
        }
    }
}

pub(crate) fn validate_output_schema(
    plugin_id: &str,
    output_schema: &[String],
    output: &BTreeMap<String, serde_json::Value>,
) -> Result<(), ContractError> {
    let allowed: BTreeSet<&str> = output_schema.iter().map(String::as_str).collect();
    for key in output.keys() {
        if !allowed.contains(key.as_str()) {
            return Err(ContractError::NodePluginOutputSchemaMismatch {
                plugin_id: plugin_id.to_owned(),
                detail: format!("output key {key} is not declared in manifest output_schema"),
            });
        }
    }

    for required in output_schema {
        if !output.contains_key(required) {
            return Err(ContractError::NodePluginOutputSchemaMismatch {
                plugin_id: plugin_id.to_owned(),
                detail: format!("required output key {required} is missing"),
            });
        }
    }

    Ok(())
}

fn validate_non_empty(
    value: &str,
    field: &'static str,
    plugin_id: &str,
) -> Result<(), ContractError> {
    if value.trim().is_empty() {
        return Err(ContractError::NodePluginInvalidField {
            plugin_id: plugin_id.to_owned(),
            field,
            detail: "value cannot be empty".to_owned(),
        });
    }
    Ok(())
}

fn validate_unique_non_empty_list(
    values: &[String],
    field: &'static str,
    plugin_id: &str,
) -> Result<(), ContractError> {
    let mut seen = BTreeSet::new();
    for value in values {
        validate_non_empty(value, field, plugin_id)?;
        if !seen.insert(value.clone()) {
            return Err(ContractError::NodePluginInvalidField {
                plugin_id: plugin_id.to_owned(),
                field,
                detail: format!("duplicated value: {value}"),
            });
        }
    }
    Ok(())
}

fn validate_input_schema(
    plugin_id: &str,
    input_schema: &[String],
    input: &BTreeMap<String, serde_json::Value>,
) -> Result<(), ContractError> {
    let allowed: BTreeSet<&str> = input_schema.iter().map(String::as_str).collect();
    for key in input.keys() {
        if !allowed.contains(key.as_str()) {
            return Err(ContractError::NodePluginInputSchemaMismatch {
                plugin_id: plugin_id.to_owned(),
                detail: format!("input key {key} is not declared in manifest input_schema"),
            });
        }
    }

    for required in input_schema {
        if !input.contains_key(required) {
            return Err(ContractError::NodePluginInputSchemaMismatch {
                plugin_id: plugin_id.to_owned(),
                detail: format!("required input key {required} is missing"),
            });
        }
    }

    Ok(())
}
