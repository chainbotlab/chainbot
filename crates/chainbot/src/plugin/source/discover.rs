//! [INPUT]
//! Materialized source repositories, plugin package paths, source metadata, and plugin manifests.
//!
//! [OUTPUT]
//! Produces source repository read models and validated discovered plugin packages for source list/show/install flows.
//!
//! [ROLE]
//! Owns plugin package discovery and source-facing metadata projection inside the source-install subsystem.

use std::path::Path;

use crate::errors::ContractError;
use crate::infrastructure::config::package_loader::validate_directory_exists;
use crate::plugin::PluginManifest;

use super::fs::{ensure_safe_relative_path, resolve_within_root};
use super::manifest::{
    load_plugin_definition_and_source, PluginSourceBuildDetail, PluginSourceBuildOutputDetail,
    PluginSourceDescriptor, PluginSourceDetail, PluginSourceListOutput, PluginSourceShowOutput,
    PluginSourceSummary, SourceIndexManifest, SourceInstallManifest, SourceInstallMode,
    LEGACY_SOURCE_INSTALL_FILE_NAME, SOURCE_INDEX_FILE_NAME,
};
use super::transport::MaterializedSource;

#[derive(Debug, Clone)]
pub(crate) struct DiscoveredPlugin {
    pub(crate) plugin_id: String,
    pub(crate) path: String,
    pub(crate) summary: Option<String>,
    pub(crate) manifest: PluginManifest,
    pub(crate) source_manifest: SourceInstallManifest,
}

#[derive(Debug, Clone)]
pub(crate) struct SourceRepository {
    pub(crate) descriptor: PluginSourceDescriptor,
    pub(crate) plugins: Vec<DiscoveredPlugin>,
}

pub(crate) fn discover_source_repository(
    source: &MaterializedSource,
    descriptor: PluginSourceDescriptor,
) -> Result<SourceRepository, ContractError> {
    let index_path = source.repo_root.join(SOURCE_INDEX_FILE_NAME);
    let plugins = if index_path.is_file() {
        discover_multi_plugin_repository(source, &index_path)?
    } else {
        vec![discover_single_plugin_repository(source)?]
    };
    Ok(SourceRepository {
        descriptor,
        plugins,
    })
}

