//! [INPUT]
//! Trigger package definitions, workflow bindings, state coordination, builtin trigger events, and external trigger plugin manifests.
//!
//! [OUTPUT]
//! Validates trigger packages, normalizes accepted events into run requests, and dispatches builtin or external trigger sources.
//!
//! [ROLE]
//! Implements the trigger plane that feeds workflow execution without owning DAG node dispatch.
//!
//! [INVARIANTS]
//! Accepted-event suppression stays restart-safe, and trigger validation fails before any workflow run request is emitted.


use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::errors::{assert_supported_major, ContractError};
use crate::plugin::{configure_plugin_host_environment, PluginKind, PluginManifest};
use crate::state::{
    sanitize_path_component, CoordinationError, CoordinationStore, FileBackedStateStore,
    FileStateError, StateLayout, TriggerEventRecord,
};

pub const CURRENT_API_MAJOR: u64 = 2;
pub const REQUIRED_TRIGGER_PLUGIN_CAPABILITY: &str = "trigger.emit.run_request";
pub const TRIGGER_KIND_BUILTIN: &str = "builtin";
pub const TRIGGER_KIND_MANUAL_ALIAS: &str = "manual";
pub const TRIGGER_KIND_MARKET_TICK_ALIAS: &str = "market_tick";
pub const TRIGGER_KIND_EXTERNAL_PLUGIN: &str = "external_plugin";
pub const TRIGGER_KIND_EXTERNAL_TRIGGER_ALIAS: &str = "external_trigger";
pub const TRIGGER_KIND_PLUGIN_ALIAS: &str = "plugin";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerKind {
    Builtin,
    ExternalPlugin,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TriggerDefinition {
    #[serde(rename = "manifest_version", alias = "api_version")]
    pub api_version: String,
    pub trigger_id: String,
    pub kind: String,
    pub source: String,
    #[serde(default)]
    pub plugin: Option<String>,
    pub workflow_id: String,
    pub enabled: bool,
    #[serde(default)]
    pub input_mapping: BTreeMap<String, String>,
    #[serde(skip)]
    pub package_root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TriggerEmission {
    pub event_id: String,
    pub occurred_at_ms: i64,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub payload: serde_json::Value,
    #[serde(default)]
    pub dedup_key: Option<String>,
    #[serde(default)]
    pub dedup_window_ms: Option<i64>,
    #[serde(default)]
    pub cooldown_key: Option<String>,
    #[serde(default)]
    pub cooldown_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TriggerPluginOutput {
    pub api_version: String,
    pub events: Vec<TriggerEmission>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TriggerRunRequest {
    pub run_id: String,
    pub workflow_id: String,
    pub trigger_id: String,
    pub event_id: String,
    pub source: String,
    pub accepted_at_ms: i64,
    pub payload: serde_json::Value,
    pub trigger_record_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerPluginHostPolicy {
    pub allowlisted_plugin_ids: BTreeSet<String>,
    pub allowed_capabilities: BTreeSet<String>,
    pub plugin_root_dir: PathBuf,
}

#[derive(Debug)]
pub struct TriggerPlane {
    definitions: Vec<TriggerDefinition>,
    builtin_events: BTreeMap<String, Vec<TriggerEmission>>,
    external_plugins: BTreeMap<String, ExternalTriggerPlugin>,
    state_store: FileBackedStateStore,
    coordination_store: CoordinationStore,
    accepted_sequence: u64,
    accepted_event_keys: BTreeSet<String>,
}

#[derive(Debug)]
pub enum TriggerPlaneError {
    Contract(ContractError),
    FileState(FileStateError),
    Coordination(CoordinationError),
}

#[derive(Debug, Clone)]
struct ExternalTriggerPlugin {
    plugin_id: String,
    executable_path: PathBuf,
}

impl TriggerDefinition {
    pub fn validate(&self) -> Result<(), ContractError> {
        assert_supported_major(
            "trigger.manifest_version",
            &self.api_version,
            CURRENT_API_MAJOR,
        )?;
        validate_non_empty_field(&self.trigger_id, "trigger.trigger_id", "<unknown-trigger>")?;
        validate_non_empty_field(&self.source, "trigger.source", &self.trigger_id)?;
        validate_non_empty_field(&self.workflow_id, "trigger.workflow_id", &self.trigger_id)?;
        match self.kind()? {
            TriggerKind::Builtin => {}
            TriggerKind::ExternalPlugin => validate_non_empty_field(
                self.plugin.as_deref().unwrap_or_default(),
                "trigger.plugin",
                &self.trigger_id,
            )?,
        }
        Ok(())
    }

    pub fn kind(&self) -> Result<TriggerKind, ContractError> {
        match self.kind.as_str() {
            TRIGGER_KIND_BUILTIN | TRIGGER_KIND_MANUAL_ALIAS | TRIGGER_KIND_MARKET_TICK_ALIAS => {
                Ok(TriggerKind::Builtin)
            }
            TRIGGER_KIND_EXTERNAL_PLUGIN
            | TRIGGER_KIND_EXTERNAL_TRIGGER_ALIAS
            | TRIGGER_KIND_PLUGIN_ALIAS => Ok(TriggerKind::ExternalPlugin),
            _ => Err(ContractError::UnknownTriggerKind {
                trigger_id: self.trigger_id.clone(),
                kind: self.kind.clone(),
            }),
        }
    }
}

impl TriggerPluginOutput {
    pub fn validate(&self) -> Result<(), ContractError> {
        assert_supported_major(
            "trigger_plugin_output.api_version",
            &self.api_version,
            CURRENT_API_MAJOR,
        )
    }
}

impl TriggerPlane {
    pub fn open(
        state_layout: StateLayout,
        definitions: Vec<TriggerDefinition>,
        plugin_manifests: Vec<PluginManifest>,
        policy: TriggerPluginHostPolicy,
        builtin_events: BTreeMap<String, Vec<TriggerEmission>>,
        now_ms: i64,
    ) -> Result<Self, TriggerPlaneError> {
        let state_store = FileBackedStateStore::new(state_layout.clone());
        state_store.initialize()?;
        let _ = state_store.recover_trigger_records()?;
        let trigger_records = state_store.load_trigger_records()?;
        let mut coordination_store = CoordinationStore::open(&state_layout, now_ms)?;
        coordination_store.rebuild_trigger_record_coordination(&trigger_records, now_ms)?;
        let accepted_event_keys = trigger_records
            .iter()
            .map(|record| accepted_event_key(&record.trigger_id, &record.event_id))
            .into_iter()
            .collect();

        let mut validated_definitions = Vec::with_capacity(definitions.len());
        let mut definition_ids = BTreeSet::new();
        for definition in definitions {
            definition.validate()?;
            if !definition_ids.insert(definition.trigger_id.clone()) {
                return Err(TriggerPlaneError::Contract(
                    ContractError::DuplicateTriggerId {
                        trigger_id: definition.trigger_id,
                    },
                ));
            }
            validated_definitions.push(definition);
        }

        let mut external_plugins = BTreeMap::new();
        for manifest in plugin_manifests {
            let plugin = validate_trigger_plugin_manifest(&manifest, &policy)?;
            external_plugins.insert(plugin.plugin_id.clone(), plugin);
        }

        Ok(Self {
            definitions: validated_definitions,
            builtin_events,
            external_plugins,
            state_store,
            coordination_store,
            accepted_sequence: 0,
            accepted_event_keys,
        })
    }

    pub fn collect_run_requests(
        &mut self,
        accepted_at_ms: i64,
    ) -> Result<Vec<TriggerRunRequest>, TriggerPlaneError> {
        let mut run_requests = Vec::new();

        let definitions = self.definitions.clone();
        for definition in definitions {
            if !definition.enabled {
                continue;
            }

            let emissions = match definition.kind()? {
                TriggerKind::Builtin => self
                    .builtin_events
                    .remove(&definition.trigger_id)
                    .unwrap_or_default(),
                TriggerKind::ExternalPlugin => {
                    let plugin_id = definition.plugin.as_deref().ok_or_else(|| {
                        ContractError::InvalidTriggerDefinitionField {
                            trigger_id: definition.trigger_id.clone(),
                            field: "trigger.plugin",
                            detail: "value cannot be empty".to_owned(),
                        }
                    })?;
                    let plugin = self.external_plugins.get(plugin_id).ok_or_else(|| {
                        ContractError::UnknownTriggerPlugin {
                            trigger_id: definition.trigger_id.clone(),
                            plugin_id: plugin_id.to_owned(),
                        }
                    })?;
                    plugin.emit(&definition.trigger_id)?
                }
            };

            for emission in emissions {
                let request = self.normalize_emission(&definition, emission, accepted_at_ms)?;
                if let Some(request) = request {
                    run_requests.push(request);
                }
            }
        }

        Ok(run_requests)
    }

    fn normalize_emission(
        &mut self,
        definition: &TriggerDefinition,
        emission: TriggerEmission,
        accepted_at_ms: i64,
    ) -> Result<Option<TriggerRunRequest>, TriggerPlaneError> {
        let accepted_event_key = accepted_event_key(&definition.trigger_id, &emission.event_id);
        if self.accepted_event_keys.contains(&accepted_event_key) {
            return Ok(None);
        }

        if emission.event_id.trim().is_empty() {
            return Err(TriggerPlaneError::Contract(
                ContractError::InvalidTriggerEmission {
                    trigger_id: definition.trigger_id.clone(),
                    detail: "event_id cannot be empty".to_owned(),
                },
            ));
        }
        let dedup_key = emission
            .dedup_window_ms
            .filter(|value| *value > 0)
            .map(|_| {
                emission
                    .dedup_key
                    .clone()
                    .unwrap_or_else(|| format!("{}:{}", definition.trigger_id, emission.event_id))
            });
        let dedup_expires_at_ms = emission
            .dedup_window_ms
            .filter(|value| *value > 0)
            .map(|window_ms| accepted_at_ms.saturating_add(window_ms));
        if let Some(dedup_key) = dedup_key.as_deref() {
            if !self
                .coordination_store
                .dedup_is_ready(dedup_key, accepted_at_ms)?
            {
                return Ok(None);
            }
        }

        let cooldown_key = emission.cooldown_ms.filter(|value| *value > 0).map(|_| {
            emission
                .cooldown_key
                .clone()
                .unwrap_or_else(|| definition.trigger_id.clone())
        });
        let cooldown_expires_at_ms = emission
            .cooldown_ms
            .filter(|value| *value > 0)
            .map(|cooldown_ms| accepted_at_ms.saturating_add(cooldown_ms));
        if let Some(cooldown_key) = cooldown_key.as_deref() {
            if !self
                .coordination_store
                .cooldown_is_ready(cooldown_key, accepted_at_ms)?
            {
                return Ok(None);
            }
        }

        self.accepted_sequence = self.accepted_sequence.saturating_add(1);
        let run_id = format!(
            "run-{}-{}-{}-{}-{}-{:020}",
            sanitize_path_component(&definition.workflow_id),
            sanitize_path_component(&definition.trigger_id),
            sanitize_path_component(&emission.event_id),
            emission.occurred_at_ms,
            accepted_at_ms,
            self.accepted_sequence
        );

        let source = emission
            .source
            .clone()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| definition.source.clone());
        if source.trim().is_empty() {
            return Err(TriggerPlaneError::Contract(
                ContractError::InvalidTriggerEmission {
                    trigger_id: definition.trigger_id.clone(),
                    detail: "source cannot be empty".to_owned(),
                },
            ));
        }

        let trigger_record = TriggerEventRecord {
            run_id: run_id.clone(),
            sequence: self.accepted_sequence,
            trigger_id: definition.trigger_id.clone(),
            event_id: emission.event_id.clone(),
            source: source.clone(),
            accepted_at_ms,
            dedup_key,
            dedup_expires_at_ms,
            cooldown_key,
            cooldown_expires_at_ms,
        };

        let trigger_record_path = self.state_store.write_trigger_record(&trigger_record)?;
        self.coordination_store
            .apply_trigger_record_coordination(&trigger_record, accepted_at_ms)?;
        self.accepted_event_keys.insert(accepted_event_key);

        Ok(Some(TriggerRunRequest {
            run_id,
            workflow_id: definition.workflow_id.clone(),
            trigger_id: definition.trigger_id.clone(),
            event_id: emission.event_id,
            source,
            accepted_at_ms,
            payload: map_trigger_payload(definition, &emission.payload),
            trigger_record_path,
        }))
    }
}

impl Display for TriggerPlaneError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Contract(source) => write!(f, "trigger contract error: {source}"),
            Self::FileState(source) => write!(f, "trigger state persistence error: {source}"),
            Self::Coordination(source) => write!(f, "trigger coordination error: {source}"),
        }
    }
}

impl Error for TriggerPlaneError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Contract(source) => Some(source),
            Self::FileState(source) => Some(source),
            Self::Coordination(source) => Some(source),
        }
    }
}

