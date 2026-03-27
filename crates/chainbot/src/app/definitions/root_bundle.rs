//! [INPUT]
//! Root layout resolution, package decoding helpers, plugin manifests, and workflow or trigger domain definitions.
//!
//! [OUTPUT]
//! Loads a validated `RootDefinitionBundle` containing root config, workflows, triggers, and plugins.
//!
//! [ROLE]
//! Bridges infrastructure-backed workspace discovery into the application definition-loading boundary.

use crate::domain::trigger::TriggerDefinition;
use crate::domain::workflow::WorkflowDefinition;
use crate::errors::ContractError;
use crate::infrastructure::config::loader::{load_effective_root_layout, resolve_root_config_path};
use crate::infrastructure::config::package_loader::{
    decode_package_collection, decode_plugin_manifests, decode_required_toml,
};
use crate::infrastructure::config::{RootConfigDefinition, RootDefinitionBundle, RootLayout};
use crate::plugin::PluginManifest;

use super::validate::validate_bundle_contracts;

pub fn load_root_definition_bundle(
    layout: &RootLayout,
) -> Result<RootDefinitionBundle, ContractError> {
    RootDefinitionBundle::load(layout)
}

impl RootDefinitionBundle {
    pub fn load(layout: &RootLayout) -> Result<Self, ContractError> {
        let effective_layout = load_effective_root_layout(layout)?;
        effective_layout.validate_paths_exist()?;

        let root_config: RootConfigDefinition =
            decode_required_toml(&resolve_root_config_path(&effective_layout)?, "root config")?;
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

        let plugins: Vec<PluginManifest> = decode_plugin_manifests(&effective_layout.plugins_dir)?;
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
