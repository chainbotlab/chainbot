/*
[INPUT]:  Plugin manifest definitions, node-plugin invocation payloads, and plugin host root path.
[OUTPUT]: Validated manifest contracts plus external-node plugin execution results with typed failures.
[POS]:    Plugin boundary module for V2 manifest compatibility and safe external-node host execution.
[UPDATE]: 2026-03-16 - Add versioned plugin manifest contract and parser.
[UPDATE]: 2026-03-16 - Add external node plugin host, manifest guards, and execution contract validation.
[UPDATE]: 2026-03-17 - Apply default-deny process environment with explicit allowlist for external plugin hosts.
*/

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};

use crate::errors::{assert_supported_major, ContractError};

pub const CURRENT_API_MAJOR: u64 = 1;
pub const NODE_PLUGIN_CONTRACT_VERSION: &str = "1.0.0";
pub const NODE_PLUGIN_CONTRACT_MAX_MAJOR: u64 = 1;
pub const NODE_PLUGIN_EXECUTE_CAPABILITY: &str = "node:execute";

pub const PLUGIN_KIND_BUILTIN: &str = "builtin";
pub const PLUGIN_KIND_EXTERNAL_NODE: &str = "external_node";
pub const PLUGIN_KIND_EXTERNAL_TRIGGER: &str = "external_trigger";
pub const PLUGIN_KIND_NODE_ALIAS: &str = "node";
pub const PLUGIN_KIND_TRIGGER_ALIAS: &str = "trigger";
pub const PLUGIN_HOST_ENV_ALLOWLIST: &[&str] =
    &["PATH", "SYSTEMROOT", "WINDIR", "COMSPEC", "PATHEXT"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginKind {
    Builtin,
    ExternalNode,
    ExternalTrigger,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginManifest {
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
}

impl PluginManifest {
    pub fn validate(&self) -> Result<(), ContractError> {
        assert_supported_major("plugin.api_version", &self.api_version, CURRENT_API_MAJOR)?;

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
            PLUGIN_KIND_EXTERNAL_NODE | PLUGIN_KIND_NODE_ALIAS => Ok(PluginKind::ExternalNode),
            PLUGIN_KIND_EXTERNAL_TRIGGER | PLUGIN_KIND_TRIGGER_ALIAS => {
                Ok(PluginKind::ExternalTrigger)
            }
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

#[derive(Debug, Clone, PartialEq)]
pub struct NodePluginExecutionResult {
    pub output: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalNodePluginHost {
    plugins_root: PathBuf,
}

impl ExternalNodePluginHost {
    pub fn new(plugins_root: PathBuf) -> Self {
        Self { plugins_root }
    }

    pub fn plugins_root(&self) -> &Path {
        &self.plugins_root
    }

    pub fn execute(
        &self,
        manifest: &PluginManifest,
        request: &ExternalNodePluginRequest,
    ) -> Result<NodePluginExecutionResult, ContractError> {
        manifest.validate()?;
        request.validate(manifest)?;

        let executable = self.resolve_executable_path(manifest)?;
        let request_json = serde_json::to_vec(request).map_err(|source| {
            ContractError::NodePluginProtocolEncode {
                plugin_id: manifest.plugin_id.clone(),
                source,
            }
        })?;

        let mut command = Command::new(&executable);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        configure_plugin_host_environment(&mut command);

        let mut child = command
            .spawn()
            .map_err(|source| ContractError::NodePluginSpawnFailed {
                plugin_id: manifest.plugin_id.clone(),
                executable: executable.clone(),
                source,
            })?;

        if let Some(stdin) = child.stdin.as_mut() {
            use std::io::Write;
            stdin.write_all(&request_json).map_err(|source| {
                ContractError::NodePluginProcessIo {
                    plugin_id: manifest.plugin_id.clone(),
                    operation: "write request to plugin stdin",
                    source,
                }
            })?;
        }

        let output =
            child
                .wait_with_output()
                .map_err(|source| ContractError::NodePluginProcessIo {
                    plugin_id: manifest.plugin_id.clone(),
                    operation: "wait for plugin process output",
                    source,
                })?;

        if !output.status.success() {
            return Err(ContractError::NodePluginProcessFailed {
                plugin_id: manifest.plugin_id.clone(),
                exit_code: output.status.code(),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            });
        }

        let response: ExternalNodePluginResponse =
            serde_json::from_slice(&output.stdout).map_err(|source| {
                ContractError::NodePluginProtocolDecode {
                    plugin_id: manifest.plugin_id.clone(),
                    source,
                }
            })?;

        response.validate(manifest)?;
        if !response.success {
            return Err(ContractError::NodePluginReturnedFailure {
                plugin_id: manifest.plugin_id.clone(),
                message: response.error.unwrap_or_else(|| {
                    "plugin returned success=false without error detail".to_owned()
                }),
            });
        }

        validate_output_schema(
            &manifest.plugin_id,
            &manifest.output_schema,
            &response.output,
        )?;

        Ok(NodePluginExecutionResult {
            output: response.output,
        })
    }

    fn resolve_executable_path(&self, manifest: &PluginManifest) -> Result<PathBuf, ContractError> {
        if manifest.kind()? != PluginKind::ExternalNode {
            return Err(ContractError::NodePluginInvalidKind {
                plugin_id: manifest.plugin_id.clone(),
                kind: manifest.kind.clone(),
            });
        }

        let executable = manifest.executable.as_deref().ok_or_else(|| {
            ContractError::NodePluginMissingExecutable {
                plugin_id: manifest.plugin_id.clone(),
            }
        })?;
        let executable_path = Path::new(executable);

        if executable_path.is_absolute() {
            return Err(ContractError::NodePluginInvalidExecutablePath {
                plugin_id: manifest.plugin_id.clone(),
                executable: executable.to_owned(),
                detail: "absolute executable paths are not allowed".to_owned(),
            });
        }

        let candidate = self.plugins_root.join(executable_path);
        let canonical_root =
            fs::canonicalize(&self.plugins_root).map_err(|source| ContractError::Io {
                path: self.plugins_root.clone(),
                operation: "canonicalize plugins root",
                source,
            })?;
        let canonical_candidate = fs::canonicalize(&candidate).map_err(|source| {
            ContractError::NodePluginInvalidExecutablePath {
                plugin_id: manifest.plugin_id.clone(),
                executable: executable.to_owned(),
                detail: source.to_string(),
            }
        })?;

        if !canonical_candidate.starts_with(&canonical_root) {
            return Err(ContractError::NodePluginInvalidExecutablePath {
                plugin_id: manifest.plugin_id.clone(),
                executable: executable.to_owned(),
                detail: "executable path escapes plugins root".to_owned(),
            });
        }

        let metadata = fs::metadata(&canonical_candidate).map_err(|source| ContractError::Io {
            path: canonical_candidate.clone(),
            operation: "inspect plugin executable metadata",
            source,
        })?;
        if !metadata.is_file() {
            return Err(ContractError::NodePluginInvalidExecutablePath {
                plugin_id: manifest.plugin_id.clone(),
                executable: executable.to_owned(),
                detail: "resolved executable path is not a file".to_owned(),
            });
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o111 == 0 {
                return Err(ContractError::NodePluginInvalidExecutablePath {
                    plugin_id: manifest.plugin_id.clone(),
                    executable: executable.to_owned(),
                    detail: "resolved executable path is not executable".to_owned(),
                });
            }
        }

        Ok(canonical_candidate)
    }
}

pub(crate) fn configure_plugin_host_environment(command: &mut Command) {
    command.env_clear();
    for key in PLUGIN_HOST_ENV_ALLOWLIST {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
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

fn validate_output_schema(
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