impl From<ContractError> for TriggerPlaneError {
    fn from(value: ContractError) -> Self {
        Self::Contract(value)
    }
}

impl From<FileStateError> for TriggerPlaneError {
    fn from(value: FileStateError) -> Self {
        Self::FileState(value)
    }
}

impl From<CoordinationError> for TriggerPlaneError {
    fn from(value: CoordinationError) -> Self {
        Self::Coordination(value)
    }
}

fn validate_trigger_plugin_manifest(
    manifest: &PluginManifest,
    policy: &TriggerPluginHostPolicy,
) -> Result<ExternalTriggerPlugin, ContractError> {
    manifest.validate()?;

    if !policy.allowlisted_plugin_ids.contains(&manifest.plugin_id) {
        return Err(ContractError::TriggerPluginNotAllowlisted {
            plugin_id: manifest.plugin_id.clone(),
        });
    }

    if manifest.kind()? != PluginKind::ExternalTrigger {
        return Err(ContractError::TriggerPluginInvalidKind {
            plugin_id: manifest.plugin_id.clone(),
            kind: manifest.kind.clone(),
        });
    }

    let required_capability = REQUIRED_TRIGGER_PLUGIN_CAPABILITY.to_owned();
    if !manifest.capabilities.contains(&required_capability) {
        return Err(ContractError::TriggerPluginMissingCapability {
            plugin_id: manifest.plugin_id.clone(),
            capability: required_capability,
        });
    }

    for capability in &manifest.capabilities {
        if !policy.allowed_capabilities.contains(capability) {
            return Err(ContractError::TriggerPluginCapabilityNotAllowed {
                plugin_id: manifest.plugin_id.clone(),
                capability: capability.clone(),
            });
        }
    }

    let executable =
        manifest
            .executable
            .as_deref()
            .ok_or_else(|| ContractError::NodePluginInvalidField {
                plugin_id: manifest.plugin_id.clone(),
                field: "plugin.executable",
                detail: "value cannot be empty".to_owned(),
            })?;

    let executable_path = resolve_executable_path(manifest, executable, &policy.plugin_root_dir)?;

    Ok(ExternalTriggerPlugin {
        plugin_id: manifest.plugin_id.clone(),
        executable_path,
    })
}

