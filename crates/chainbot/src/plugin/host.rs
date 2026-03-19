//! [INPUT]
//! External-node plugin manifests, protocol payloads, filesystem roots, and subprocess execution policy.
//!
//! [OUTPUT]
//! Executes external node plugins under executable-path and host-environment guards.
//!
//! [ROLE]
//! Owns the external node-plugin host runtime boundary.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::errors::ContractError;

use super::contract::{
    validate_output_schema, ExternalNodePluginRequest, ExternalNodePluginResponse, PluginKind,
    PluginManifest,
};

pub(crate) const PLUGIN_HOST_ENV_ALLOWLIST: &[&str] =
    &["PATH", "SYSTEMROOT", "WINDIR", "COMSPEC", "PATHEXT"];

#[derive(Debug, Clone, PartialEq)]
pub struct NodePluginExecutionResult {
    pub output: std::collections::BTreeMap<String, serde_json::Value>,
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

        let manifest_root = self
            .manifest_root(manifest)
            .unwrap_or_else(|| self.plugins_root.clone());
        let candidate = manifest_root.join(executable_path);
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

    fn manifest_root(&self, manifest: &PluginManifest) -> Option<PathBuf> {
        manifest
            .manifest_path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .map(Path::to_path_buf)
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
