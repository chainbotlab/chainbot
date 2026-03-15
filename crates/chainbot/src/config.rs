/*
[INPUT]:  Root path overrides, TOML definitions on disk, and JSON contract fixtures.
[OUTPUT]: Explicit root layout paths plus validated TOML definition bundle.
[POS]:    Config boundary module for root layout and definition loading.
[UPDATE]: 2026-03-16 - Add explicit root resolver and TOML loaders.
*/

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

pub const CURRENT_SCHEMA_MAJOR: u64 = 1;
pub const DEFAULT_ROOT_DIR_NAME: &str = ".chainbot";
pub const ROOT_CONFIG_FILE_NAME: &str = "root.toml";

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
    pub schema_version: String,
    #[serde(default)]
    pub profile: Option<String>,
    #[serde(default)]
    pub secret_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RootDefinitionBundle {
    pub root_config: RootConfigDefinition,
    pub workflows: Vec<WorkflowDefinition>,
    pub triggers: Vec<TriggerDefinition>,
    pub plugins: Vec<PluginManifest>,
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
            "root_config.schema_version",
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
    pub fn resolve(root_override: Option<&Path>) -> Result<Self, ContractError> {
        Self::resolve_with_home(root_override, None)
    }

    pub fn resolve_with_home(
        root_override: Option<&Path>,
        home_override: Option<&Path>,
    ) -> Result<Self, ContractError> {
        let root = match root_override {
            Some(path) => path.to_path_buf(),
            None => {
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
        layout.validate_paths_exist()?;

        let root_config: RootConfigDefinition =
            decode_required_toml(&layout.root_config_path(), "root config")?;
        root_config.validate()?;

        let workflows: Vec<WorkflowDefinition> = decode_toml_collection(&layout.workflows_dir)?;
        for workflow in &workflows {
            workflow.validate()?;
        }

        let triggers: Vec<TriggerDefinition> = decode_toml_collection(&layout.triggers_dir)?;
        for trigger in &triggers {
            trigger.validate()?;
        }

        let plugins: Vec<PluginManifest> = decode_toml_collection(&layout.plugins_dir)?;
        for plugin in &plugins {
            plugin.validate()?;
        }

        Ok(Self {
            root_config,
            workflows,
            triggers,
            plugins,
        })
    }
}

pub fn resolve_root_layout(root_override: Option<&Path>) -> Result<RootLayout, ContractError> {
    RootLayout::resolve(root_override)
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

fn decode_toml_collection<T>(directory: &Path) -> Result<Vec<T>, ContractError>
where
    T: DeserializeOwned,
{
    let mut definitions = Vec::new();

    for path in collect_toml_files(directory)? {
        let definition = decode_required_toml(&path, "definition")?;
        definitions.push(definition);
    }

    Ok(definitions)
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