fn resolve_executable_path(
    manifest: &PluginManifest,
    entrypoint: &str,
    plugin_root_dir: &Path,
) -> Result<PathBuf, ContractError> {
    let entrypoint_path = Path::new(entrypoint);
    if entrypoint_path.is_absolute()
        || entrypoint_path
            .components()
            .any(|component| matches!(component, Component::RootDir))
    {
        return Err(ContractError::TriggerPluginEntrypointMustBeRelative {
            plugin_id: manifest.plugin_id.clone(),
            entrypoint: entrypoint.to_owned(),
        });
    }

    let manifest_root_dir = manifest
        .manifest_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(plugin_root_dir);
    let plugin_root =
        std::fs::canonicalize(plugin_root_dir).map_err(|source| ContractError::Io {
            path: plugin_root_dir.to_path_buf(),
            operation: "canonicalize trigger plugin root",
            source,
        })?;
    if entrypoint_escapes_root(manifest_root_dir, &plugin_root, entrypoint_path) {
        return Err(ContractError::TriggerPluginEntrypointEscapesRoot {
            plugin_id: manifest.plugin_id.clone(),
            entrypoint: entrypoint.to_owned(),
            root: plugin_root,
        });
    }
    let executable_path = manifest_root_dir.join(entrypoint_path);

    if !executable_path.exists() {
        return Err(ContractError::TriggerPluginExecutableMissing {
            plugin_id: manifest.plugin_id.clone(),
            path: executable_path,
        });
    }

    let executable_canonical =
        std::fs::canonicalize(&executable_path).map_err(|source| ContractError::Io {
            path: executable_path.clone(),
            operation: "canonicalize trigger plugin executable",
            source,
        })?;

    if !executable_canonical.starts_with(&plugin_root) {
        return Err(ContractError::TriggerPluginEntrypointEscapesRoot {
            plugin_id: manifest.plugin_id.clone(),
            entrypoint: entrypoint.to_owned(),
            root: plugin_root,
        });
    }

    let metadata =
        std::fs::metadata(&executable_canonical).map_err(|source| ContractError::Io {
            path: executable_canonical.clone(),
            operation: "read trigger plugin executable metadata",
            source,
        })?;
    if !metadata.is_file() {
        return Err(ContractError::TriggerPluginExecutableNotFile {
            plugin_id: manifest.plugin_id.clone(),
            path: executable_canonical,
        });
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err(ContractError::TriggerPluginExecutableNotExecutable {
                plugin_id: manifest.plugin_id.clone(),
                path: executable_canonical,
            });
        }
    }

    Ok(executable_canonical)
}

