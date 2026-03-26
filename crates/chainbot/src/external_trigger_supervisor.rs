//! [INPUT]
//! Trigger definitions, external trigger plugin manifests, and daemon lease ownership metadata.
//!
//! [OUTPUT]
//! Tracks daemon-owned external trigger sessions with explicit start, stop, reconcile, and wasm callback staging seams keyed by trigger ID.
//!
//! [ROLE]
//! Owns long-lived external trigger session registry state outside `TriggerPlane` acceptance semantics.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::errors::ContractError;
use crate::plugin::{PluginManifest, TriggerRuntimeLifecycle};
use crate::state::StagedTriggerEventRecord;
use crate::state_db::RuntimeStateStore;
use crate::trigger::{TriggerDefinition, TriggerKind};
use crate::trigger_wasm::{
    host_push_result_from_staged_append, map_control_flow_source_to_push_outcome,
    push_event_with_host_callback, HostPushControlFlowSource, HostPushError, HostPushErrorSource,
    HostPushOutcome, HostPushResult, TriggerPushHost, WasmGuestTransportEnvelope,
    WasmTriggerSession, WasmTriggerSessionConfig,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExternalTriggerSupervisorBudget {
    pub max_sessions_per_cycle: usize,
    pub max_polls_per_session: usize,
}

impl Default for ExternalTriggerSupervisorBudget {
    fn default() -> Self {
        Self {
            max_sessions_per_cycle: 64,
            max_polls_per_session: 1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ExternalTriggerSupervisorSettings {
    pub budget: ExternalTriggerSupervisorBudget,
    pub wasm_session: WasmTriggerSessionConfig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalTriggerPollBudget {
    pub trigger_id: String,
    pub poll_budget: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalTriggerSessionRuntime {
    Process,
    Wasm,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalTriggerSessionSpec {
    pub trigger_id: String,
    pub plugin_id: String,
    pub runtime: ExternalTriggerSessionRuntime,
    pub wasm_component: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalTriggerSessionState {
    Starting,
    Active,
    Stopping,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalTriggerPushControlState {
    Ready,
    QueueSaturated,
    BudgetExhausted,
    ShuttingDown,
    LeaseLost,
}

#[derive(Debug)]
pub struct ExternalTriggerSession {
    pub trigger_id: String,
    pub plugin_id: String,
    pub runtime: ExternalTriggerSessionRuntime,
    pub owner_id: String,
    pub state: ExternalTriggerSessionState,
    pub started_at_ms: i64,
    pub push_control_state: ExternalTriggerPushControlState,
    pub wasm_session: Option<WasmTriggerSession>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalTriggerReconcileReport {
    pub started: Vec<String>,
    pub stopped: Vec<String>,
    pub retained: Vec<String>,
}

#[derive(Debug)]
pub struct ExternalTriggerSupervisor {
    owner_id: String,
    sessions: BTreeMap<String, ExternalTriggerSession>,
    settings: ExternalTriggerSupervisorSettings,
    next_cycle_start_index: usize,
}

impl ExternalTriggerSupervisor {
    pub fn new(owner_id: impl Into<String>) -> Self {
        Self::with_settings(owner_id, ExternalTriggerSupervisorSettings::default())
    }

    pub fn with_settings(
        owner_id: impl Into<String>,
        settings: ExternalTriggerSupervisorSettings,
    ) -> Self {
        Self {
            owner_id: owner_id.into(),
            sessions: BTreeMap::new(),
            settings,
            next_cycle_start_index: 0,
        }
    }

    pub fn owner_id(&self) -> &str {
        &self.owner_id
    }

    pub fn sessions(&self) -> &BTreeMap<String, ExternalTriggerSession> {
        &self.sessions
    }

    pub fn settings(&self) -> ExternalTriggerSupervisorSettings {
        self.settings
    }

    pub fn start_session(&mut self, spec: ExternalTriggerSessionSpec, now_ms: i64) -> bool {
        let ExternalTriggerSessionSpec {
            trigger_id,
            plugin_id,
            runtime,
            wasm_component,
        } = spec;
        if self.sessions.contains_key(&trigger_id) {
            return false;
        }

        let wasm_session = matches!(runtime, ExternalTriggerSessionRuntime::Wasm).then(|| {
            WasmTriggerSession::new_with_config(
                trigger_id.clone(),
                plugin_id.clone(),
                wasm_component.unwrap_or_default(),
                now_ms,
                self.settings.wasm_session,
            )
        });

        self.sessions.insert(
            trigger_id.clone(),
            ExternalTriggerSession {
                trigger_id,
                plugin_id,
                runtime,
                owner_id: self.owner_id.clone(),
                state: ExternalTriggerSessionState::Active,
                started_at_ms: now_ms,
                push_control_state: ExternalTriggerPushControlState::Ready,
                wasm_session,
            },
        );
        true
    }

    pub fn stop_session(&mut self, trigger_id: &str) -> bool {
        self.sessions.remove(trigger_id).is_some()
    }

    pub fn record_session_turn(&mut self, trigger_id: &str, now_ms: i64) -> bool {
        if let Some(session) = self.sessions.get_mut(trigger_id) {
            if let Some(wasm_session) = session.wasm_session.as_mut() {
                wasm_session.begin_turn(now_ms);
            }
            return true;
        }

        false
    }

    pub fn record_session_turns(&mut self, now_ms: i64) {
        for session in self.sessions.values_mut() {
            if let Some(wasm_session) = session.wasm_session.as_mut() {
                wasm_session.begin_turn(now_ms);
            }
        }
    }

    pub fn execute_wasm_guest_turn(
        &mut self,
        trigger_id: &str,
        now_ms: i64,
        state_store: &mut RuntimeStateStore,
        mut decode_to_staged_record: impl FnMut(
            &ExternalTriggerSession,
            &[u8],
        )
            -> Result<StagedTriggerEventRecord, HostPushError>,
    ) -> HostPushOutcome {
        let Some(session) = self.sessions.get_mut(trigger_id) else {
            return map_control_flow_source_to_push_outcome(HostPushControlFlowSource::LeaseLost);
        };

        if let Some(source) = classify_push_control_flow_source(session, &self.owner_id) {
            return map_control_flow_source_to_push_outcome(source);
        }

        let session_view = ExternalTriggerSession {
            trigger_id: session.trigger_id.clone(),
            plugin_id: session.plugin_id.clone(),
            runtime: session.runtime,
            owner_id: session.owner_id.clone(),
            state: session.state,
            started_at_ms: session.started_at_ms,
            push_control_state: session.push_control_state,
            wasm_session: None,
        };

        let mut host = DurableStagedEventHost {
            session: &session_view,
            state_store,
            decode_to_staged_record: &mut decode_to_staged_record,
        };

        let Some(wasm_session) = session.wasm_session.as_mut() else {
            return map_control_flow_source_to_push_outcome(HostPushControlFlowSource::LeaseLost);
        };

        wasm_session
            .execute_guest_turn(now_ms, &mut host)
            .unwrap_or_else(|_| {
                map_control_flow_source_to_push_outcome(
                    HostPushControlFlowSource::DaemonShuttingDown,
                )
            })
    }

    pub fn stage_wasm_guest_turn(
        &mut self,
        trigger_id: &str,
        definition: &TriggerDefinition,
        now_ms: i64,
        state_store: &mut RuntimeStateStore,
    ) -> HostPushOutcome {
        self.execute_wasm_guest_turn(trigger_id, now_ms, state_store, |session, event_bytes| {
            decode_wasm_guest_event_to_staged_record(session, definition, event_bytes, now_ms)
        })
    }

    pub fn plan_process_polls_for_cycle(&mut self, _now_ms: i64) -> Vec<ExternalTriggerPollBudget> {
        let session_ids = self.sessions.keys().cloned().collect::<Vec<_>>();
        let total_sessions = session_ids.len();
        if total_sessions == 0 {
            self.next_cycle_start_index = 0;
            return Vec::new();
        }

        let max_sessions = self
            .settings
            .budget
            .max_sessions_per_cycle
            .min(total_sessions);
        let poll_budget = self.settings.budget.max_polls_per_session;
        let cycle_start = self.next_cycle_start_index % total_sessions;
        let active_ids = if max_sessions == 0 || poll_budget == 0 {
            Vec::new()
        } else {
            (0..max_sessions)
                .map(|index| {
                    let offset = (cycle_start + index) % total_sessions;
                    session_ids[offset].clone()
                })
                .collect::<Vec<_>>()
        };
        self.next_cycle_start_index = (cycle_start + max_sessions) % total_sessions;

        let active_lookup = active_ids
            .iter()
            .map(String::as_str)
            .collect::<std::collections::BTreeSet<_>>();
        for (trigger_id, session) in &mut self.sessions {
            if session_is_lease_lost(session, &self.owner_id) {
                session.push_control_state = ExternalTriggerPushControlState::LeaseLost;
                continue;
            }

            if !matches!(
                session.push_control_state,
                ExternalTriggerPushControlState::Ready
                    | ExternalTriggerPushControlState::BudgetExhausted
            ) {
                continue;
            }

            if active_lookup.contains(trigger_id.as_str()) {
                session.push_control_state = ExternalTriggerPushControlState::Ready;
            } else {
                session.push_control_state = ExternalTriggerPushControlState::BudgetExhausted;
            }
        }

        active_ids
            .into_iter()
            .filter_map(|trigger_id| {
                let session = self.sessions.get(&trigger_id)?;
                if session.runtime != ExternalTriggerSessionRuntime::Process {
                    return None;
                }
                Some(ExternalTriggerPollBudget {
                    trigger_id,
                    poll_budget,
                })
            })
            .collect()
    }

    pub fn set_push_control_state(
        &mut self,
        trigger_id: &str,
        control_state: ExternalTriggerPushControlState,
    ) -> bool {
        let Some(session) = self.sessions.get_mut(trigger_id) else {
            return false;
        };
        session.push_control_state = control_state;
        true
    }

    pub fn push_wasm_guest_event<F>(
        &mut self,
        trigger_id: &str,
        event_bytes: &[u8],
        state_store: &mut RuntimeStateStore,
        mut decode_to_staged_record: F,
    ) -> HostPushOutcome
    where
        F: FnMut(&ExternalTriggerSession, &[u8]) -> Result<StagedTriggerEventRecord, HostPushError>,
    {
        let Some(session) = self.sessions.get(trigger_id) else {
            return map_control_flow_source_to_push_outcome(HostPushControlFlowSource::LeaseLost);
        };

        if let Some(source) = classify_push_control_flow_source(session, &self.owner_id) {
            return map_control_flow_source_to_push_outcome(source);
        }

        let mut host = DurableStagedEventHost {
            session,
            state_store,
            decode_to_staged_record: &mut decode_to_staged_record,
        };
        push_event_with_host_callback(&mut host, event_bytes)
    }

    pub fn reconcile(
        &mut self,
        desired: BTreeMap<String, ExternalTriggerSessionSpec>,
        now_ms: i64,
    ) -> ExternalTriggerReconcileReport {
        let existing_ids = self.sessions.keys().cloned().collect::<Vec<_>>();
        let mut report = ExternalTriggerReconcileReport {
            started: Vec::new(),
            stopped: Vec::new(),
            retained: Vec::new(),
        };

        for trigger_id in existing_ids {
            let should_stop_for_disable = !desired.contains_key(&trigger_id);
            let should_stop_for_lease_loss = self
                .sessions
                .get(&trigger_id)
                .is_some_and(|session| session_is_lease_lost(session, &self.owner_id));

            if should_stop_for_lease_loss {
                if let Some(session) = self.sessions.get_mut(&trigger_id) {
                    session.state = ExternalTriggerSessionState::Stopping;
                    session.push_control_state = ExternalTriggerPushControlState::LeaseLost;
                }
            }

            if (should_stop_for_disable || should_stop_for_lease_loss)
                && self.stop_session(&trigger_id)
            {
                report.stopped.push(trigger_id);
            }
        }

        for (trigger_id, spec) in desired {
            if let Some(session) = self.sessions.get_mut(&trigger_id) {
                if session_matches_spec(session, &spec, &self.owner_id) {
                    if let Some(wasm_session) = session.wasm_session.as_mut() {
                        wasm_session.mark_reconciled(now_ms);
                    }
                    report.retained.push(trigger_id);
                    continue;
                }
            }

            if self.stop_session(&trigger_id) {
                report.stopped.push(trigger_id.clone());
            }
            if self.start_session(spec, now_ms) {
                report.started.push(trigger_id);
            }
        }

        report
    }
}

fn session_matches_spec(
    session: &ExternalTriggerSession,
    spec: &ExternalTriggerSessionSpec,
    expected_owner_id: &str,
) -> bool {
    if session_is_lease_lost(session, expected_owner_id) {
        return false;
    }

    if session.plugin_id != spec.plugin_id || session.runtime != spec.runtime {
        return false;
    }

    if session.runtime != ExternalTriggerSessionRuntime::Wasm {
        return true;
    }

    let expected_component = spec.wasm_component.as_deref().unwrap_or_default();
    session
        .wasm_session
        .as_ref()
        .map(WasmTriggerSession::component)
        == Some(expected_component)
}

fn session_is_lease_lost(session: &ExternalTriggerSession, expected_owner_id: &str) -> bool {
    session.owner_id != expected_owner_id
        || session.push_control_state == ExternalTriggerPushControlState::LeaseLost
}

fn classify_push_control_flow_source(
    session: &ExternalTriggerSession,
    expected_owner_id: &str,
) -> Option<HostPushControlFlowSource> {
    if session.owner_id != expected_owner_id {
        return Some(HostPushControlFlowSource::LeaseLost);
    }
    if session.runtime != ExternalTriggerSessionRuntime::Wasm {
        return Some(HostPushControlFlowSource::LeaseLost);
    }

    let state_source = if session.state == ExternalTriggerSessionState::Stopping {
        Some(HostPushControlFlowSource::DaemonShuttingDown)
    } else {
        None
    };
    state_source.or_else(|| match session.push_control_state {
        ExternalTriggerPushControlState::Ready => None,
        ExternalTriggerPushControlState::QueueSaturated => {
            Some(HostPushControlFlowSource::QueueSaturated)
        }
        ExternalTriggerPushControlState::BudgetExhausted => {
            Some(HostPushControlFlowSource::BudgetExhausted)
        }
        ExternalTriggerPushControlState::ShuttingDown => {
            Some(HostPushControlFlowSource::DaemonShuttingDown)
        }
        ExternalTriggerPushControlState::LeaseLost => Some(HostPushControlFlowSource::LeaseLost),
    })
}

fn decode_wasm_guest_event_to_staged_record(
    session: &ExternalTriggerSession,
    definition: &TriggerDefinition,
    event_bytes: &[u8],
    staged_at_ms: i64,
) -> Result<StagedTriggerEventRecord, HostPushError> {
    let envelope: WasmGuestTransportEnvelope =
        serde_json::from_slice(event_bytes).map_err(|_| HostPushError::ShuttingDown)?;

    Ok(StagedTriggerEventRecord {
        schema_version: String::from("1.0.0"),
        staging_id: format!(
            "wasm:{}:{}:{}",
            session.trigger_id, envelope.event_key, staged_at_ms
        ),
        trigger_id: session.trigger_id.clone(),
        workflow_id: definition.workflow_id.clone(),
        event_id: format!("{}:{}", session.trigger_id, envelope.event_key),
        source: definition.source.clone(),
        occurred_at_ms: envelope.occurred_at_ms,
        staged_at_ms,
        checkpoint: envelope.checkpoint,
        payload: map_definition_input_payload(definition, &envelope.payload),
        dedup_key: envelope.dedup_key,
        dedup_window_ms: envelope.dedup_window_ms,
        cooldown_key: envelope.cooldown_key,
        cooldown_ms: envelope.cooldown_ms,
        accepted_at_ms: None,
        last_error: None,
    })
}

fn map_definition_input_payload(
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

fn select_payload_value<'a>(payload: &'a serde_json::Value, selector: &str) -> Option<&'a serde_json::Value> {
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

struct DurableStagedEventHost<'a, F> {
    session: &'a ExternalTriggerSession,
    state_store: &'a mut RuntimeStateStore,
    decode_to_staged_record: &'a mut F,
}

impl<F> TriggerPushHost for DurableStagedEventHost<'_, F>
where
    F: FnMut(&ExternalTriggerSession, &[u8]) -> Result<StagedTriggerEventRecord, HostPushError>,
{
    fn push_trigger_event(&mut self, event_bytes: &[u8]) -> HostPushResult {
        let staged_record = (self.decode_to_staged_record)(self.session, event_bytes)?;
        let session_state = self.session.state;
        host_push_result_from_staged_append(
            self.state_store
                .append_staged_trigger_event_record(&staged_record),
            |_| {
                if session_state == ExternalTriggerSessionState::Stopping {
                    HostPushErrorSource::ShuttingDown
                } else {
                    HostPushErrorSource::Backpressure
                }
            },
        )
    }
}

pub fn build_desired_external_trigger_sessions(
    definitions: &[TriggerDefinition],
    plugin_manifests: &[PluginManifest],
) -> Result<BTreeMap<String, ExternalTriggerSessionSpec>, ContractError> {
    let plugins_by_id = plugin_manifests
        .iter()
        .map(|manifest| (manifest.plugin_id.clone(), manifest))
        .collect::<BTreeMap<_, _>>();

    let mut desired = BTreeMap::new();
    for definition in definitions {
        if !definition.enabled || definition.kind()? != TriggerKind::ExternalPlugin {
            continue;
        }

        let plugin_id = definition.plugin.as_deref().ok_or_else(|| {
            ContractError::InvalidTriggerDefinitionField {
                trigger_id: definition.trigger_id.clone(),
                field: "trigger.plugin",
                detail: "value cannot be empty".to_owned(),
            }
        })?;

        let plugin =
            plugins_by_id
                .get(plugin_id)
                .ok_or_else(|| ContractError::UnknownTriggerPlugin {
                    trigger_id: definition.trigger_id.clone(),
                    plugin_id: plugin_id.to_owned(),
                })?;

        let runtime = match plugin
            .trigger_runtime
            .as_ref()
            .and_then(|runtime| runtime.lifecycle)
        {
            Some(TriggerRuntimeLifecycle::WasmDaemonPersistentSession) => {
                ExternalTriggerSessionRuntime::Wasm
            }
            _ => ExternalTriggerSessionRuntime::Process,
        };

        let wasm_component = resolve_wasm_module_for_session(plugin)?;

        desired.insert(
            definition.trigger_id.clone(),
            ExternalTriggerSessionSpec {
                trigger_id: definition.trigger_id.clone(),
                plugin_id: plugin_id.to_owned(),
                runtime,
                wasm_component,
            },
        );
    }

    Ok(desired)
}

fn resolve_wasm_module_for_session(
    plugin: &PluginManifest,
) -> Result<Option<String>, ContractError> {
    let runtime = match plugin.trigger_runtime.as_ref() {
        Some(runtime) => runtime,
        None => return Ok(None),
    };

    if runtime.lifecycle != Some(TriggerRuntimeLifecycle::WasmDaemonPersistentSession) {
        return Ok(runtime.module.clone());
    }

    let module = runtime.module.clone();
    let Some(module_value) = module.as_deref() else {
        return Ok(module);
    };

    let module_path = Path::new(module_value);
    if module_path.is_absolute() {
        return canonicalize_wasm_module_path(plugin, module_path.to_path_buf(), module_value)
            .map(Some);
    }

    let manifest_root = plugin
        .manifest_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .map(Path::to_path_buf);
    let Some(manifest_root) = manifest_root else {
        return Ok(module);
    };

    canonicalize_wasm_module_path(plugin, manifest_root.join(module_path), module_value).map(Some)
}

fn canonicalize_wasm_module_path(
    plugin: &PluginManifest,
    candidate: PathBuf,
    module_value: &str,
) -> Result<String, ContractError> {
    let canonical_candidate =
        fs::canonicalize(&candidate).map_err(|source| ContractError::NodePluginInvalidField {
            plugin_id: plugin.plugin_id.clone(),
            field: "plugin.trigger_runtime.module",
            detail: format!(
                "failed to resolve wasm module path {module_value} for plugin {}: {source}",
                plugin.plugin_id
            ),
        })?;

    Ok(canonical_candidate.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::config::{RuntimeStorageBackend, RuntimeStorageConfig};

    #[test]
    fn stop_session_removes_existing_trigger_session() {
        let mut supervisor = ExternalTriggerSupervisor::new("daemon-owner");
        let started = supervisor.start_session(
            ExternalTriggerSessionSpec {
                trigger_id: String::from("tr-external"),
                plugin_id: String::from("plugin-external"),
                runtime: ExternalTriggerSessionRuntime::Process,
                wasm_component: None,
            },
            100,
        );
        assert!(started);

        assert!(supervisor.stop_session("tr-external"));
        assert!(!supervisor.stop_session("tr-external"));
        assert!(supervisor.sessions().is_empty());
    }

    #[test]
    fn reconcile_stops_removed_sessions_and_starts_new_sessions() {
        let mut supervisor = ExternalTriggerSupervisor::new("daemon-owner");
        let started = supervisor.start_session(
            ExternalTriggerSessionSpec {
                trigger_id: String::from("tr-old"),
                plugin_id: String::from("plugin-old"),
                runtime: ExternalTriggerSessionRuntime::Process,
                wasm_component: None,
            },
            100,
        );
        assert!(started);

        let mut desired = BTreeMap::new();
        desired.insert(
            String::from("tr-new"),
            ExternalTriggerSessionSpec {
                trigger_id: String::from("tr-new"),
                plugin_id: String::from("plugin-new"),
                runtime: ExternalTriggerSessionRuntime::Wasm,
                wasm_component: Some(String::from("trigger_wasm_component")),
            },
        );

        let report = supervisor.reconcile(desired, 200);
        assert_eq!(report.started, vec![String::from("tr-new")]);
        assert_eq!(report.stopped, vec![String::from("tr-old")]);
        assert!(report.retained.is_empty());

        let session = supervisor
            .sessions()
            .get("tr-new")
            .expect("new session should be tracked");
        assert_eq!(session.owner_id, "daemon-owner");
        assert_eq!(session.runtime, ExternalTriggerSessionRuntime::Wasm);
        assert!(session.wasm_session.is_some());
    }

    #[test]
    fn wasm_session_reuses_same_runtime_across_repeated_turns() {
        let mut supervisor = ExternalTriggerSupervisor::new("daemon-owner");
        let started = supervisor.start_session(
            ExternalTriggerSessionSpec {
                trigger_id: String::from("tr-wasm"),
                plugin_id: String::from("plugin-wasm"),
                runtime: ExternalTriggerSessionRuntime::Wasm,
                wasm_component: Some(String::from("trigger_wasm_component")),
            },
            100,
        );
        assert!(started);

        let first_handles = supervisor
            .sessions()
            .get("tr-wasm")
            .and_then(|session| session.wasm_session.as_ref())
            .expect("wasm runtime should be initialized")
            .runtime_handles();

        assert!(supervisor.record_session_turn("tr-wasm", 110));
        assert!(supervisor.record_session_turn("tr-wasm", 120));

        let session = supervisor
            .sessions()
            .get("tr-wasm")
            .expect("session should remain active across turns");
        let runtime = session
            .wasm_session
            .as_ref()
            .expect("wasm runtime must stay attached while retained");

        assert_eq!(runtime.runtime_handles(), first_handles);
        assert_eq!(runtime.store_turn_count(), 2);
        assert_eq!(runtime.guest_state().turn_count, 2);
        assert_eq!(runtime.guest_state().last_turn_at_ms, Some(120));
    }

    #[test]
    fn reconcile_stop_releases_wasm_runtime_before_restart() {
        let mut supervisor = ExternalTriggerSupervisor::new("daemon-owner");
        let initial_spec = ExternalTriggerSessionSpec {
            trigger_id: String::from("tr-wasm"),
            plugin_id: String::from("plugin-wasm"),
            runtime: ExternalTriggerSessionRuntime::Wasm,
            wasm_component: Some(String::from("trigger_wasm_component")),
        };

        assert!(supervisor.start_session(initial_spec.clone(), 100));
        let old_runtime_probe = supervisor
            .sessions()
            .get("tr-wasm")
            .and_then(|session| session.wasm_session.as_ref())
            .expect("wasm runtime should exist before stop")
            .runtime_drop_probe();

        let report = supervisor.reconcile(BTreeMap::new(), 200);
        assert_eq!(report.stopped, vec![String::from("tr-wasm")]);
        assert!(!supervisor.sessions().contains_key("tr-wasm"));
        assert!(old_runtime_probe.upgrade().is_none());

        let mut desired = BTreeMap::new();
        desired.insert(String::from("tr-wasm"), initial_spec);
        let restart_report = supervisor.reconcile(desired, 300);
        assert_eq!(restart_report.started, vec![String::from("tr-wasm")]);

        let new_probe = supervisor
            .sessions()
            .get("tr-wasm")
            .and_then(|session| session.wasm_session.as_ref())
            .expect("wasm runtime should exist after restart")
            .runtime_drop_probe();
        assert!(new_probe.upgrade().is_some());
    }

    #[test]
    fn wasm_guest_push_returns_durable_ack_only_after_staged_append() {
        let mut supervisor = ExternalTriggerSupervisor::new("daemon-owner");
        assert!(supervisor.start_session(
            ExternalTriggerSessionSpec {
                trigger_id: String::from("tr-wasm"),
                plugin_id: String::from("plugin-wasm"),
                runtime: ExternalTriggerSessionRuntime::Wasm,
                wasm_component: Some(String::from("trigger_wasm_component")),
            },
            100,
        ));

        let sqlite_path = unique_sqlite_path("guest-push-durable-ack");
        let storage_config = RuntimeStorageConfig {
            backend: RuntimeStorageBackend::Local {
                database_path: sqlite_path.clone(),
            },
            history_retention: None,
            raw_debug_enabled: false,
            raw_debug_artifacts_dir: None,
        };
        let mut state_store = RuntimeStateStore::open(&storage_config, 110)
            .expect("runtime state store should open for durable staging");

        let outcome = supervisor.push_wasm_guest_event(
            "tr-wasm",
            b"transport-envelope",
            &mut state_store,
            |session, event_bytes| {
                Ok(StagedTriggerEventRecord {
                    schema_version: String::from("1.0.0"),
                    staging_id: String::from("staging-1"),
                    trigger_id: session.trigger_id.clone(),
                    workflow_id: String::from("wf-alpha"),
                    event_id: String::from("evt-1"),
                    source: String::from("external.plugin"),
                    occurred_at_ms: 110,
                    staged_at_ms: 111,
                    checkpoint: Some(String::from("cp-1")),
                    payload: serde_json::json!({
                        "event_bytes": String::from_utf8_lossy(event_bytes).to_string()
                    }),
                    dedup_key: Some(String::from("dedup-1")),
                    dedup_window_ms: Some(15_000),
                    cooldown_key: Some(String::from("cooldown-1")),
                    cooldown_ms: Some(8_000),
                    accepted_at_ms: None,
                    last_error: None,
                })
            },
        );
        assert_eq!(outcome, HostPushOutcome::DurableAck);

        let staged_rows = state_store
            .list_pending_staged_trigger_event_records("tr-wasm", 10)
            .expect("pending staged rows should be queryable");
        assert_eq!(staged_rows.len(), 1);
        assert_eq!(staged_rows[0].staging_id, "staging-1");
        assert_eq!(staged_rows[0].event_id, "evt-1");

        drop(state_store);
        let _ = fs::remove_file(sqlite_path);
    }

    #[test]
    fn wasm_guest_push_does_not_ack_when_staged_append_is_not_persisted() {
        let mut supervisor = ExternalTriggerSupervisor::new("daemon-owner");
        assert!(supervisor.start_session(
            ExternalTriggerSessionSpec {
                trigger_id: String::from("tr-wasm"),
                plugin_id: String::from("plugin-wasm"),
                runtime: ExternalTriggerSessionRuntime::Wasm,
                wasm_component: Some(String::from("trigger_wasm_component")),
            },
            100,
        ));

        let sqlite_path = unique_sqlite_path("guest-push-no-false-ack");
        let storage_config = RuntimeStorageConfig {
            backend: RuntimeStorageBackend::Local {
                database_path: sqlite_path.clone(),
            },
            history_retention: None,
            raw_debug_enabled: false,
            raw_debug_artifacts_dir: None,
        };
        let mut state_store = RuntimeStateStore::open(&storage_config, 120)
            .expect("runtime state store should open for durable staging");

        let first_outcome = supervisor.push_wasm_guest_event(
            "tr-wasm",
            b"transport-envelope-1",
            &mut state_store,
            |session, _event_bytes| {
                Ok(StagedTriggerEventRecord {
                    schema_version: String::from("1.0.0"),
                    staging_id: String::from("staging-first"),
                    trigger_id: session.trigger_id.clone(),
                    workflow_id: String::from("wf-alpha"),
                    event_id: String::from("evt-shared"),
                    source: String::from("external.plugin"),
                    occurred_at_ms: 120,
                    staged_at_ms: 121,
                    checkpoint: None,
                    payload: serde_json::json!({"ordinal": 1}),
                    dedup_key: None,
                    dedup_window_ms: None,
                    cooldown_key: None,
                    cooldown_ms: None,
                    accepted_at_ms: None,
                    last_error: None,
                })
            },
        );
        assert_eq!(first_outcome, HostPushOutcome::DurableAck);

        let second_outcome = supervisor.push_wasm_guest_event(
            "tr-wasm",
            b"transport-envelope-2",
            &mut state_store,
            |session, _event_bytes| {
                Ok(StagedTriggerEventRecord {
                    schema_version: String::from("1.0.0"),
                    staging_id: String::from("staging-second"),
                    trigger_id: session.trigger_id.clone(),
                    workflow_id: String::from("wf-alpha"),
                    event_id: String::from("evt-shared"),
                    source: String::from("external.plugin"),
                    occurred_at_ms: 122,
                    staged_at_ms: 123,
                    checkpoint: None,
                    payload: serde_json::json!({"ordinal": 2}),
                    dedup_key: None,
                    dedup_window_ms: None,
                    cooldown_key: None,
                    cooldown_ms: None,
                    accepted_at_ms: None,
                    last_error: None,
                })
            },
        );
        assert_eq!(second_outcome, HostPushOutcome::RetryableBackpressure);

        let staged_rows = state_store
            .list_pending_staged_trigger_event_records("tr-wasm", 10)
            .expect("pending staged rows should remain queryable");
        assert_eq!(staged_rows.len(), 1);
        assert_eq!(staged_rows[0].staging_id, "staging-first");

        drop(state_store);
        let _ = fs::remove_file(sqlite_path);
    }

    #[test]
    fn wasm_guest_push_returns_terminal_lease_lost_when_session_is_missing() {
        let mut supervisor = ExternalTriggerSupervisor::new("daemon-owner");
        let sqlite_path = unique_sqlite_path("guest-push-lease-lost-missing-session");
        let storage_config = RuntimeStorageConfig {
            backend: RuntimeStorageBackend::Local {
                database_path: sqlite_path.clone(),
            },
            history_retention: None,
            raw_debug_enabled: false,
            raw_debug_artifacts_dir: None,
        };
        let mut state_store = RuntimeStateStore::open(&storage_config, 120)
            .expect("runtime state store should open for callback outcome tests");

        let outcome = supervisor.push_wasm_guest_event(
            "tr-missing",
            b"transport-envelope",
            &mut state_store,
            |_session, _event_bytes| {
                panic!("decode callback should not run when no leased wasm session is present")
            },
        );

        assert_eq!(outcome, HostPushOutcome::TerminalLeaseLost);

        drop(state_store);
        let _ = fs::remove_file(sqlite_path);
    }

    #[test]
    fn wasm_guest_push_returns_terminal_shutting_down_when_session_is_stopping() {
        let mut supervisor = ExternalTriggerSupervisor::new("daemon-owner");
        assert!(supervisor.start_session(
            ExternalTriggerSessionSpec {
                trigger_id: String::from("tr-wasm"),
                plugin_id: String::from("plugin-wasm"),
                runtime: ExternalTriggerSessionRuntime::Wasm,
                wasm_component: Some(String::from("trigger_wasm_component")),
            },
            100,
        ));
        supervisor
            .sessions
            .get_mut("tr-wasm")
            .expect("session should exist before stopping")
            .state = ExternalTriggerSessionState::Stopping;

        let sqlite_path = unique_sqlite_path("guest-push-shutting-down-session");
        let storage_config = RuntimeStorageConfig {
            backend: RuntimeStorageBackend::Local {
                database_path: sqlite_path.clone(),
            },
            history_retention: None,
            raw_debug_enabled: false,
            raw_debug_artifacts_dir: None,
        };
        let mut state_store = RuntimeStateStore::open(&storage_config, 120)
            .expect("runtime state store should open for callback outcome tests");

        let outcome = supervisor.push_wasm_guest_event(
            "tr-wasm",
            b"transport-envelope",
            &mut state_store,
            |_session, _event_bytes| {
                panic!("decode callback should not run when session is shutting down")
            },
        );

        assert_eq!(outcome, HostPushOutcome::TerminalShuttingDown);

        drop(state_store);
        let _ = fs::remove_file(sqlite_path);
    }

    #[test]
    fn wasm_guest_push_returns_retryable_backpressure_when_session_queue_is_saturated() {
        let mut supervisor = ExternalTriggerSupervisor::new("daemon-owner");
        assert!(supervisor.start_session(
            ExternalTriggerSessionSpec {
                trigger_id: String::from("tr-wasm"),
                plugin_id: String::from("plugin-wasm"),
                runtime: ExternalTriggerSessionRuntime::Wasm,
                wasm_component: Some(String::from("trigger_wasm_component")),
            },
            100,
        ));
        assert!(supervisor
            .set_push_control_state("tr-wasm", ExternalTriggerPushControlState::QueueSaturated,));

        let sqlite_path = unique_sqlite_path("guest-push-queue-saturated");
        let storage_config = RuntimeStorageConfig {
            backend: RuntimeStorageBackend::Local {
                database_path: sqlite_path.clone(),
            },
            history_retention: None,
            raw_debug_enabled: false,
            raw_debug_artifacts_dir: None,
        };
        let mut state_store = RuntimeStateStore::open(&storage_config, 120)
            .expect("runtime state store should open for callback outcome tests");

        let outcome = supervisor.push_wasm_guest_event(
            "tr-wasm",
            b"transport-envelope",
            &mut state_store,
            |_session, _event_bytes| {
                panic!("decode callback should not run when queue is saturated")
            },
        );

        assert_eq!(outcome, HostPushOutcome::RetryableBackpressure);

        drop(state_store);
        let _ = fs::remove_file(sqlite_path);
    }

    #[test]
    fn wasm_guest_push_returns_retryable_backpressure_when_session_budget_is_exhausted() {
        let mut supervisor = ExternalTriggerSupervisor::new("daemon-owner");
        assert!(supervisor.start_session(
            ExternalTriggerSessionSpec {
                trigger_id: String::from("tr-wasm"),
                plugin_id: String::from("plugin-wasm"),
                runtime: ExternalTriggerSessionRuntime::Wasm,
                wasm_component: Some(String::from("trigger_wasm_component")),
            },
            100,
        ));
        assert!(supervisor
            .set_push_control_state("tr-wasm", ExternalTriggerPushControlState::BudgetExhausted,));

        let sqlite_path = unique_sqlite_path("guest-push-budget-exhausted");
        let storage_config = RuntimeStorageConfig {
            backend: RuntimeStorageBackend::Local {
                database_path: sqlite_path.clone(),
            },
            history_retention: None,
            raw_debug_enabled: false,
            raw_debug_artifacts_dir: None,
        };
        let mut state_store = RuntimeStateStore::open(&storage_config, 120)
            .expect("runtime state store should open for callback outcome tests");

        let outcome = supervisor.push_wasm_guest_event(
            "tr-wasm",
            b"transport-envelope",
            &mut state_store,
            |_session, _event_bytes| {
                panic!("decode callback should not run when callback budget is exhausted")
            },
        );

        assert_eq!(outcome, HostPushOutcome::RetryableBackpressure);

        drop(state_store);
        let _ = fs::remove_file(sqlite_path);
    }

    #[test]
    fn wasm_guest_push_returns_terminal_shutting_down_when_control_state_marks_shutdown() {
        let mut supervisor = ExternalTriggerSupervisor::new("daemon-owner");
        assert!(supervisor.start_session(
            ExternalTriggerSessionSpec {
                trigger_id: String::from("tr-wasm"),
                plugin_id: String::from("plugin-wasm"),
                runtime: ExternalTriggerSessionRuntime::Wasm,
                wasm_component: Some(String::from("trigger_wasm_component")),
            },
            100,
        ));
        assert!(supervisor
            .set_push_control_state("tr-wasm", ExternalTriggerPushControlState::ShuttingDown,));

        let sqlite_path = unique_sqlite_path("guest-push-shutdown-control-state");
        let storage_config = RuntimeStorageConfig {
            backend: RuntimeStorageBackend::Local {
                database_path: sqlite_path.clone(),
            },
            history_retention: None,
            raw_debug_enabled: false,
            raw_debug_artifacts_dir: None,
        };
        let mut state_store = RuntimeStateStore::open(&storage_config, 120)
            .expect("runtime state store should open for callback outcome tests");

        let outcome = supervisor.push_wasm_guest_event(
            "tr-wasm",
            b"transport-envelope",
            &mut state_store,
            |_session, _event_bytes| {
                panic!("decode callback should not run when session is shutting down")
            },
        );

        assert_eq!(outcome, HostPushOutcome::TerminalShuttingDown);

        drop(state_store);
        let _ = fs::remove_file(sqlite_path);
    }

    #[test]
    fn wasm_guest_push_returns_terminal_lease_lost_when_control_state_marks_lease_loss() {
        let mut supervisor = ExternalTriggerSupervisor::new("daemon-owner");
        assert!(supervisor.start_session(
            ExternalTriggerSessionSpec {
                trigger_id: String::from("tr-wasm"),
                plugin_id: String::from("plugin-wasm"),
                runtime: ExternalTriggerSessionRuntime::Wasm,
                wasm_component: Some(String::from("trigger_wasm_component")),
            },
            100,
        ));
        assert!(supervisor
            .set_push_control_state("tr-wasm", ExternalTriggerPushControlState::LeaseLost,));

        let sqlite_path = unique_sqlite_path("guest-push-lease-lost-control-state");
        let storage_config = RuntimeStorageConfig {
            backend: RuntimeStorageBackend::Local {
                database_path: sqlite_path.clone(),
            },
            history_retention: None,
            raw_debug_enabled: false,
            raw_debug_artifacts_dir: None,
        };
        let mut state_store = RuntimeStateStore::open(&storage_config, 120)
            .expect("runtime state store should open for callback outcome tests");

        let outcome = supervisor.push_wasm_guest_event(
            "tr-wasm",
            b"transport-envelope",
            &mut state_store,
            |_session, _event_bytes| {
                panic!("decode callback should not run when lease ownership is lost")
            },
        );

        assert_eq!(outcome, HostPushOutcome::TerminalLeaseLost);

        drop(state_store);
        let _ = fs::remove_file(sqlite_path);
    }

    #[test]
    fn reconcile_stops_disabled_trigger_sessions_and_removes_registry_entry() {
        let mut supervisor = ExternalTriggerSupervisor::new("daemon-owner");
        assert!(supervisor.start_session(
            ExternalTriggerSessionSpec {
                trigger_id: String::from("tr-disabled"),
                plugin_id: String::from("plugin-process"),
                runtime: ExternalTriggerSessionRuntime::Process,
                wasm_component: None,
            },
            100,
        ));

        let report = supervisor.reconcile(BTreeMap::new(), 200);
        assert_eq!(report.stopped, vec![String::from("tr-disabled")]);
        assert!(report.started.is_empty());
        assert!(report.retained.is_empty());
        assert!(!supervisor.sessions().contains_key("tr-disabled"));
    }

    #[test]
    fn reconcile_restarts_lease_lost_session_instead_of_retaining_stale_runtime() {
        let mut supervisor = ExternalTriggerSupervisor::new("daemon-owner");
        let spec = ExternalTriggerSessionSpec {
            trigger_id: String::from("tr-wasm"),
            plugin_id: String::from("plugin-wasm"),
            runtime: ExternalTriggerSessionRuntime::Wasm,
            wasm_component: Some(String::from("trigger_wasm_component")),
        };
        assert!(supervisor.start_session(spec.clone(), 100));

        let stale_runtime_probe = supervisor
            .sessions()
            .get("tr-wasm")
            .and_then(|session| session.wasm_session.as_ref())
            .expect("wasm runtime should exist before lease loss")
            .runtime_drop_probe();

        assert!(supervisor
            .set_push_control_state("tr-wasm", ExternalTriggerPushControlState::LeaseLost,));

        let mut desired = BTreeMap::new();
        desired.insert(String::from("tr-wasm"), spec);
        let report = supervisor.reconcile(desired, 200);

        assert_eq!(report.stopped, vec![String::from("tr-wasm")]);
        assert_eq!(report.started, vec![String::from("tr-wasm")]);
        assert!(report.retained.is_empty());
        assert!(stale_runtime_probe.upgrade().is_none());

        let refreshed = supervisor
            .sessions()
            .get("tr-wasm")
            .expect("lease-lost session should restart under desired ownership");
        assert_eq!(
            refreshed.push_control_state,
            ExternalTriggerPushControlState::Ready
        );
    }

    #[test]
    fn reconcile_restarts_wasm_session_when_component_identity_changes() {
        let mut supervisor = ExternalTriggerSupervisor::new("daemon-owner");
        assert!(supervisor.start_session(
            ExternalTriggerSessionSpec {
                trigger_id: String::from("tr-wasm"),
                plugin_id: String::from("plugin-wasm"),
                runtime: ExternalTriggerSessionRuntime::Wasm,
                wasm_component: Some(String::from("trigger_wasm_component_v1")),
            },
            100,
        ));

        let stale_runtime_probe = supervisor
            .sessions()
            .get("tr-wasm")
            .and_then(|session| session.wasm_session.as_ref())
            .expect("wasm runtime should exist before manifest change")
            .runtime_drop_probe();

        let mut desired = BTreeMap::new();
        desired.insert(
            String::from("tr-wasm"),
            ExternalTriggerSessionSpec {
                trigger_id: String::from("tr-wasm"),
                plugin_id: String::from("plugin-wasm"),
                runtime: ExternalTriggerSessionRuntime::Wasm,
                wasm_component: Some(String::from("trigger_wasm_component_v2")),
            },
        );

        let report = supervisor.reconcile(desired, 200);
        assert_eq!(report.stopped, vec![String::from("tr-wasm")]);
        assert_eq!(report.started, vec![String::from("tr-wasm")]);
        assert!(report.retained.is_empty());
        assert!(stale_runtime_probe.upgrade().is_none());

        let refreshed_component = supervisor
            .sessions()
            .get("tr-wasm")
            .and_then(|session| session.wasm_session.as_ref())
            .expect("changed component should force a new wasm runtime")
            .component();
        assert_eq!(refreshed_component, "trigger_wasm_component_v2");
    }

    #[test]
    fn record_session_turns_keeps_shared_lifecycle_surface_for_process_and_wasm() {
        let mut supervisor = ExternalTriggerSupervisor::new("daemon-owner");
        assert!(supervisor.start_session(
            ExternalTriggerSessionSpec {
                trigger_id: String::from("tr-process"),
                plugin_id: String::from("plugin-process"),
                runtime: ExternalTriggerSessionRuntime::Process,
                wasm_component: None,
            },
            100,
        ));
        assert!(supervisor.start_session(
            ExternalTriggerSessionSpec {
                trigger_id: String::from("tr-wasm"),
                plugin_id: String::from("plugin-wasm"),
                runtime: ExternalTriggerSessionRuntime::Wasm,
                wasm_component: Some(String::from("trigger_wasm_component")),
            },
            100,
        ));

        supervisor.record_session_turns(120);

        let process_session = supervisor
            .sessions()
            .get("tr-process")
            .expect("process session should remain tracked");
        assert!(process_session.wasm_session.is_none());

        let wasm_session = supervisor
            .sessions()
            .get("tr-wasm")
            .and_then(|session| session.wasm_session.as_ref())
            .expect("wasm session should remain tracked");
        assert_eq!(wasm_session.store_turn_count(), 1);
        assert_eq!(wasm_session.guest_state().turn_count, 1);
        assert_eq!(wasm_session.guest_state().last_turn_at_ms, Some(120));
    }

    #[test]
    fn cycle_planner_enforces_round_robin_session_fairness_budget() {
        let mut supervisor = ExternalTriggerSupervisor::with_settings(
            "daemon-owner",
            ExternalTriggerSupervisorSettings {
                budget: ExternalTriggerSupervisorBudget {
                    max_sessions_per_cycle: 1,
                    max_polls_per_session: 2,
                },
                wasm_session: WasmTriggerSessionConfig::default(),
            },
        );
        assert!(supervisor.start_session(
            ExternalTriggerSessionSpec {
                trigger_id: String::from("tr-a"),
                plugin_id: String::from("plugin-a"),
                runtime: ExternalTriggerSessionRuntime::Process,
                wasm_component: None,
            },
            100,
        ));
        assert!(supervisor.start_session(
            ExternalTriggerSessionSpec {
                trigger_id: String::from("tr-b"),
                plugin_id: String::from("plugin-b"),
                runtime: ExternalTriggerSessionRuntime::Process,
                wasm_component: None,
            },
            100,
        ));

        let first_cycle = supervisor.plan_process_polls_for_cycle(110);
        assert_eq!(
            first_cycle,
            vec![ExternalTriggerPollBudget {
                trigger_id: String::from("tr-a"),
                poll_budget: 2,
            }]
        );
        assert_eq!(
            supervisor
                .sessions()
                .get("tr-a")
                .expect("first trigger should exist")
                .push_control_state,
            ExternalTriggerPushControlState::Ready
        );
        assert_eq!(
            supervisor
                .sessions()
                .get("tr-b")
                .expect("second trigger should exist")
                .push_control_state,
            ExternalTriggerPushControlState::BudgetExhausted
        );

        let second_cycle = supervisor.plan_process_polls_for_cycle(120);
        assert_eq!(
            second_cycle,
            vec![ExternalTriggerPollBudget {
                trigger_id: String::from("tr-b"),
                poll_budget: 2,
            }]
        );
        assert_eq!(
            supervisor
                .sessions()
                .get("tr-a")
                .expect("first trigger should exist")
                .push_control_state,
            ExternalTriggerPushControlState::BudgetExhausted
        );
        assert_eq!(
            supervisor
                .sessions()
                .get("tr-b")
                .expect("second trigger should exist")
                .push_control_state,
            ExternalTriggerPushControlState::Ready
        );
    }

    #[test]
    fn cycle_planner_budget_exhaustion_maps_to_retryable_wasm_push_backpressure() {
        let mut supervisor = ExternalTriggerSupervisor::with_settings(
            "daemon-owner",
            ExternalTriggerSupervisorSettings {
                budget: ExternalTriggerSupervisorBudget {
                    max_sessions_per_cycle: 1,
                    max_polls_per_session: 1,
                },
                wasm_session: WasmTriggerSessionConfig::default(),
            },
        );
        assert!(supervisor.start_session(
            ExternalTriggerSessionSpec {
                trigger_id: String::from("tr-a"),
                plugin_id: String::from("plugin-a"),
                runtime: ExternalTriggerSessionRuntime::Wasm,
                wasm_component: Some(String::from("trigger_wasm_component")),
            },
            100,
        ));
        assert!(supervisor.start_session(
            ExternalTriggerSessionSpec {
                trigger_id: String::from("tr-b"),
                plugin_id: String::from("plugin-b"),
                runtime: ExternalTriggerSessionRuntime::Wasm,
                wasm_component: Some(String::from("trigger_wasm_component")),
            },
            100,
        ));

        let sqlite_path = unique_sqlite_path("cycle-planner-budget-exhaustion");
        let storage_config = RuntimeStorageConfig {
            backend: RuntimeStorageBackend::Local {
                database_path: sqlite_path.clone(),
            },
            history_retention: None,
            raw_debug_enabled: false,
            raw_debug_artifacts_dir: None,
        };
        let mut state_store = RuntimeStateStore::open(&storage_config, 130)
            .expect("runtime state store should open for budget outcome checks");

        let _ = supervisor.plan_process_polls_for_cycle(130);
        let budget_exhausted_outcome = supervisor.push_wasm_guest_event(
            "tr-b",
            b"transport-envelope",
            &mut state_store,
            |_session, _event_bytes| {
                panic!("decode callback should not run for budget-exhausted sessions")
            },
        );
        assert_eq!(
            budget_exhausted_outcome,
            HostPushOutcome::RetryableBackpressure
        );

        let _ = supervisor.plan_process_polls_for_cycle(131);
        let next_cycle_outcome = supervisor.push_wasm_guest_event(
            "tr-a",
            b"transport-envelope",
            &mut state_store,
            |_session, _event_bytes| {
                panic!("decode callback should not run for budget-exhausted sessions")
            },
        );
        assert_eq!(next_cycle_outcome, HostPushOutcome::RetryableBackpressure);

        drop(state_store);
        let _ = fs::remove_file(sqlite_path);
    }

    #[test]
    fn cycle_planner_round_robin_stress_remains_deterministic_and_fair() {
        let mut supervisor = ExternalTriggerSupervisor::with_settings(
            "daemon-owner",
            ExternalTriggerSupervisorSettings {
                budget: ExternalTriggerSupervisorBudget {
                    max_sessions_per_cycle: 2,
                    max_polls_per_session: 1,
                },
                wasm_session: WasmTriggerSessionConfig::default(),
            },
        );
        for trigger_id in ["tr-a", "tr-b", "tr-c", "tr-d"] {
            assert!(supervisor.start_session(
                ExternalTriggerSessionSpec {
                    trigger_id: String::from(trigger_id),
                    plugin_id: format!("plugin-{trigger_id}"),
                    runtime: ExternalTriggerSessionRuntime::Process,
                    wasm_component: None,
                },
                100,
            ));
        }

        let mut picks = Vec::new();
        for cycle in 0..8 {
            let plan = supervisor.plan_process_polls_for_cycle(200 + cycle);
            assert_eq!(plan.len(), 2);
            picks.push(
                plan.into_iter()
                    .map(|item| item.trigger_id)
                    .collect::<Vec<_>>(),
            );
        }

        assert_eq!(
            picks,
            vec![
                vec![String::from("tr-a"), String::from("tr-b")],
                vec![String::from("tr-c"), String::from("tr-d")],
                vec![String::from("tr-a"), String::from("tr-b")],
                vec![String::from("tr-c"), String::from("tr-d")],
                vec![String::from("tr-a"), String::from("tr-b")],
                vec![String::from("tr-c"), String::from("tr-d")],
                vec![String::from("tr-a"), String::from("tr-b")],
                vec![String::from("tr-c"), String::from("tr-d")],
            ]
        );

        let exhausted_count = supervisor
            .sessions()
            .values()
            .filter(|session| {
                session.push_control_state == ExternalTriggerPushControlState::BudgetExhausted
            })
            .count();
        assert_eq!(exhausted_count, 2);
    }

    #[test]
    fn wasm_push_control_matrix_handles_backpressure_lease_loss_and_recovery() {
        let mut supervisor = ExternalTriggerSupervisor::new("daemon-owner");
        let spec = ExternalTriggerSessionSpec {
            trigger_id: String::from("tr-wasm"),
            plugin_id: String::from("plugin-wasm"),
            runtime: ExternalTriggerSessionRuntime::Wasm,
            wasm_component: Some(String::from("trigger_wasm_component")),
        };
        assert!(supervisor.start_session(spec.clone(), 100));

        let sqlite_path = unique_sqlite_path("wasm-push-control-matrix");
        let storage_config = RuntimeStorageConfig {
            backend: RuntimeStorageBackend::Local {
                database_path: sqlite_path.clone(),
            },
            history_retention: None,
            raw_debug_enabled: false,
            raw_debug_artifacts_dir: None,
        };
        let mut state_store = RuntimeStateStore::open(&storage_config, 110)
            .expect("runtime state store should open for push-control matrix");

        assert!(supervisor
            .set_push_control_state("tr-wasm", ExternalTriggerPushControlState::QueueSaturated,));
        let saturated_outcome = supervisor.push_wasm_guest_event(
            "tr-wasm",
            br#"{"staging_id":"staged-blocked","event_id":"evt-blocked"}"#,
            &mut state_store,
            |_session, _event_bytes| {
                panic!("decode callback should not run when queue saturation backpressure applies")
            },
        );
        assert_eq!(saturated_outcome, HostPushOutcome::RetryableBackpressure);

        assert!(
            supervisor.set_push_control_state("tr-wasm", ExternalTriggerPushControlState::Ready,)
        );
        let durable_outcome = supervisor.push_wasm_guest_event(
            "tr-wasm",
            br#"{"staging_id":"staged-ok-1","event_id":"evt-ok-1"}"#,
            &mut state_store,
            |session, event_bytes| {
                let payload: serde_json::Value =
                    serde_json::from_slice(event_bytes).expect("payload should decode");
                Ok(StagedTriggerEventRecord {
                    schema_version: String::from("1.0.0"),
                    staging_id: payload["staging_id"]
                        .as_str()
                        .expect("staging id should be a string")
                        .to_owned(),
                    trigger_id: session.trigger_id.clone(),
                    workflow_id: String::from("wf-alpha"),
                    event_id: payload["event_id"]
                        .as_str()
                        .expect("event id should be a string")
                        .to_owned(),
                    source: String::from("external.plugin"),
                    occurred_at_ms: 111,
                    staged_at_ms: 112,
                    checkpoint: Some(String::from("cp-matrix")),
                    payload,
                    dedup_key: None,
                    dedup_window_ms: None,
                    cooldown_key: None,
                    cooldown_ms: None,
                    accepted_at_ms: None,
                    last_error: None,
                })
            },
        );
        assert_eq!(durable_outcome, HostPushOutcome::DurableAck);

        assert!(supervisor
            .set_push_control_state("tr-wasm", ExternalTriggerPushControlState::LeaseLost,));
        let lease_lost_outcome = supervisor.push_wasm_guest_event(
            "tr-wasm",
            br#"{"staging_id":"staged-lost","event_id":"evt-lost"}"#,
            &mut state_store,
            |_session, _event_bytes| {
                panic!("decode callback should not run when lease-loss terminal state applies")
            },
        );
        assert_eq!(lease_lost_outcome, HostPushOutcome::TerminalLeaseLost);

        let mut desired = BTreeMap::new();
        desired.insert(String::from("tr-wasm"), spec);
        let report = supervisor.reconcile(desired, 120);
        assert_eq!(report.stopped, vec![String::from("tr-wasm")]);
        assert_eq!(report.started, vec![String::from("tr-wasm")]);

        let recovered_outcome = supervisor.push_wasm_guest_event(
            "tr-wasm",
            br#"{"staging_id":"staged-ok-2","event_id":"evt-ok-2"}"#,
            &mut state_store,
            |session, event_bytes| {
                let payload: serde_json::Value =
                    serde_json::from_slice(event_bytes).expect("payload should decode");
                Ok(StagedTriggerEventRecord {
                    schema_version: String::from("1.0.0"),
                    staging_id: payload["staging_id"]
                        .as_str()
                        .expect("staging id should be a string")
                        .to_owned(),
                    trigger_id: session.trigger_id.clone(),
                    workflow_id: String::from("wf-alpha"),
                    event_id: payload["event_id"]
                        .as_str()
                        .expect("event id should be a string")
                        .to_owned(),
                    source: String::from("external.plugin"),
                    occurred_at_ms: 121,
                    staged_at_ms: 122,
                    checkpoint: Some(String::from("cp-matrix")),
                    payload,
                    dedup_key: None,
                    dedup_window_ms: None,
                    cooldown_key: None,
                    cooldown_ms: None,
                    accepted_at_ms: None,
                    last_error: None,
                })
            },
        );
        assert_eq!(recovered_outcome, HostPushOutcome::DurableAck);

        let staged_rows = state_store
            .list_pending_staged_trigger_event_records("tr-wasm", 10)
            .expect("pending staged rows should be queryable after recovered pushes");
        let staged_ids = staged_rows
            .iter()
            .map(|row| row.staging_id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(staged_ids, vec!["staged-ok-1", "staged-ok-2"]);

        drop(state_store);
        let _ = fs::remove_file(sqlite_path);
    }

    #[test]
    fn wasm_supervisor_settings_pass_explicit_store_limiter_to_session_runtime() {
        let settings = ExternalTriggerSupervisorSettings {
            budget: ExternalTriggerSupervisorBudget::default(),
            wasm_session: WasmTriggerSessionConfig {
                store_limiter: crate::trigger_wasm::WasmStoreLimiterConfig {
                    memory_size_bytes: 512 * 1024,
                    table_elements: 32,
                    instances: 2,
                    tables: 4,
                    memories: 4,
                    trap_on_grow_failure: true,
                },
            },
        };
        let mut supervisor = ExternalTriggerSupervisor::with_settings("daemon-owner", settings);
        assert!(supervisor.start_session(
            ExternalTriggerSessionSpec {
                trigger_id: String::from("tr-wasm"),
                plugin_id: String::from("plugin-wasm"),
                runtime: ExternalTriggerSessionRuntime::Wasm,
                wasm_component: Some(String::from("trigger_wasm_component")),
            },
            100,
        ));

        let runtime = supervisor
            .sessions()
            .get("tr-wasm")
            .and_then(|session| session.wasm_session.as_ref())
            .expect("wasm runtime should exist");
        assert_eq!(runtime.config(), settings.wasm_session);
    }

    #[test]
    fn build_desired_sessions_matches_daemon_external_trigger_composition() {
        let definitions = vec![
            trigger_definition("tr-process", true, Some("plugin-process")),
            trigger_definition("tr-wasm", true, Some("plugin-wasm")),
            trigger_definition("tr-disabled", false, Some("plugin-process")),
        ];
        let plugin_manifests = vec![
            plugin_manifest("plugin-process", TriggerRuntimeLifecycle::ProcessShortLived),
            plugin_manifest(
                "plugin-wasm",
                TriggerRuntimeLifecycle::WasmDaemonPersistentSession,
            ),
        ];

        let desired = build_desired_external_trigger_sessions(&definitions, &plugin_manifests)
            .expect("desired external trigger sessions should build");

        assert_eq!(desired.len(), 2);
        assert_eq!(
            desired
                .get("tr-process")
                .expect("process trigger should be included")
                .runtime,
            ExternalTriggerSessionRuntime::Process
        );
        assert!(desired
            .get("tr-process")
            .expect("process trigger should be included")
            .wasm_component
            .is_none());
        assert_eq!(
            desired
                .get("tr-wasm")
                .expect("wasm trigger should be included")
                .runtime,
            ExternalTriggerSessionRuntime::Wasm
        );
        assert_eq!(
            desired
                .get("tr-wasm")
                .expect("wasm trigger should be included")
                .wasm_component
                .as_deref(),
            Some("trigger_wasm_component")
        );
        assert!(
            !desired.contains_key("tr-disabled"),
            "disabled external trigger should not start daemon-owned session"
        );
    }

    fn trigger_definition(
        trigger_id: &str,
        enabled: bool,
        plugin_id: Option<&str>,
    ) -> TriggerDefinition {
        TriggerDefinition {
            api_version: String::from("2.0.0"),
            trigger_id: trigger_id.to_owned(),
            kind: String::from("external_plugin"),
            source: String::from("external.source"),
            plugin: plugin_id.map(str::to_owned),
            workflow_id: String::from("wf-alpha"),
            enabled,
            params: BTreeMap::new(),
            input_mapping: BTreeMap::new(),
            package_root: PathBuf::new(),
        }
    }

    fn plugin_manifest(plugin_id: &str, lifecycle: TriggerRuntimeLifecycle) -> PluginManifest {
        let (lifecycle_value, push_callback, durable_ack, module) = match lifecycle {
            TriggerRuntimeLifecycle::ProcessShortLived => (
                "process_short_lived",
                "inline_response",
                "caller_scope",
                None,
            ),
            TriggerRuntimeLifecycle::WasmDaemonPersistentSession => (
                "wasm_daemon_persistent_session",
                "host_callback",
                "after_store_persist",
                Some("trigger_wasm_component"),
            ),
        };
        let mut manifest: PluginManifest = serde_json::from_value(serde_json::json!({
            "manifest_version": "2.0.0",
            "plugin_id": plugin_id,
            "kind": "external_trigger",
            "entrypoint": "trigger.listen.event",
            "capabilities": ["trigger.listen.event"],
            "executable": "bin/trigger.sh",
            "trigger_runtime": {
                "lifecycle": lifecycle_value,
                "push_callback": push_callback,
                "durable_ack": durable_ack,
                "host_error_categories": ["transport", "protocol_contract", "plugin_fatal"],
                "module": module
            }
        }))
        .expect("plugin manifest fixture should deserialize");
        manifest.manifest_path = PathBuf::new();
        manifest
    }

    fn unique_sqlite_path(prefix: &str) -> PathBuf {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("chainbot-{prefix}-{timestamp}.sqlite3"))
    }
}
