//! [INPUT]
//! Trigger package definitions, workflow bindings, params payloads, state coordination, builtin trigger events, and external trigger plugin manifests.
//!
//! [OUTPUT]
//! Validates trigger packages, normalizes accepted events into run requests, and dispatches builtin or external trigger sources with restart-safe event suppression.
//!
//! [ROLE]
//! Implements the trigger plane that feeds workflow execution without owning DAG node dispatch.
//!
//! [INVARIANTS]
//! Accepted-event suppression stays restart-safe, and trigger validation fails before any workflow run request is emitted.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::builtins::triggers::validate_builtin_trigger_definition;
use crate::errors::{assert_required_major, assert_supported_major, ContractError};
use crate::plugin::{configure_plugin_host_environment, PluginKind, PluginManifest};
use crate::state::{
    sanitize_path_component, CoordinationError, CoordinationStore, FileBackedStateStore,
    FileStateError, StateLayout, TriggerCheckpointRecord, TriggerEventRecord,
};

pub const CURRENT_API_MAJOR: u64 = 2;
pub const REQUIRED_TRIGGER_PLUGIN_CAPABILITY: &str = "trigger.listen.event";
pub const TRIGGER_KIND_BUILTIN: &str = "builtin";
pub const TRIGGER_KIND_EXTERNAL_PLUGIN: &str = "external_plugin";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerKind {
    Builtin,
    ExternalPlugin,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TriggerDefinition {
    #[serde(rename = "manifest_version")]
    pub api_version: String,
    pub trigger_id: String,
    pub kind: String,
    pub source: String,
    #[serde(default)]
    pub plugin: Option<String>,
    pub workflow_id: String,
    pub enabled: bool,
    #[serde(default)]
    pub params: BTreeMap<String, serde_json::Value>,
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
    pub checkpoint: Option<String>,
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
pub struct TriggerStartCommand {
    pub protocol_version: String,
    pub trigger_id: String,
    pub source: String,
    #[serde(default)]
    pub params: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub resume_checkpoint: Option<String>,
    pub heartbeat_interval_ms: i64,
    pub shutdown_grace_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TriggerAck {
    pub checkpoint: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TriggerStop {
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TriggerHostMessage {
    Start(TriggerStartCommand),
    Ack(TriggerAck),
    Stop(TriggerStop),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TriggerReady {
    pub protocol_version: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TriggerEventFrame {
    pub checkpoint: String,
    pub event_key: String,
    pub occurred_at_ms: i64,
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
pub struct TriggerHeartbeat {
    pub at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TriggerFatal {
    pub message: String,
    pub retryable: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TriggerPluginMessage {
    Ready(TriggerReady),
    Event(TriggerEventFrame),
    Heartbeat(TriggerHeartbeat),
    Fatal(TriggerFatal),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ListenerSessionState {
    WaitingReady,
    Active,
    Draining,
    Stopped,
}

#[derive(Debug)]
enum ListenerFrame {
    Stdout(String),
    StdoutClosed,
    StdoutError(std::io::Error),
}

impl TriggerDefinition {
    pub fn validate(&self) -> Result<(), ContractError> {
        assert_required_major(
            "trigger.manifest_version",
            &self.api_version,
            CURRENT_API_MAJOR,
        )?;
        validate_non_empty_field(&self.trigger_id, "trigger.trigger_id", "<unknown-trigger>")?;
        validate_non_empty_field(&self.source, "trigger.source", &self.trigger_id)?;
        validate_non_empty_field(&self.workflow_id, "trigger.workflow_id", &self.trigger_id)?;
        match self.kind()? {
            TriggerKind::Builtin => validate_builtin_trigger_definition(self)?,
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
            TRIGGER_KIND_BUILTIN => Ok(TriggerKind::Builtin),
            TRIGGER_KIND_EXTERNAL_PLUGIN => Ok(TriggerKind::ExternalPlugin),
            _ => Err(ContractError::UnknownTriggerKind {
                trigger_id: self.trigger_id.clone(),
                kind: self.kind.clone(),
            }),
        }
    }

    pub fn builtin_subtype(&self) -> Result<Option<&str>, ContractError> {
        match self.kind()? {
            TriggerKind::ExternalPlugin => Ok(None),
            TriggerKind::Builtin => {
                if self.kind == TRIGGER_KIND_BUILTIN {
                    Ok(Some(self.source.as_str()))
                } else {
                    Ok(Some(self.kind.as_str()))
                }
            }
        }
    }
}

impl TriggerStartCommand {
    pub fn validate(&self) -> Result<(), ContractError> {
        assert_supported_major(
            "trigger_start_command.protocol_version",
            &self.protocol_version,
            CURRENT_API_MAJOR,
        )
    }
}

impl TriggerHostMessage {
    pub fn validate(&self) -> Result<(), ContractError> {
        match self {
            Self::Start(start) => start.validate(),
            Self::Ack(_) | Self::Stop(_) => Ok(()),
        }
    }
}

impl ListenerSessionState {
    fn protocol_error(self, plugin_id: &str, detail: impl Into<String>) -> ContractError {
        ContractError::TriggerPluginProtocolContractViolation {
            plugin_id: plugin_id.to_owned(),
            detail: detail.into(),
        }
    }

    fn handle_message(
        self,
        plugin_id: &str,
        message: &TriggerPluginMessage,
    ) -> Result<Self, ContractError> {
        match (self, message) {
            (Self::WaitingReady, TriggerPluginMessage::Ready(_)) => Ok(Self::Active),
            (Self::WaitingReady, TriggerPluginMessage::Heartbeat(_)) => {
                Err(self.protocol_error(plugin_id, "received heartbeat before ready"))
            }
            (Self::WaitingReady, TriggerPluginMessage::Event(_)) => {
                Err(self.protocol_error(plugin_id, "received event before ready"))
            }
            (Self::WaitingReady, TriggerPluginMessage::Fatal(_)) => Ok(Self::Draining),
            (Self::Active, TriggerPluginMessage::Ready(_)) => {
                Err(self.protocol_error(plugin_id, "received duplicate ready message"))
            }
            (Self::Active, TriggerPluginMessage::Heartbeat(_))
            | (Self::Active, TriggerPluginMessage::Event(_)) => Ok(Self::Active),
            (Self::Active, TriggerPluginMessage::Fatal(_)) => Ok(Self::Draining),
            (Self::Draining | Self::Stopped, TriggerPluginMessage::Ready(_))
            | (Self::Draining | Self::Stopped, TriggerPluginMessage::Heartbeat(_))
            | (Self::Draining | Self::Stopped, TriggerPluginMessage::Event(_))
            | (Self::Draining | Self::Stopped, TriggerPluginMessage::Fatal(_)) => {
                Err(self.protocol_error(plugin_id, "received message after listener stopped"))
            }
        }
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
        self.collect_run_requests_with_progress(accepted_at_ms, &mut || Ok(()))
    }

    pub fn collect_run_requests_with_progress<F>(
        &mut self,
        accepted_at_ms: i64,
        on_progress: &mut F,
    ) -> Result<Vec<TriggerRunRequest>, TriggerPlaneError>
    where
        F: FnMut() -> Result<(), TriggerPlaneError>,
    {
        let mut run_requests = Vec::new();

        let definitions = self.definitions.clone();
        for definition in definitions {
            on_progress()?;
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
                    plugin.stream_emissions(&self.state_store, &definition, on_progress)?
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

        let payload = map_trigger_payload(definition, &emission.payload);
        let checkpoint = emission.checkpoint.clone();
        let trigger_record = TriggerEventRecord {
            schema_version: String::from("1.0.0"),
            run_id: run_id.clone(),
            sequence: self.accepted_sequence,
            trigger_id: definition.trigger_id.clone(),
            workflow_id: definition.workflow_id.clone(),
            event_id: emission.event_id.clone(),
            checkpoint: checkpoint.clone(),
            source: source.clone(),
            accepted_at_ms,
            payload: payload.clone(),
            dedup_key,
            dedup_expires_at_ms,
            cooldown_key,
            cooldown_expires_at_ms,
        };

        let trigger_record_path = self.state_store.write_trigger_record(&trigger_record)?;
        self.coordination_store
            .apply_trigger_record_coordination(&trigger_record, accepted_at_ms)?;
        if let Some(checkpoint) = checkpoint {
            self.state_store
                .write_trigger_checkpoint(&TriggerCheckpointRecord {
                    schema_version: String::from("1.0.0"),
                    trigger_id: definition.trigger_id.clone(),
                    checkpoint,
                    acked_at_ms: accepted_at_ms,
                })?;
        }
        self.accepted_event_keys.insert(accepted_event_key);

        Ok(Some(TriggerRunRequest {
            run_id,
            workflow_id: definition.workflow_id.clone(),
            trigger_id: definition.trigger_id.clone(),
            event_id: emission.event_id,
            source,
            accepted_at_ms,
            payload,
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

    // Canonicalize both paths to ensure they can be compared correctly.
    // manifest_root_dir may be relative (e.g., "plugins/manifests") while
    // plugin_root_dir is typically absolute. Without canonicalization,
    // entrypoint_escapes_root would fail to strip prefixes correctly.
    let manifest_root =
        std::fs::canonicalize(manifest_root_dir).map_err(|source| ContractError::Io {
            path: manifest_root_dir.to_path_buf(),
            operation: "canonicalize trigger plugin manifest root",
            source,
        })?;
    let plugin_root =
        std::fs::canonicalize(plugin_root_dir).map_err(|source| ContractError::Io {
            path: plugin_root_dir.to_path_buf(),
            operation: "canonicalize trigger plugin root",
            source,
        })?;
    if entrypoint_escapes_root(&manifest_root, &plugin_root, entrypoint_path) {
        return Err(ContractError::TriggerPluginEntrypointEscapesRoot {
            plugin_id: manifest.plugin_id.clone(),
            entrypoint: entrypoint.to_owned(),
            root: plugin_root,
        });
    }
    let executable_path = manifest_root.join(entrypoint_path);

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
    fn stream_emissions<F>(
        &self,
        state_store: &FileBackedStateStore,
        definition: &TriggerDefinition,
        on_progress: &mut F,
    ) -> Result<Vec<TriggerEmission>, ContractError>
    where
        F: FnMut() -> Result<(), TriggerPlaneError>,
    {
        validate_existing_executable(&self.plugin_id, &self.executable_path)?;

        let input = TriggerHostMessage::Start(TriggerStartCommand {
            protocol_version: String::from("2.0.0"),
            trigger_id: definition.trigger_id.clone(),
            source: definition.source.clone(),
            params: definition.params.clone(),
            resume_checkpoint: state_store
                .read_trigger_checkpoint(&definition.trigger_id)
                .map_err(|error| ContractError::InvalidTriggerEmission {
                    trigger_id: definition.trigger_id.clone(),
                    detail: error.to_string(),
                })?
                .map(|record| record.checkpoint),
            heartbeat_interval_ms: 5_000,
            shutdown_grace_ms: 10_000,
        });
        input.validate()?;
        let encoded_input = serde_json::to_string(&input).map_err(|source| {
            ContractError::TriggerPluginProtocolEncode {
                plugin_id: self.plugin_id.clone(),
                source,
            }
        })?;

        let mut command = Command::new(&self.executable_path);
        command
            .arg("--trigger-id")
            .arg(&definition.trigger_id)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        configure_plugin_host_environment(&mut command);

        let mut child =
            command
                .spawn()
                .map_err(|source| ContractError::TriggerPluginSpawnFailed {
                    plugin_id: self.plugin_id.clone(),
                    path: self.executable_path.clone(),
                    source,
                })?;

        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(encoded_input.as_bytes())
                .map_err(|source| ContractError::TriggerPluginProcessIo {
                    plugin_id: self.plugin_id.clone(),
                    operation: "write stdin",
                    source,
                })?;
            stdin
                .write_all(b"\n")
                .map_err(|source| ContractError::TriggerPluginProcessIo {
                    plugin_id: self.plugin_id.clone(),
                    operation: "write stdin delimiter",
                    source,
                })?;
            stdin
                .flush()
                .map_err(|source| ContractError::TriggerPluginProcessIo {
                    plugin_id: self.plugin_id.clone(),
                    operation: "flush stdin",
                    source,
                })?;

            let stdout =
                child
                    .stdout
                    .take()
                    .ok_or_else(|| ContractError::TriggerPluginProcessIo {
                        plugin_id: self.plugin_id.clone(),
                        operation: "capture stdout",
                        source: std::io::Error::other("missing stdout pipe"),
                    })?;
            let stderr =
                child
                    .stderr
                    .take()
                    .ok_or_else(|| ContractError::TriggerPluginProcessIo {
                        plugin_id: self.plugin_id.clone(),
                        operation: "capture stderr",
                        source: std::io::Error::other("missing stderr pipe"),
                    })?;

            let stderr_handle = thread::spawn(move || -> std::io::Result<String> {
                let mut reader = BufReader::new(stderr);
                let mut output = String::new();
                reader.read_to_string(&mut output)?;
                Ok(output)
            });

            let (stdout_tx, stdout_rx) = mpsc::channel();
            let stdout_handle = thread::spawn(move || {
                let mut reader = BufReader::new(stdout);
                loop {
                    let mut line = String::new();
                    match reader.read_line(&mut line) {
                        Ok(0) => {
                            let _ = stdout_tx.send(ListenerFrame::StdoutClosed);
                            break;
                        }
                        Ok(_) => {
                            let _ = stdout_tx.send(ListenerFrame::Stdout(line));
                        }
                        Err(source) => {
                            let _ = stdout_tx.send(ListenerFrame::StdoutError(source));
                            break;
                        }
                    }
                }
            });

            let mut emissions = Vec::new();
            let mut state = ListenerSessionState::WaitingReady;
            let heartbeat_interval_ms = input_heartbeat_interval_ms(&input);
            let mut last_activity_ms = contract_now_ms(definition)?;

            loop {
                let now_ms = contract_now_ms(definition)?;
                let timeout_ms = match state {
                    ListenerSessionState::WaitingReady => heartbeat_interval_ms,
                    ListenerSessionState::Active => {
                        let deadline = last_activity_ms.saturating_add(heartbeat_interval_ms * 2);
                        deadline
                            .saturating_sub(now_ms)
                            .clamp(1, heartbeat_interval_ms)
                    }
                    ListenerSessionState::Draining | ListenerSessionState::Stopped => 1,
                };

                match stdout_rx.recv_timeout(Duration::from_millis(timeout_ms as u64)) {
                    Ok(ListenerFrame::Stdout(line)) => {
                        on_progress().map_err(progress_to_contract_error(definition))?;
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        let message: TriggerPluginMessage =
                            serde_json::from_str(trimmed).map_err(|source| {
                                ContractError::TriggerPluginOutputDecode {
                                    plugin_id: self.plugin_id.clone(),
                                    source,
                                }
                            })?;
                        state = state.handle_message(&self.plugin_id, &message)?;
                        last_activity_ms = contract_now_ms(definition)?;

                        match message {
                            TriggerPluginMessage::Ready(_) | TriggerPluginMessage::Heartbeat(_) => {
                            }
                            TriggerPluginMessage::Fatal(fatal) => {
                                let _ = request_plugin_stop(
                                    &self.plugin_id,
                                    &mut stdin,
                                    TriggerStop {
                                        reason: String::from("plugin_fatal"),
                                    },
                                );
                                let _ = child.kill();
                                let _ = stdout_handle.join();
                                return Err(ContractError::TriggerPluginReturnedFailure {
                                    plugin_id: self.plugin_id.clone(),
                                    message: fatal.message,
                                });
                            }
                            TriggerPluginMessage::Event(event) => {
                                emissions.push(TriggerEmission {
                                    event_id: format!(
                                        "{}:{}",
                                        definition.trigger_id, event.event_key
                                    ),
                                    occurred_at_ms: event.occurred_at_ms,
                                    checkpoint: Some(event.checkpoint.clone()),
                                    source: Some(definition.source.clone()),
                                    payload: event.payload,
                                    dedup_key: event.dedup_key,
                                    dedup_window_ms: event.dedup_window_ms,
                                    cooldown_key: event.cooldown_key,
                                    cooldown_ms: event.cooldown_ms,
                                });

                                let ack = TriggerHostMessage::Ack(TriggerAck {
                                    checkpoint: event.checkpoint,
                                });
                                let encoded_ack =
                                    serde_json::to_string(&ack).map_err(|source| {
                                        ContractError::TriggerPluginProtocolEncode {
                                            plugin_id: self.plugin_id.clone(),
                                            source,
                                        }
                                    })?;
                                if let Err(source) = stdin
                                    .write_all(encoded_ack.as_bytes())
                                    .and_then(|_| stdin.write_all(b"\n"))
                                    .and_then(|_| stdin.flush())
                                {
                                    if source.kind() != std::io::ErrorKind::BrokenPipe {
                                        return Err(ContractError::TriggerPluginProcessIo {
                                            plugin_id: self.plugin_id.clone(),
                                            operation: "write ack to plugin stdin",
                                            source,
                                        });
                                    }
                                }
                            }
                        }
                    }
                    Ok(ListenerFrame::StdoutClosed) => {
                        state = ListenerSessionState::Stopped;
                        break;
                    }
                    Ok(ListenerFrame::StdoutError(source)) => {
                        return Err(ContractError::TriggerPluginProcessIo {
                            plugin_id: self.plugin_id.clone(),
                            operation: "read stdout",
                            source,
                        });
                    }
                    Err(RecvTimeoutError::Timeout) => {
                        on_progress().map_err(progress_to_contract_error(definition))?;
                        let now_ms = contract_now_ms(definition)?;
                        if heartbeat_timed_out(
                            state,
                            last_activity_ms,
                            now_ms,
                            heartbeat_interval_ms,
                        ) {
                            let _ = request_plugin_stop(
                                &self.plugin_id,
                                &mut stdin,
                                TriggerStop {
                                    reason: String::from("heartbeat_timeout"),
                                },
                            );
                            let _ = child.kill();
                            let _ = stdout_handle.join();
                            return Err(ContractError::TriggerPluginProtocolContractViolation {
                                plugin_id: self.plugin_id.clone(),
                                detail: String::from("heartbeat timed out after ready"),
                            });
                        }
                    }
                    Err(RecvTimeoutError::Disconnected) => {
                        state = ListenerSessionState::Stopped;
                        break;
                    }
                }
            }

            let _ = stdout_handle.join();

            if state == ListenerSessionState::WaitingReady {
                return Err(ContractError::TriggerPluginProtocolContractViolation {
                    plugin_id: self.plugin_id.clone(),
                    detail: String::from("listener exited before ready"),
                });
            }

            let status = child
                .wait()
                .map_err(|source| ContractError::TriggerPluginProcessIo {
                    plugin_id: self.plugin_id.clone(),
                    operation: "wait for process",
                    source,
                })?;
            let stderr = stderr_handle
                .join()
                .map_err(|_| ContractError::TriggerPluginProcessIo {
                    plugin_id: self.plugin_id.clone(),
                    operation: "join stderr reader",
                    source: std::io::Error::other("stderr reader panicked"),
                })?
                .map_err(|source| ContractError::TriggerPluginProcessIo {
                    plugin_id: self.plugin_id.clone(),
                    operation: "read stderr",
                    source,
                })?;

            if !status.success() {
                return Err(ContractError::TriggerPluginProcessFailed {
                    plugin_id: self.plugin_id.clone(),
                    status: status.code().unwrap_or(-1),
                    stderr: stderr.trim().to_owned(),
                });
            }

            return Ok(emissions);
        }

        Err(ContractError::TriggerPluginProcessIo {
            plugin_id: self.plugin_id.clone(),
            operation: "open stdin",
            source: std::io::Error::other("missing stdin pipe"),
        })
    }
}

fn input_heartbeat_interval_ms(input: &TriggerHostMessage) -> i64 {
    match input {
        TriggerHostMessage::Start(command) => command.heartbeat_interval_ms.max(1),
        TriggerHostMessage::Ack(_) | TriggerHostMessage::Stop(_) => 1,
    }
}

fn heartbeat_timed_out(
    state: ListenerSessionState,
    last_activity_ms: i64,
    now_ms: i64,
    heartbeat_interval_ms: i64,
) -> bool {
    state == ListenerSessionState::Active
        && now_ms >= last_activity_ms.saturating_add(heartbeat_interval_ms.saturating_mul(2))
}

fn contract_now_ms(definition: &TriggerDefinition) -> Result<i64, ContractError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|source| ContractError::InvalidTriggerEmission {
            trigger_id: definition.trigger_id.clone(),
            detail: format!("system time before UNIX_EPOCH: {source}"),
        })?;
    i64::try_from(duration.as_millis()).map_err(|source| ContractError::InvalidTriggerEmission {
        trigger_id: definition.trigger_id.clone(),
        detail: format!("system time overflowed i64 millis: {source}"),
    })
}

fn progress_to_contract_error<'a>(
    definition: &'a TriggerDefinition,
) -> impl FnOnce(TriggerPlaneError) -> ContractError + 'a {
    move |error| ContractError::InvalidTriggerEmission {
        trigger_id: definition.trigger_id.clone(),
        detail: error.to_string(),
    }
}

fn request_plugin_stop(
    plugin_id: &str,
    stdin: &mut dyn Write,
    stop: TriggerStop,
) -> Result<(), ContractError> {
    let encoded_stop =
        serde_json::to_string(&TriggerHostMessage::Stop(stop)).map_err(|source| {
            ContractError::TriggerPluginProtocolEncode {
                plugin_id: plugin_id.to_owned(),
                source,
            }
        })?;
    stdin
        .write_all(encoded_stop.as_bytes())
        .and_then(|_| stdin.write_all(b"\n"))
        .and_then(|_| stdin.flush())
        .map_err(|source| ContractError::TriggerPluginProcessIo {
            plugin_id: plugin_id.to_owned(),
            operation: "write stop to plugin stdin",
            source,
        })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listener_state_requires_ready_before_event() {
        let error = ListenerSessionState::WaitingReady
            .handle_message(
                "plugin-test",
                &TriggerPluginMessage::Event(TriggerEventFrame {
                    checkpoint: String::from("cp-1"),
                    event_key: String::from("evt-1"),
                    occurred_at_ms: 1,
                    payload: serde_json::json!({}),
                    dedup_key: None,
                    dedup_window_ms: None,
                    cooldown_key: None,
                    cooldown_ms: None,
                }),
            )
            .expect_err("event before ready should fail");

        assert!(matches!(
            error,
            ContractError::TriggerPluginProtocolContractViolation { .. }
        ));
    }

    #[test]
    fn listener_state_rejects_duplicate_ready() {
        let error = ListenerSessionState::Active
            .handle_message(
                "plugin-test",
                &TriggerPluginMessage::Ready(TriggerReady {
                    protocol_version: String::from("2.0.0"),
                }),
            )
            .expect_err("duplicate ready should fail");

        assert!(matches!(
            error,
            ContractError::TriggerPluginProtocolContractViolation { .. }
        ));
    }

    #[test]
    fn heartbeat_timeout_only_applies_after_ready() {
        assert!(!heartbeat_timed_out(
            ListenerSessionState::WaitingReady,
            100,
            250,
            50,
        ));
        assert!(heartbeat_timed_out(
            ListenerSessionState::Active,
            100,
            250,
            50,
        ));
    }

    #[test]
    fn entrypoint_escapes_root_within_subdirectory() {
        let root = Path::new("/tmp/chainbot-demo/plugins");
        let manifest_root = Path::new("/tmp/chainbot-demo/plugins/manifests");
        let entrypoint = Path::new("../bin/demo-trigger.sh");

        assert!(
            !entrypoint_escapes_root(manifest_root, root, entrypoint),
            "../bin/demo-trigger.sh from manifests/ should stay within plugins/"
        );
    }

    #[test]
    fn entrypoint_escapes_root_truly_escapes() {
        let root = Path::new("/tmp/chainbot-demo/plugins");
        let manifest_root = Path::new("/tmp/chainbot-demo/plugins/manifests");
        let entrypoint = Path::new("../../escaped.sh");

        assert!(
            entrypoint_escapes_root(manifest_root, root, entrypoint),
            "../../escaped.sh should escape plugins/"
        );
    }

    #[test]
    fn entrypoint_escapes_root_stays_within_manifest_dir() {
        let root = Path::new("/tmp/chainbot-demo/plugins");
        let manifest_root = Path::new("/tmp/chainbot-demo/plugins/manifests");
        let entrypoint = Path::new("bin/demo-trigger.sh");

        assert!(
            !entrypoint_escapes_root(manifest_root, root, entrypoint),
            "bin/demo-trigger.sh from manifests/ should stay within plugins/"
        );
    }
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
