//! [INPUT]
//! Plugin package config documents, source metadata blocks, and source index manifests.
//!
//! [OUTPUT]
//! Defines source-manifest contracts and CLI-facing read models for plugin source discovery and install.
//!
//! [ROLE]
//! Owns repository-local source metadata decoding and validation for plugin packages.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::errors::ContractError;
use crate::plugin::PluginManifest;

use super::fs::{
    ensure_safe_relative_path, resolve_within_root, resolve_within_root_allow_parents,
};

pub(crate) const SOURCE_MANIFEST_VERSION: &str = "1.0.0";
pub(crate) const SOURCE_INDEX_FILE_NAME: &str = "chainbot-plugin-index.toml";
pub(crate) const LEGACY_SOURCE_INSTALL_FILE_NAME: &str = "source.toml";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct PluginSourceDescriptor {
    pub source_kind: String,
    pub target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct PluginSourceListOutput {
    pub source: PluginSourceDescriptor,
    pub plugins: Vec<PluginSourceSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct PluginSourceSummary {
    pub plugin_id: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    pub plugin_kind: String,
    pub runtime: String,
    pub install_mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct PluginSourceShowOutput {
    pub source: PluginSourceDescriptor,
    pub plugin: PluginSourceDetail,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct PluginSourceDetail {
    pub plugin_id: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    pub plugin_kind: String,
    pub runtime: String,
    pub install_mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_version: Option<String>,
    pub entrypoint: String,
    pub capabilities: Vec<String>,
    pub entry_artifact: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub build: Option<PluginSourceBuildDetail>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct PluginSourceBuildDetail {
    pub kind: String,
    pub command: Vec<String>,
    pub workdir: String,
    pub outputs: Vec<PluginSourceBuildOutputDetail>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct PluginSourceBuildOutputDetail {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub(crate) struct SourceIndexManifest {
    pub manifest_version: String,
    #[serde(default)]
    pub plugins: Vec<SourceIndexEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub(crate) struct SourceIndexEntry {
    pub plugin_id: String,
    pub path: String,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub(crate) struct SourceInstallManifest {
    pub manifest_version: String,
    pub install_mode: SourceInstallMode,
    pub runtime: SourceRuntime,
    pub entry_artifact: String,
    #[serde(default)]
    pub release_version: Option<String>,
    #[serde(default)]
    pub build: Option<SourceBuildSpec>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct PluginSourceConfigEnvelope {
    #[serde(default)]
    source: Option<SourceInstallManifest>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SourceInstallMode {
    Direct,
    BuildRequired,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SourceRuntime {
    Python,
    Node,
    Bin,
    Wasm,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub(crate) struct SourceBuildSpec {
    pub kind: BuildKind,
    pub command: Vec<String>,
    pub workdir: String,
    #[serde(default)]
    pub outputs: Vec<SourceBuildOutput>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BuildKind {
    Cargo,
    Npm,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub(crate) struct SourceBuildOutput {
    pub from: String,
    pub to: String,
}

impl SourceIndexManifest {
    pub(crate) fn load(path: &Path) -> Result<Self, ContractError> {
        let contents = read_toml_document(path, "plugin source index")?;
        let manifest: Self = decode_toml_document(path, &contents, "plugin source index")?;
        manifest.validate(path)?;
        Ok(manifest)
    }

    pub(crate) fn validate(&self, _path: &Path) -> Result<(), ContractError> {
        if self.manifest_version != SOURCE_MANIFEST_VERSION {
            return Err(ContractError::CliUsage {
                message: format!(
                    "plugin source index manifest_version must be {}, got {}",
                    SOURCE_MANIFEST_VERSION, self.manifest_version
                ),
            });
        }
        if self.plugins.is_empty() {
            return Err(ContractError::CliUsage {
                message: "plugin source index must declare at least one plugin".to_owned(),
            });
        }
        let mut seen = std::collections::BTreeSet::new();
        for entry in &self.plugins {
            if entry.plugin_id.trim().is_empty() {
                return Err(ContractError::CliUsage {
                    message: "plugin source index plugin_id must not be empty".to_owned(),
                });
            }
            if !seen.insert(entry.plugin_id.clone()) {
                return Err(ContractError::CliUsage {
                    message: format!(
                        "duplicate plugin_id in plugin source index: {}",
                        entry.plugin_id
                    ),
                });
            }
            let _ = ensure_safe_relative_path(&entry.path, "chainbot-plugin-index.plugins.path")?;
            if entry.summary.trim().is_empty() {
                return Err(ContractError::CliUsage {
                    message: format!(
                        "plugin source index summary must not be empty for {}",
                        entry.plugin_id
                    ),
                });
            }
        }
        Ok(())
    }
}

impl SourceInstallManifest {
    pub(crate) fn validate(&self, _path: &Path) -> Result<(), ContractError> {
        if self.manifest_version != SOURCE_MANIFEST_VERSION {
            return Err(ContractError::CliUsage {
                message: format!(
                    "plugin source manifest_version must be {}, got {}",
                    SOURCE_MANIFEST_VERSION, self.manifest_version
                ),
            });
        }
        ensure_no_raw_server_reference(&self.entry_artifact, "source.entry_artifact")?;
        let _ = ensure_safe_relative_path(&self.entry_artifact, "source.entry_artifact")?;
        match self.install_mode {
            SourceInstallMode::Direct => {
                if self.build.is_some() {
                    return Err(ContractError::CliUsage {
                        message: "source.build is only allowed when install_mode=build_required"
                            .to_owned(),
                    });
                }
            }
            SourceInstallMode::BuildRequired => {
                let build = self.build.as_ref().ok_or_else(|| ContractError::CliUsage {
                    message: "source.build is required when install_mode=build_required".to_owned(),
                })?;
                if build.command.is_empty() {
                    return Err(ContractError::CliUsage {
                        message: "source.build.command must not be empty".to_owned(),
                    });
                }
                if build.workdir.trim().is_empty() {
                    return Err(ContractError::CliUsage {
                        message: "source.build.workdir must not be empty".to_owned(),
                    });
                }
                if build.outputs.is_empty() {
                    return Err(ContractError::CliUsage {
                        message: "source.build.outputs must contain at least one output".to_owned(),
                    });
                }
                for output in &build.outputs {
                    if output.from.trim().is_empty() || output.to.trim().is_empty() {
                        return Err(ContractError::CliUsage {
                            message: "source.build.outputs.from/to must not be empty".to_owned(),
                        });
                    }
                    let _ = ensure_safe_relative_path(&output.to, "source.build.outputs.to")?;
                }
            }
        }
        Ok(())
    }

    pub(crate) fn entry_artifact_path(
        &self,
        package_root: &Path,
    ) -> Result<PathBuf, ContractError> {
        let relative = ensure_safe_relative_path(&self.entry_artifact, "source.entry_artifact")?;
        resolve_within_root(package_root, &relative, "source.entry_artifact")
    }

    pub(crate) fn build_workdir_path(
        &self,
        package_root: &Path,
        repo_root: &Path,
    ) -> Result<Option<PathBuf>, ContractError> {
        let Some(build) = self.build.as_ref() else {
            return Ok(None);
        };
        let workdir = resolve_within_root_allow_parents(
            package_root,
            Path::new(&build.workdir),
            "source.build.workdir",
        )?;
        let canonical_repo_root =
            std::fs::canonicalize(repo_root).map_err(|source| ContractError::Io {
                path: repo_root.to_path_buf(),
                operation: "canonicalize repo root",
                source,
            })?;
        if !workdir.starts_with(&canonical_repo_root) {
            return Err(ContractError::CliUsage {
                message: format!(
                    "source.build.workdir escapes repo root: {}",
                    workdir.display()
                ),
            });
        }
        Ok(Some(workdir))
    }

    pub(crate) fn output_paths(
        &self,
        package_root: &Path,
        repo_root: &Path,
    ) -> Result<Vec<(PathBuf, PathBuf)>, ContractError> {
        let Some(build) = self.build.as_ref() else {
            return Ok(Vec::new());
        };
        let workdir = self
            .build_workdir_path(package_root, repo_root)?
            .ok_or_else(|| ContractError::CliUsage {
                message: "missing build workdir".to_owned(),
            })?;
        let mut outputs = Vec::with_capacity(build.outputs.len());
        for output in &build.outputs {
            let from = resolve_within_root_allow_parents(
                &workdir,
                Path::new(&output.from),
                "source.build.outputs.from",
            )?;
            let canonical_repo_root =
                std::fs::canonicalize(repo_root).map_err(|source| ContractError::Io {
                    path: repo_root.to_path_buf(),
                    operation: "canonicalize repo root",
                    source,
                })?;
            if !from.starts_with(&canonical_repo_root) {
                return Err(ContractError::CliUsage {
                    message: format!(
                        "source.build.outputs.from escapes repo root: {}",
                        from.display()
                    ),
                });
            }
            let to = resolve_within_root(
                package_root,
                &ensure_safe_relative_path(&output.to, "source.build.outputs.to")?,
                "source.build.outputs.to",
            )?;
            outputs.push((from, to));
        }
        Ok(outputs)
    }
}

fn ensure_no_raw_server_reference(value: &str, field: &'static str) -> Result<(), ContractError> {
    if value.contains("://") {
        return Err(ContractError::CliUsage {
            message: format!(
                "{field} must reference a package-relative artifact path; raw server URLs are not allowed: {value}"
            ),
        });
    }
    Ok(())
}

pub(crate) fn load_plugin_definition_and_source(
    path: &Path,
) -> Result<(PluginManifest, SourceInstallManifest), ContractError> {
    let contents = read_toml_document(path, "plugin definition")?;
    let manifest: PluginManifest = decode_toml_document(path, &contents, "plugin definition")?;
    let envelope: PluginSourceConfigEnvelope =
        decode_toml_document(path, &contents, "plugin source metadata")?;
    let source_manifest = envelope.source.ok_or_else(|| ContractError::CliUsage {
        message: format!(
            "plugin source metadata must be declared in config.toml [source]: {}",
            path.display()
        ),
    })?;
    source_manifest.validate(path)?;
    Ok((manifest, source_manifest))
}

fn read_toml_document(path: &Path, kind: &'static str) -> Result<String, ContractError> {
    match fs::read_to_string(path) {
        Ok(contents) => Ok(contents),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Err(ContractError::MissingFile {
            path: path.to_path_buf(),
            kind,
        }),
        Err(err) => Err(ContractError::Io {
            path: path.to_path_buf(),
            operation: "read file",
            source: err,
        }),
    }
}

fn decode_toml_document<T>(
    path: &Path,
    contents: &str,
    _kind: &'static str,
) -> Result<T, ContractError>
where
    T: DeserializeOwned,
{
    toml::from_str(contents)
        .map_err(|source| ContractError::toml_decode(path.to_path_buf(), contents, source))
}

impl SourceInstallMode {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::BuildRequired => "build_required",
        }
    }
}

impl SourceRuntime {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::Python => "python",
            Self::Node => "node",
            Self::Bin => "bin",
            Self::Wasm => "wasm",
        }
    }
}

impl BuildKind {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::Cargo => "cargo",
            Self::Npm => "npm",
        }
    }
}
