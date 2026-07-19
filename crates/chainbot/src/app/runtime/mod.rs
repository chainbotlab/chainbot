//! [INPUT]
//! Long-running runtime orchestration dependencies from CLI command handlers and runtime/state subsystems.
//!
//! [OUTPUT]
//! App-layer runtime controllers for daemon lifecycle and serve-loop orchestration.
//!
//! [ROLE]
//! Isolates runtime process orchestration from CLI parsing/help and read-model rendering.

use std::collections::BTreeMap;

use crate::app::runtime::external_triggers::process_listener::{
    collect_external_process_trigger_emissions, validate_external_trigger_plugin_manifest,
};
use crate::domain::trigger::{
    TriggerDefinition, TriggerEmission, TriggerPlane, TriggerPlaneError, TriggerPluginHostPolicy,
};
use crate::infrastructure::config::{RuntimeStorageBackend, RuntimeStorageConfig};
use crate::infrastructure::state::{RuntimeStateStore, StateLayout};
use crate::plugin::PluginManifest;

pub(crate) mod daemon;
pub(crate) mod execution;
pub(crate) mod external_triggers;
pub(crate) mod plugin_activation;

impl TriggerPlane {
    pub fn open(
        state_store: RuntimeStateStore,
        definitions: Vec<TriggerDefinition>,
        plugin_manifests: Vec<PluginManifest>,
        policy: TriggerPluginHostPolicy,
        builtin_events: BTreeMap<String, Vec<TriggerEmission>>,
    ) -> Result<Self, TriggerPlaneError> {
        validate_external_trigger_manifests(&definitions, &plugin_manifests, &policy)?;
        Self::open_domain_with_store(state_store, definitions, builtin_events)
    }

    pub fn open_legacy_state_layout_for_tests(
        state_layout: StateLayout,
        definitions: Vec<TriggerDefinition>,
        plugin_manifests: Vec<PluginManifest>,
        policy: TriggerPluginHostPolicy,
        builtin_events: BTreeMap<String, Vec<TriggerEmission>>,
        now_ms: i64,
    ) -> Result<Self, TriggerPlaneError> {
        let mut state_store = RuntimeStateStore::open(
            &RuntimeStorageConfig {
                backend: RuntimeStorageBackend::Local {
                    database_path: state_layout.coordination_db_path,
                },
                history_retention: None,
                raw_debug_enabled: false,
                raw_debug_artifacts_dir: None,
            },
            now_ms,
        )
        .map_err(|error| TriggerPlaneError::runtime_state(error.to_string()))?;

        validate_external_trigger_manifests(&definitions, &plugin_manifests, &policy)?;
        let manifests_by_id = plugin_manifests
            .iter()
            .map(|manifest| (manifest.plugin_id.as_str(), manifest))
            .collect::<BTreeMap<_, _>>();
        for definition in definitions.iter().filter(|definition| definition.enabled) {
            if definition.kind()? != crate::domain::trigger::TriggerKind::ExternalPlugin {
                continue;
            }
            let plugin_id = definition.plugin.as_deref().ok_or_else(|| {
                TriggerPlaneError::Contract(
                    crate::errors::ContractError::InvalidTriggerDefinitionField {
                        trigger_id: definition.trigger_id.clone(),
                        field: "trigger.plugin",
                        detail: "value cannot be empty".to_owned(),
                    },
                )
            })?;
            let manifest = manifests_by_id.get(plugin_id).copied().ok_or_else(|| {
                TriggerPlaneError::Contract(crate::errors::ContractError::UnknownTriggerPlugin {
                    trigger_id: definition.trigger_id.clone(),
                    plugin_id: plugin_id.to_owned(),
                })
            })?;
            let mut on_progress = || Ok(());
            let _ = collect_external_process_trigger_emissions(
                &mut state_store,
                definition,
                manifest,
                &policy,
                &mut on_progress,
            )?;
        }

        Self::open_domain_with_store(state_store, definitions, builtin_events)
    }

    pub fn open_with_store(
        state_store: RuntimeStateStore,
        definitions: Vec<TriggerDefinition>,
        builtin_events: BTreeMap<String, Vec<TriggerEmission>>,
    ) -> Result<Self, TriggerPlaneError> {
        Self::open_domain_with_store(state_store, definitions, builtin_events)
    }
}

fn validate_external_trigger_manifests(
    definitions: &[TriggerDefinition],
    plugin_manifests: &[PluginManifest],
    policy: &TriggerPluginHostPolicy,
) -> Result<(), TriggerPlaneError> {
    let manifests_by_id = plugin_manifests
        .iter()
        .map(|manifest| (manifest.plugin_id.as_str(), manifest))
        .collect::<BTreeMap<_, _>>();

    for definition in definitions {
        if !definition.enabled {
            continue;
        }
        if definition.kind()? != crate::domain::trigger::TriggerKind::ExternalPlugin {
            continue;
        }
        let plugin_id = definition.plugin.as_deref().ok_or_else(|| {
            TriggerPlaneError::Contract(
                crate::errors::ContractError::InvalidTriggerDefinitionField {
                    trigger_id: definition.trigger_id.clone(),
                    field: "trigger.plugin",
                    detail: "value cannot be empty".to_owned(),
                },
            )
        })?;
        let manifest = manifests_by_id.get(plugin_id).copied().ok_or_else(|| {
            TriggerPlaneError::Contract(crate::errors::ContractError::UnknownTriggerPlugin {
                trigger_id: definition.trigger_id.clone(),
                plugin_id: plugin_id.to_owned(),
            })
        })?;
        validate_external_trigger_plugin_manifest(manifest, policy)
            .map_err(TriggerPlaneError::from)?;
    }

    Ok(())
}