fn validate_non_empty_field(
    value: &str,
    field: &'static str,
    trigger_id: &str,
) -> Result<(), ContractError> {
    if value.trim().is_empty() {
        return Err(ContractError::InvalidTriggerDefinitionField {
            trigger_id: trigger_id.to_owned(),
            field,
            detail: "value cannot be empty".to_owned(),
        });
    }
    Ok(())
}

impl ExternalTriggerPlugin {
    fn emit(&self, trigger_id: &str) -> Result<Vec<TriggerEmission>, ContractError> {
        validate_existing_executable(&self.plugin_id, &self.executable_path)?;

        let mut command = Command::new(&self.executable_path);
        command.arg("--trigger-id").arg(trigger_id);
        configure_plugin_host_environment(&mut command);

        let output =
            command
                .output()
                .map_err(|source| ContractError::TriggerPluginSpawnFailed {
                    plugin_id: self.plugin_id.clone(),
                    path: self.executable_path.clone(),
                    source,
                })?;

        if !output.status.success() {
            return Err(ContractError::TriggerPluginProcessFailed {
                plugin_id: self.plugin_id.clone(),
                status: output.status.code().unwrap_or(-1),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            });
        }

        let decoded: TriggerPluginOutput =
            serde_json::from_slice(&output.stdout).map_err(|source| {
                ContractError::TriggerPluginOutputDecode {
                    plugin_id: self.plugin_id.clone(),
                    source,
                }
            })?;
        decoded.validate()?;
        Ok(decoded.events)
    }
}

