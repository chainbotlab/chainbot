//! [INPUT]
//! Environment-derived root paths, on-disk package manifests, plugin manifests, and serialized contract payloads.
//!
//! [OUTPUT]
//! Resolves canonical root layouts and loads validated root, workflow, trigger, plugin, and worker definition bundles with package-root context.
//!
//! [ROLE]
//! Defines the configuration and package-loading boundary for ChainBot runtime state on disk.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::errors::{assert_supported_major, ContractError};
use crate::plugin::PluginManifest;
use crate::secrets::SecretReference;
use crate::state::RunRecordSummary;
use crate::trigger::TriggerDefinition;
use crate::worker::{WorkerRequestEnvelope, WorkerResponseEnvelope};
use crate::workflow::WorkflowDefinition;

pub const CURRENT_SCHEMA_MAJOR: u64 = 2;
pub const DEFAULT_ROOT_DIR_NAME: &str = ".chainbot";
pub const CHAINBOT_CONFIG_DIR_ENV: &str = "CHAINBOT_CONFIG_DIR";
pub const ROOT_CONFIG_FILE_NAME: &str = "root.toml";
pub const PACKAGE_CONFIG_FILE_NAME: &str = "config.toml";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfigRoot {
    pub schema_version: String,
    pub workflows: Vec<WorkflowDefinition>,
    pub plugins: Vec<PluginManifest>,
    pub worker_templates: Vec<WorkerTemplate>,
    pub run_defaults: Option<RunRecordSummary>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkerTemplate {
    pub name: String,
    pub request: WorkerRequestEnvelope,
    pub response: WorkerResponseEnvelope,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RootConfigDefinition {
    #[serde(rename = "manifest_version", alias = "schema_version")]
    pub schema_version: String,
    #[serde(default)]
    pub profile: Option<String>,
    #[serde(default)]
    pub secret_refs: Vec<String>,
    #[serde(default)]
    pub runtime_defaults: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub paths: RootPathOverrides,
    #[serde(default)]
    pub plugins: RootPluginDiscovery,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RootPathOverrides {
    #[serde(default)]
    pub workflows_dir: Option<String>,
    #[serde(default)]
    pub triggers_dir: Option<String>,
    #[serde(default)]
    pub plugins_dir: Option<String>,
    #[serde(default)]
    pub secrets_dir: Option<String>,
    #[serde(default)]
    pub state_dir: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RootPluginDiscovery {
    #[serde(default)]
    pub manifest_globs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RootDefinitionBundle {
    pub root_config: RootConfigDefinition,
    pub workflows: Vec<WorkflowDefinition>,
    pub triggers: Vec<TriggerDefinition>,
    pub plugins: Vec<PluginManifest>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerToggleResult {
    pub trigger_id: String,
    pub workflow_id: String,
    pub previous_enabled: bool,
    pub enabled: bool,
    pub changed: bool,
    pub config_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootLayout {
    pub root: PathBuf,
    pub config_dir: PathBuf,
    pub workflows_dir: PathBuf,
    pub triggers_dir: PathBuf,
    pub plugins_dir: PathBuf,
    pub secrets_dir: PathBuf,
    pub state_dir: PathBuf,
}

impl ConfigRoot {
    pub fn validate(&self) -> Result<(), ContractError> {
        assert_supported_major(
            "config.schema_version",
            &self.schema_version,
            CURRENT_SCHEMA_MAJOR,
        )?;

        for workflow in &self.workflows {
            workflow.validate()?;
        }

        for plugin in &self.plugins {
            plugin.validate()?;
        }

        for template in &self.worker_templates {
            template.request.validate()?;
            template.response.validate()?;
        }

        if let Some(summary) = &self.run_defaults {
            summary.validate()?;
        }

        Ok(())
    }

    pub fn from_json_str(input: &str) -> Result<Self, ContractError> {
        let config: Self = serde_json::from_str(input)?;
        config.validate()?;
        Ok(config)
    }
}

impl RootConfigDefinition {
    pub fn validate(&self) -> Result<(), ContractError> {
        assert_supported_major(
            "root_config.manifest_version",
            &self.schema_version,
            CURRENT_SCHEMA_MAJOR,
        )?;

        for secret_ref in &self.secret_refs {
            let _ = SecretReference::parse(secret_ref)?;
        }

        Ok(())
    }
}

impl RootLayout {
    pub fn resolve() -> Result<Self, ContractError> {
        Self::resolve_with_env_home(None, None)
    }

    pub fn resolve_with_env_home(
        config_dir_override: Option<&Path>,
        home_override: Option<&Path>,
    ) -> Result<Self, ContractError> {
        let root = match config_dir_override {
            Some(path) => path.to_path_buf(),
            None => {
                if let Some(path) =
                    std::env::var_os(CHAINBOT_CONFIG_DIR_ENV).filter(|value| !value.is_empty())
                {
                    return Ok(Self::from_root(PathBuf::from(path)));
                }
                let home = match home_override {
                    Some(path) => path.to_path_buf(),
                    None => {
                        let home_os =
                            std::env::var_os("HOME").ok_or(ContractError::MissingHomeDirectory)?;
                        PathBuf::from(home_os)
                    }
                };
                home.join(DEFAULT_ROOT_DIR_NAME)
            }
        };

        Ok(Self::from_root(root))
    }

    pub fn from_root(root: PathBuf) -> Self {
        Self {
            config_dir: root.join("config"),
            workflows_dir: root.join("workflows"),
            triggers_dir: root.join("triggers"),
            plugins_dir: root.join("plugins"),
            secrets_dir: root.join("secrets"),
            state_dir: root.join("state"),
            root,
        }
    }

    pub fn root_config_path(&self) -> PathBuf {
        self.config_dir.join(ROOT_CONFIG_FILE_NAME)
    }

    pub fn plugin_manifests_dir(&self) -> PathBuf {
        self.plugins_dir.join("manifests")
    }

    pub fn validate_bootstrap_paths_exist(&self) -> Result<(), ContractError> {
        validate_directory_exists(&self.root, "root")?;
        validate_directory_exists(&self.config_dir, "config")?;
        Ok(())
    }

    pub fn validate_paths_exist(&self) -> Result<(), ContractError> {
        validate_directory_exists(&self.root, "root")?;
        validate_directory_exists(&self.config_dir, "config")?;
        validate_directory_exists(&self.workflows_dir, "workflows")?;
        validate_directory_exists(&self.triggers_dir, "triggers")?;
        validate_directory_exists(&self.plugins_dir, "plugins")?;
        validate_directory_exists(&self.secrets_dir, "secrets")?;
        validate_directory_exists(&self.state_dir, "state")?;
        Ok(())
    }
}

impl RootDefinitionBundle {
    pub fn load(layout: &RootLayout) -> Result<Self, ContractError> {
        let effective_layout = load_effective_root_layout(layout)?;
        effective_layout.validate_paths_exist()?;

        let root_config: RootConfigDefinition =
            decode_required_toml(&effective_layout.root_config_path(), "root config")?;
        root_config.validate()?;

        let workflows: Vec<WorkflowDefinition> =
            decode_package_collection(&effective_layout.workflows_dir)?;
        for workflow in &workflows {
            workflow.validate()?;
        }

        let triggers: Vec<TriggerDefinition> =
            decode_package_collection(&effective_layout.triggers_dir)?;
        for trigger in &triggers {
            trigger.validate()?;
        }

        let plugins: Vec<PluginManifest> = decode_plugin_manifests(
            &effective_layout.root,
            &effective_layout.plugins_dir,
            &root_config.plugins,
        )?;
        for plugin in &plugins {
            plugin.validate()?;
        }

        validate_bundle_contracts(&workflows, &triggers, &plugins)?;

        Ok(Self {
            root_config,
            workflows,
            triggers,
            plugins,
        })
    }
}

pub fn set_trigger_enabled(
    layout: &RootLayout,
    trigger_id: &str,
    enabled: bool,
) -> Result<TriggerToggleResult, ContractError> {
    let effective_layout = load_effective_root_layout(layout)?;
    let trigger = load_trigger_definitions(&effective_layout)?
        .into_iter()
        .find(|definition| definition.trigger_id == trigger_id)
        .ok_or_else(|| ContractError::CliUsage {
            message: format!(
                "Unknown trigger `{trigger_id}`. Run `chainbot trigger list` to inspect configured triggers."
            ),
        })?;

    let config_path = trigger.package_root.join(PACKAGE_CONFIG_FILE_NAME);
    let mut stored_definition: TriggerDefinition =
        decode_required_toml(&config_path, "definition")?;
    stored_definition.package_root = trigger.package_root;
    let previous_enabled = stored_definition.enabled;
    stored_definition.enabled = enabled;
    stored_definition.validate()?;

    if previous_enabled != enabled {
        let contents = toml::to_string_pretty(&stored_definition).map_err(|source| {
            ContractError::TomlEncode {
                path: config_path.clone(),
                source,
            }
        })?;
        write_atomic_string(&config_path, &contents)?;
    }

    Ok(TriggerToggleResult {
        trigger_id: stored_definition.trigger_id,
        workflow_id: stored_definition.workflow_id,
        previous_enabled,
        enabled,
        changed: previous_enabled != enabled,
        config_path,
    })
}

pub fn load_trigger_definitions(
    layout: &RootLayout,
) -> Result<Vec<TriggerDefinition>, ContractError> {
    let effective_layout = load_effective_root_layout(layout)?;
    validate_directory_exists(&effective_layout.root, "root")?;
    validate_directory_exists(&effective_layout.config_dir, "config")?;
    validate_directory_exists(&effective_layout.triggers_dir, "triggers")?;
    let triggers: Vec<TriggerDefinition> =
        decode_package_collection(&effective_layout.triggers_dir)?;
    for trigger in &triggers {
        trigger.validate()?;
    }
    Ok(triggers)
}

fn validate_bundle_contracts(
    workflows: &[WorkflowDefinition],
    triggers: &[TriggerDefinition],
    plugins: &[PluginManifest],
) -> Result<(), ContractError> {
    let mut workflow_ids = std::collections::BTreeSet::new();
    for workflow in workflows {
        if !workflow_ids.insert(workflow.workflow_id.clone()) {
            return Err(ContractError::DuplicateWorkflowId {
                workflow_id: workflow.workflow_id.clone(),
            });
        }
        validate_package_identity("workflow", &workflow.package_root, &workflow.workflow_id)?;
    }

    let mut trigger_ids = std::collections::BTreeSet::new();
    for trigger in triggers {
        if !trigger_ids.insert(trigger.trigger_id.clone()) {
            return Err(ContractError::DuplicateTriggerId {
                trigger_id: trigger.trigger_id.clone(),
            });
        }
        validate_package_identity("trigger", &trigger.package_root, &trigger.trigger_id)?;
        if !workflow_ids.contains(&trigger.workflow_id) {
            return Err(ContractError::TriggerReferencesUnknownWorkflow {
                trigger_id: trigger.trigger_id.clone(),
                workflow_id: trigger.workflow_id.clone(),
            });
        }
    }

    let mut plugin_ids = std::collections::BTreeSet::new();
    for plugin in plugins {
        if !plugin_ids.insert(plugin.plugin_id.clone()) {
            return Err(ContractError::DuplicatePluginId {
                plugin_id: plugin.plugin_id.clone(),
            });
        }
    }

    Ok(())
}

fn validate_package_identity(
    kind: &'static str,
    package_root: &Path,
    expected_id: &str,
) -> Result<(), ContractError> {
    let directory_name = package_root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_owned();
    if directory_name != expected_id {
        return Err(ContractError::PackageDirectoryIdentityMismatch {
            kind,
            path: package_root.to_path_buf(),
            expected_id: expected_id.to_owned(),
            directory_name,
        });
    }
    Ok(())
}

pub fn resolve_root_layout() -> Result<RootLayout, ContractError> {
    RootLayout::resolve()
}

pub fn load_effective_root_layout(base_layout: &RootLayout) -> Result<RootLayout, ContractError> {
    base_layout.validate_bootstrap_paths_exist()?;
    let root_config: RootConfigDefinition =
        decode_required_toml(&base_layout.root_config_path(), "root config")?;
    root_config.validate()?;
    base_layout.apply_root_config(&root_config)
}

impl RootLayout {
    pub fn apply_root_config(
        &self,
        root_config: &RootConfigDefinition,
    ) -> Result<Self, ContractError> {
        Ok(Self {
            root: self.root.clone(),
            config_dir: self.config_dir.clone(),
            workflows_dir: resolve_root_relative_dir(
                &self.root,
                "root_config.paths.workflows_dir",
                root_config
                    .paths
                    .workflows_dir
                    .as_deref()
                    .unwrap_or("workflows"),
            )?,
            triggers_dir: resolve_root_relative_dir(
                &self.root,
                "root_config.paths.triggers_dir",
                root_config
                    .paths
                    .triggers_dir
                    .as_deref()
                    .unwrap_or("triggers"),
            )?,
            plugins_dir: resolve_root_relative_dir(
                &self.root,
                "root_config.paths.plugins_dir",
                root_config
                    .paths
                    .plugins_dir
                    .as_deref()
                    .unwrap_or("plugins"),
            )?,
            secrets_dir: resolve_root_relative_dir(
                &self.root,
                "root_config.paths.secrets_dir",
                root_config
                    .paths
                    .secrets_dir
                    .as_deref()
                    .unwrap_or("secrets"),
            )?,
            state_dir: resolve_root_relative_dir(
                &self.root,
                "root_config.paths.state_dir",
                root_config.paths.state_dir.as_deref().unwrap_or("state"),
            )?,
        })
    }
}

fn validate_directory_exists(path: &Path, kind: &'static str) -> Result<(), ContractError> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_dir() => Ok(()),
        Ok(_) => Err(ContractError::MissingDirectory {
            path: path.to_path_buf(),
            kind,
        }),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            Err(ContractError::MissingDirectory {
                path: path.to_path_buf(),
                kind,
            })
        }
        Err(err) => Err(ContractError::Io {
            path: path.to_path_buf(),
            operation: "inspect metadata",
            source: err,
        }),
    }
}

fn decode_required_toml<T>(path: &Path, kind: &'static str) -> Result<T, ContractError>
where
    T: DeserializeOwned,
{
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Err(ContractError::MissingFile {
                path: path.to_path_buf(),
                kind,
            });
        }
        Err(err) => {
            return Err(ContractError::Io {
                path: path.to_path_buf(),
                operation: "read file",
                source: err,
            });
        }
    };

    toml::from_str(&contents).map_err(|source| ContractError::TomlDecode {
        path: path.to_path_buf(),
        source,
    })
}

fn write_atomic_string(path: &Path, contents: &str) -> Result<(), ContractError> {
    let temporary_path = path.with_extension("toml.tmp");
    fs::write(&temporary_path, contents).map_err(|source| ContractError::Io {
        path: temporary_path.clone(),
        operation: "write file",
        source,
    })?;
    fs::rename(&temporary_path, path).map_err(|source| ContractError::Io {
        path: path.to_path_buf(),
        operation: "rename file",
        source,
    })?;
    Ok(())
}

fn decode_package_collection<T>(directory: &Path) -> Result<Vec<T>, ContractError>
where
    T: DeserializeOwned + PackageManifestExt,
{
    let mut definitions = Vec::new();

    for path in collect_package_config_files(directory)? {
        let mut definition = decode_required_toml::<T>(&path, "definition")?;
        definition.set_package_root(
            path.parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(PathBuf::new),
        );
        definitions.push(definition);
    }

    Ok(definitions)
}

fn decode_plugin_manifests(
    root: &Path,
    plugins_dir: &Path,
    discovery: &RootPluginDiscovery,
) -> Result<Vec<PluginManifest>, ContractError> {
    let mut definitions = Vec::new();

    for path in collect_plugin_manifest_files(root, plugins_dir, discovery)? {
        let mut definition = decode_required_toml::<PluginManifest>(&path, "definition")?;
        definition.manifest_path = path;
        definitions.push(definition);
    }

    Ok(definitions)
}

fn collect_plugin_manifest_files(
    root: &Path,
    plugins_dir: &Path,
    discovery: &RootPluginDiscovery,
) -> Result<Vec<PathBuf>, ContractError> {
    let manifest_globs = if discovery.manifest_globs.is_empty() {
        vec![String::from("plugins/manifests/*.toml")]
    } else {
        discovery.manifest_globs.clone()
    };

    let mut files = Vec::new();
    for pattern in manifest_globs {
        let Some(directory_pattern) = pattern.strip_suffix("/*.toml") else {
            return Err(ContractError::InvalidRootConfigField {
                field: "root_config.plugins.manifest_globs",
                detail: format!(
                    "unsupported pattern `{pattern}`; expected a root-relative <dir>/*.toml form"
                ),
            });
        };
        let directory = if directory_pattern == "plugins/manifests" {
            plugins_dir.join("manifests")
        } else {
            resolve_root_relative_dir(
                root,
                "root_config.plugins.manifest_globs",
                directory_pattern,
            )?
        };
        files.extend(collect_toml_files(&directory)?);
    }

    files.sort();
    files.dedup();
    Ok(files)
}

fn collect_toml_files(directory: &Path) -> Result<Vec<PathBuf>, ContractError> {
    let entries = fs::read_dir(directory).map_err(|source| ContractError::Io {
        path: directory.to_path_buf(),
        operation: "read directory",
        source,
    })?;

    let mut files = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| ContractError::Io {
            path: directory.to_path_buf(),
            operation: "read directory entry",
            source,
        })?;
        let path = entry.path();
        if path.is_file() && path.extension() == Some(OsStr::new("toml")) {
            files.push(path);
        }
    }

    files.sort();
    Ok(files)
}

fn collect_package_config_files(directory: &Path) -> Result<Vec<PathBuf>, ContractError> {
    let entries = fs::read_dir(directory).map_err(|source| ContractError::Io {
        path: directory.to_path_buf(),
        operation: "read directory",
        source,
    })?;

    let mut files = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| ContractError::Io {
            path: directory.to_path_buf(),
            operation: "read directory entry",
            source,
        })?;
        let path = entry.path();
        if path.is_dir() {
            let config_path = path.join(PACKAGE_CONFIG_FILE_NAME);
            if config_path.is_file() {
                files.push(config_path);
            }
        }
    }

    files.sort();
    Ok(files)
}

fn resolve_root_relative_dir(
    root: &Path,
    field: &'static str,
    value: &str,
) -> Result<PathBuf, ContractError> {
    let candidate = Path::new(value);
    if candidate.is_absolute() {
        return Err(ContractError::InvalidRootConfigField {
            field,
            detail: format!("absolute paths are not allowed: {value}"),
        });
    }

    if candidate
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(ContractError::InvalidRootConfigField {
            field,
            detail: format!("paths must stay within <root> and may not contain `..`: {value}"),
        });
    }

    Ok(root.join(candidate))
}

trait PackageManifestExt {
    fn set_package_root(&mut self, package_root: PathBuf);
}

impl PackageManifestExt for WorkflowDefinition {
    fn set_package_root(&mut self, package_root: PathBuf) {
        self.package_root = package_root;
    }
}

impl PackageManifestExt for TriggerDefinition {
    fn set_package_root(&mut self, package_root: PathBuf) {
        self.package_root = package_root;
    }
}