pub(crate) fn resolve_plugin_selection<'a>(
    repository: &'a SourceRepository,
    requested_plugin_id: Option<&str>,
    command_name: &str,
) -> Result<&'a DiscoveredPlugin, ContractError> {
    match (repository.plugins.as_slice(), requested_plugin_id) {
        ([plugin], None) => Ok(plugin),
        ([plugin], Some(value)) if plugin.plugin_id == value => Ok(plugin),
        ([plugin], Some(value)) => Err(ContractError::CliUsage {
            message: format!(
                "unknown plugin `{value}` for {command_name}; available plugin_id: {}",
                plugin.plugin_id
            ),
        }),
        (_, None) => Err(ContractError::CliUsage {
            message: format!(
                "{command_name} requires `--plugin <plugin_id>` because the source repo contains multiple plugins: {}",
                repository
                    .plugins
                    .iter()
                    .map(|plugin| plugin.plugin_id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }),
        (_, Some(plugin_id)) => repository
            .plugins
            .iter()
            .find(|plugin| plugin.plugin_id == plugin_id)
            .ok_or_else(|| ContractError::CliUsage {
                message: format!(
                    "unknown plugin `{plugin_id}`; available plugin ids: {}",
                    repository
                        .plugins
                        .iter()
                        .map(|plugin| plugin.plugin_id.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            }),
    }
}

pub(crate) fn build_list_output(repository: &SourceRepository) -> PluginSourceListOutput {
    PluginSourceListOutput {
        source: repository.descriptor.clone(),
        plugins: repository
            .plugins
            .iter()
            .map(|plugin| PluginSourceSummary {
                plugin_id: plugin.plugin_id.clone(),
                path: plugin.path.clone(),
                summary: plugin.summary.clone(),
                plugin_kind: plugin.manifest.kind.clone(),
                runtime: plugin.source_manifest.runtime.as_str().to_owned(),
                install_mode: plugin.source_manifest.install_mode.as_str().to_owned(),
                surfaces: plugin_surface_summary(&plugin.manifest),
                release_version: plugin.source_manifest.release_version.clone(),
            })
            .collect(),
    }
}

pub(crate) fn build_show_output(
    repository: &SourceRepository,
    plugin: &DiscoveredPlugin,
) -> PluginSourceShowOutput {
    PluginSourceShowOutput {
        source: repository.descriptor.clone(),
        plugin: PluginSourceDetail {
            plugin_id: plugin.plugin_id.clone(),
            path: plugin.path.clone(),
            summary: plugin.summary.clone(),
            plugin_kind: plugin.manifest.kind.clone(),
            runtime: plugin.source_manifest.runtime.as_str().to_owned(),
            install_mode: plugin.source_manifest.install_mode.as_str().to_owned(),
            release_version: plugin.source_manifest.release_version.clone(),
            entrypoint: plugin.manifest.entrypoint.clone(),
            capabilities: plugin.manifest.capabilities.clone(),
            entry_artifact: plugin.source_manifest.entry_artifact.clone(),
            surfaces: plugin_surface_summary(&plugin.manifest),
            build: plugin
                .source_manifest
                .build
                .as_ref()
                .map(|build| PluginSourceBuildDetail {
                    kind: build.kind.as_str().to_owned(),
                    command: build.command.clone(),
                    workdir: build.workdir.clone(),
                    outputs: build
                        .outputs
                        .iter()
                        .map(|output| PluginSourceBuildOutputDetail {
                            from: output.from.clone(),
                            to: output.to.clone(),
                        })
                        .collect(),
                }),
        },
    }
}

fn plugin_surface_summary(manifest: &PluginManifest) -> Vec<String> {
    let mut summary = Vec::new();
    if matches!(manifest.plugin_id.as_str(), "eth-node" | "eth-trigger") {
        summary.push(String::from("chain=ethereum"));
    } else if matches!(manifest.plugin_id.as_str(), "solana-node" | "solana-trigger") {
        summary.push(String::from("chain=solana"));
    }

    if !manifest.operations.is_empty() {
        let operations = manifest
            .operations
            .iter()
            .map(|operation| match operation.kind {
                crate::plugin::PluginOperationKind::Generic => operation.name.clone(),
                crate::plugin::PluginOperationKind::Read => format!("{}:read", operation.name),
                crate::plugin::PluginOperationKind::Write => format!("{}:write", operation.name),
                crate::plugin::PluginOperationKind::Transfer => {
                    format!("{}:transfer", operation.name)
                }
                crate::plugin::PluginOperationKind::RawRead => {
                    format!("{}:raw_read", operation.name)
                }
                crate::plugin::PluginOperationKind::RawWrite => {
                    format!("{}:raw_write", operation.name)
                }
            })
            .collect::<Vec<_>>()
            .join(", ");
        summary.push(format!("operations={operations}"));
    }

    if let Some(event_schema) = manifest.event_schema.as_ref()
        && !event_schema.listener_modes.is_empty()
    {
        let listeners = event_schema
            .listener_modes
            .iter()
            .map(|mode| match mode {
                crate::plugin::PluginTriggerListenerMode::EventLog => "event_log",
                crate::plugin::PluginTriggerListenerMode::StateChange => "state_change",
            })
            .collect::<Vec<_>>()
            .join(", ");
        summary.push(format!("listeners={listeners}"));
    }

    summary
}

fn discover_single_plugin_repository(
    source: &MaterializedSource,
) -> Result<DiscoveredPlugin, ContractError> {
    load_plugin_package(source, &source.repo_root, String::from("."), None)
}

fn discover_multi_plugin_repository(
    source: &MaterializedSource,
    index_path: &Path,
) -> Result<Vec<DiscoveredPlugin>, ContractError> {
    let index = SourceIndexManifest::load(index_path)?;
    let mut plugins = Vec::with_capacity(index.plugins.len());
    for entry in index.plugins {
        let relative =
            ensure_safe_relative_path(&entry.path, "chainbot-plugin-index.plugins.path")?;
        let package_root = resolve_within_root(
            &source.repo_root,
            &relative,
            "chainbot-plugin-index.plugins.path",
        )?;
        plugins.push(load_plugin_package(
            source,
            &package_root,
            entry.path,
            Some(entry.summary),
        )?);
    }
    Ok(plugins)
}

fn load_plugin_package(
    source: &MaterializedSource,
    package_root: &Path,
    path_label: String,
    summary: Option<String>,
) -> Result<DiscoveredPlugin, ContractError> {
    validate_directory_exists(package_root, "plugin package")?;
    let manifest_path = package_root.join("config.toml");
    let legacy_source_path = package_root.join(LEGACY_SOURCE_INSTALL_FILE_NAME);
    if legacy_source_path.is_file() {
        return Err(ContractError::CliUsage {
            message: format!(
                "legacy source.toml is no longer supported; move metadata into config.toml [source]: {}",
                legacy_source_path.display()
            ),
        });
    }

    let (mut manifest, source_manifest) = load_plugin_definition_and_source(&manifest_path)?;
    manifest.manifest_path = manifest_path;
    manifest.validate()?;
    validate_manifest_alignment(&manifest, &source_manifest)?;
    validate_source_entry_artifact_anchor(package_root, &manifest, &source_manifest)?;

    if !package_root.starts_with(&source.repo_root) {
        return Err(ContractError::CliUsage {
            message: format!(
                "plugin package path escapes repo root: {}",
                package_root.display()
            ),
        });
    }

    Ok(DiscoveredPlugin {
        plugin_id: manifest.plugin_id.clone(),
        path: path_label,
        summary,
        manifest,
        source_manifest,
    })
}

fn validate_manifest_alignment(
    manifest: &PluginManifest,
    source_manifest: &SourceInstallManifest,
) -> Result<(), ContractError> {
    if manifest.is_streamable_http_mcp_entrypoint() {
        return Ok(());
    }

    match source_manifest.runtime.as_str() {
        "python" | "node" | "bin" => {
            let executable =
                manifest
                    .executable
                    .as_deref()
                    .ok_or_else(|| ContractError::CliUsage {
                        message: format!(
                            "plugin {} runtime {} requires plugin.executable in config.toml",
                            manifest.plugin_id,
                            source_manifest.runtime.as_str()
                        ),
                    })?;
            if executable != source_manifest.entry_artifact {
                return Err(ContractError::CliUsage {
                    message: format!(
                        "plugin {} source.entry_artifact `{}` must match plugin.executable `{}`",
                        manifest.plugin_id, source_manifest.entry_artifact, executable
                    ),
                });
            }
        }
        "wasm" => {
            let module = manifest
                .trigger_runtime
                .as_ref()
                .and_then(|runtime| runtime.module.as_deref())
                .ok_or_else(|| ContractError::CliUsage {
                    message: format!(
                        "plugin {} runtime wasm requires plugin.trigger_runtime.module in config.toml",
                        manifest.plugin_id
                    ),
                })?;
            if module != source_manifest.entry_artifact {
                return Err(ContractError::CliUsage {
                    message: format!(
                        "plugin {} source.entry_artifact `{}` must match plugin.trigger_runtime.module `{}`",
                        manifest.plugin_id, source_manifest.entry_artifact, module
                    ),
                });
            }
        }
        _ => {}
    }
    Ok(())
}

fn validate_source_entry_artifact_anchor(
    package_root: &Path,
    manifest: &PluginManifest,
    source_manifest: &SourceInstallManifest,
) -> Result<(), ContractError> {
    let should_require_anchor = manifest.is_streamable_http_mcp_entrypoint()
        || matches!(source_manifest.install_mode, SourceInstallMode::Direct);
    if !should_require_anchor {
        return Ok(());
    }

    let artifact_path = source_manifest.entry_artifact_path(package_root)?;
    let metadata = std::fs::metadata(&artifact_path).map_err(|source| ContractError::CliUsage {
        message: format!(
            "entry artifact missing for plugin {} at {}: {source}",
            manifest.plugin_id,
            artifact_path.display()
        ),
    })?;
    if !metadata.is_file() {
        return Err(ContractError::CliUsage {
            message: format!(
                "entry artifact must be a file for plugin {}: {}",
                manifest.plugin_id,
                artifact_path.display()
            ),
        });
    }

    Ok(())
}