fn accepted_event_key(trigger_id: &str, event_id: &str) -> String {
    format!("{trigger_id}:::{event_id}")
}

fn entrypoint_escapes_root(base_dir: &Path, root_dir: &Path, entrypoint_path: &Path) -> bool {
    let mut depth = match base_dir.strip_prefix(root_dir) {
        Ok(relative) => relative
            .components()
            .filter(|component| matches!(component, Component::Normal(_)))
            .count() as isize,
        Err(_) => 0,
    };

    for component in entrypoint_path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(_) => depth += 1,
            Component::ParentDir => {
                depth -= 1;
                if depth < 0 {
                    return true;
                }
            }
            Component::Prefix(_) | Component::RootDir => return true,
        }
    }

    false
}

fn map_trigger_payload(
    definition: &TriggerDefinition,
    payload: &serde_json::Value,
) -> serde_json::Value {
    if definition.input_mapping.is_empty() {
        return payload.clone();
    }

    let mut mapped = serde_json::Map::new();
    for (target, selector) in &definition.input_mapping {
        if let Some(value) = select_payload_value(payload, selector) {
            mapped.insert(target.clone(), value.clone());
        }
    }
    serde_json::Value::Object(mapped)
}

fn select_payload_value<'a>(
    payload: &'a serde_json::Value,
    selector: &str,
) -> Option<&'a serde_json::Value> {
    if selector == "payload" {
        return Some(payload);
    }
    let remainder = selector.strip_prefix("payload.")?;

    let mut current = payload;
    for segment in remainder.split('.') {
        current = current.get(segment)?;
    }
    Some(current)
}

fn validate_existing_executable(
    plugin_id: &str,
    executable_path: &Path,
) -> Result<(), ContractError> {
    if !executable_path.exists() {
        return Err(ContractError::TriggerPluginExecutableMissing {
            plugin_id: plugin_id.to_owned(),
            path: executable_path.to_path_buf(),
        });
    }

    let metadata = std::fs::metadata(executable_path).map_err(|source| ContractError::Io {
        path: executable_path.to_path_buf(),
        operation: "read trigger plugin executable metadata",
        source,
    })?;
    if !metadata.is_file() {
        return Err(ContractError::TriggerPluginExecutableNotFile {
            plugin_id: plugin_id.to_owned(),
            path: executable_path.to_path_buf(),
        });
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err(ContractError::TriggerPluginExecutableNotExecutable {
                plugin_id: plugin_id.to_owned(),
                path: executable_path.to_path_buf(),
            });
        }
    }

    Ok(())
}
