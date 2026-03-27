//! [INPUT]
//! Bootstrap root layout candidates, canonical root config files, and trigger package definitions.
//!
//! [OUTPUT]
//! Resolves effective root layouts with version reconciliation side effects and exposes trigger-list/toggle loader operations.
//!
//! [ROLE]
//! Owns infrastructure loader side effects and root/trigger loading entrypoints.

use std::fs;
use std::path::{Path, PathBuf};

use crate::domain::trigger::TriggerDefinition;
use crate::errors::ContractError;

use super::package_loader::{
    decode_package_collection, decode_required_toml, validate_directory_exists,
};
use super::{RootConfigDefinition, RootLayout, PACKAGE_CONFIG_FILE_NAME};

const RUNNING_CHAINBOT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerToggleResult {
    pub trigger_id: String,
    pub workflow_id: String,
    pub previous_enabled: bool,
    pub enabled: bool,
    pub changed: bool,
    pub config_path: PathBuf,
}

pub fn resolve_root_layout() -> Result<RootLayout, ContractError> {
    RootLayout::resolve()
}

pub fn load_effective_root_layout(base_layout: &RootLayout) -> Result<RootLayout, ContractError> {
    base_layout.validate_bootstrap_paths_exist()?;
    let root_config_path = resolve_root_config_path(base_layout)?;
    let mut root_config: RootConfigDefinition =
        decode_required_toml(&root_config_path, "root config")?;
    root_config.validate()?;
    reconcile_root_config_version(&root_config_path, &mut root_config)?;
    base_layout.apply_root_config(&root_config)
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
    let _ = resolve_root_config_path(&effective_layout)?;
    validate_directory_exists(&effective_layout.triggers_dir, "triggers")?;
    let triggers: Vec<TriggerDefinition> =
        decode_package_collection(&effective_layout.triggers_dir)?;
    for trigger in &triggers {
        trigger.validate()?;
    }
    Ok(triggers)
}

pub(crate) fn resolve_root_config_path(layout: &RootLayout) -> Result<PathBuf, ContractError> {
    let path = layout.root_config_path();
    if path.is_file() {
        Ok(path)
    } else {
        Err(ContractError::MissingFile {
            path,
            kind: "root config",
        })
    }
}

fn reconcile_root_config_version(
    root_config_path: &Path,
    root_config: &mut RootConfigDefinition,
) -> Result<(), ContractError> {
    match root_config.chainbot_version.as_deref() {
        Some(stored_version) if stored_version != RUNNING_CHAINBOT_VERSION => {
            run_root_config_version_migration(stored_version, RUNNING_CHAINBOT_VERSION)
        }
        None => {
            persist_root_config_version(root_config_path, root_config, RUNNING_CHAINBOT_VERSION)
        }
        Some(_) => Ok(()),
    }
}

fn run_root_config_version_migration(
    stored_version: &str,
    running_version: &str,
) -> Result<(), ContractError> {
    Err(ContractError::ConfigVersionMigrationRequired {
        stored_version: stored_version.to_owned(),
        running_version: running_version.to_owned(),
    })
}

fn persist_root_config_version(
    root_config_path: &Path,
    root_config: &mut RootConfigDefinition,
    running_version: &str,
) -> Result<(), ContractError> {
    root_config.chainbot_version = Some(running_version.to_owned());
    let contents =
        toml::to_string_pretty(root_config).map_err(|source| ContractError::TomlEncode {
            path: root_config_path.to_path_buf(),
            source,
        })?;
    write_atomic_string(root_config_path, &contents)
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
