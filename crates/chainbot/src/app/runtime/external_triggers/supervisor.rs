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

#[cfg(test)]
use crate::app::runtime::external_triggers::wasmtime::push_event_with_host_callback;
use crate::app::runtime::external_triggers::process_listener::{
    collect_external_process_trigger_emissions, ProcessTriggerSession,
};
use crate::app::runtime::external_triggers::wasmtime::{
    host_push_result_from_staged_append, map_control_flow_source_to_push_outcome,
    ComponentWasmTriggerSession, HostPushControlFlowSource, HostPushError, HostPushErrorSource,
    HostPushOutcome, HostPushResult, TriggerPushHost, WasmGuestTransportEnvelope,
    WasmTriggerSession, WasmTriggerSessionConfig,
};
use crate::domain::state::StagedTriggerEventRecord;
use crate::domain::trigger::{
    TriggerDefinition, TriggerKind, TriggerPlaneError, TriggerPluginHostPolicy,
};
use crate::errors::ContractError;
use crate::infrastructure::state::RuntimeStateStore;
use crate::plugin::{
    HostCancellation, PluginManifest, TriggerRuntimeLifecycle, WasmTriggerAbi,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ExternalTriggerSupervisorBudget {
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
struct ExternalTriggerSupervisorSettings {
    pub budget: ExternalTriggerSupervisorBudget,
    pub wasm_session: WasmTriggerSessionConfig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExternalTriggerPollBudget {
    pub trigger_id: String,
    pub poll_budget: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExternalTriggerSessionRuntime {
    Process,
    ProcessDaemon,
    Wasm,
    WasmComponent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExternalTriggerSessionSpec {
    pub trigger_id: String,
    pub plugin_id: String,
    pub runtime: ExternalTriggerSessionRuntime,
    pub wasm_component: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExternalTriggerSessionState {
    Active,
    Stopping,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExternalTriggerPushControlState {
    Ready,
    #[cfg(test)]
    QueueSaturated,
    BudgetExhausted,
    LeaseLost,
}

#[derive(Debug)]
struct ExternalTriggerSession {
    pub trigger_id: String,
    pub plugin_id: String,
    pub runtime: ExternalTriggerSessionRuntime,
    pub owner_id: String,
    pub state: ExternalTriggerSessionState,
    pub started_at_ms: i64,
    pub push_control_state: ExternalTriggerPushControlState,
    pub process_session: Option<ProcessTriggerSession>,
    pub wasm_session: Option<WasmTriggerSession>,
    pub component_session: Option<ComponentWasmTriggerSession>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExternalTriggerReconcileReport {
    pub started: Vec<String>,
    pub stopped: Vec<String>,
    pub retained: Vec<String>,
}

#[derive(Debug)]
pub(crate) struct ExternalTriggerSupervisor {
    owner_id: String,
    sessions: BTreeMap<String, ExternalTriggerSession>,
    settings: ExternalTriggerSupervisorSettings,
    next_cycle_start_index: usize,
    cancellation: HostCancellation,
}

impl ExternalTriggerSupervisor {
    pub(crate) fn new(owner_id: impl Into<String>) -> Self {
        Self::with_settings(owner_id, ExternalTriggerSupervisorSettings::default())
    }

    fn with_settings(
        owner_id: impl Into<String>,
        settings: ExternalTriggerSupervisorSettings,
    ) -> Self {
        Self {
            owner_id: owner_id.into(),
            sessions: BTreeMap::new(),
            settings,
            next_cycle_start_index: 0,
            cancellation: HostCancellation::default(),
        }
    }

    pub(crate) fn with_cancellation(mut self, cancellation: HostCancellation) -> Self {
        self.cancellation = cancellation;
        self
    }

    pub(crate) fn run_cycle<F>(
        &mut self,
        definitions: &[TriggerDefinition],
        manifests: &[PluginManifest],
        policy: &TriggerPluginHostPolicy,
        state_store: &mut RuntimeStateStore,
        observed_at_ms: i64,
        on_progress: &mut F,
    ) -> Result<(), TriggerPlaneError>
    where
        F: FnMut() -> Result<(), TriggerPlaneError>,
    {
        let desired = build_desired_external_trigger_sessions(definitions, manifests)?;
        let definitions_by_id = definitions
            .iter()
            .map(|definition| (definition.trigger_id.as_str(), definition))
            .collect::<BTreeMap<_, _>>();
        let manifests_by_id = manifests
            .iter()
            .map(|manifest| (manifest.plugin_id.as_str(), manifest))
            .collect::<BTreeMap<_, _>>();

        self.try_reconcile_with_process(
            desired,
            observed_at_ms,
            |spec, cancellation| {
                if spec.runtime != ExternalTriggerSessionRuntime::ProcessDaemon {
                    return Ok(None);
                }
                let definition = definitions_by_id.get(spec.trigger_id.as_str()).ok_or_else(|| {
                    ContractError::InvalidTriggerDefinitionField {
                        trigger_id: spec.trigger_id.clone(),
                        field: "trigger.trigger_id",
                        detail: "managed process session is missing trigger definition".to_owned(),
                    }
                })?;
                let manifest = manifests_by_id.get(spec.plugin_id.as_str()).ok_or_else(|| {
                    ContractError::UnknownTriggerPlugin {
                        trigger_id: spec.trigger_id.clone(),
                        plugin_id: spec.plugin_id.clone(),
                    }
                })?;
                ProcessTriggerSession::start(
                    state_store,
                    definition,
                    manifest,
                    policy,
                    spec.wasm_component.clone().unwrap_or_default(),
                    cancellation,
                )
                .map(Some)
            },
        )?;
        self.drain_process_sessions(definitions, state_store, observed_at_ms)?;
        stage_process_external_trigger_sessions(
            definitions,
            manifests,
            policy,
            self,
            state_store,
            observed_at_ms,
            on_progress,
        )?;
        stage_wasm_external_trigger_sessions(
            definitions,
            manifests,
            self,
            state_store,
            observed_at_ms,
            on_progress,
        )
    }

    pub(crate) fn shutdown(&mut self, observed_at_ms: i64) -> Result<(), ContractError> {
        self.try_reconcile_with_process(BTreeMap::new(), observed_at_ms, |_, _| Ok(None))
            .map(|_| ())
    }

    fn sessions(&self) -> &BTreeMap<String, ExternalTriggerSession> {
        &self.sessions
    }

    #[cfg(test)]
    fn start_session(&mut self, spec: ExternalTriggerSessionSpec, now_ms: i64) -> bool {
        self.try_start_session_with_process(spec, now_ms, None)
            .expect("test external trigger session should start")
    }

    fn try_start_session_with_process(
        &mut self,
        spec: ExternalTriggerSessionSpec,
        now_ms: i64,
        process_session: Option<ProcessTriggerSession>,
    ) -> Result<bool, ContractError> {
        let ExternalTriggerSessionSpec {
            trigger_id,
            plugin_id,
            runtime,
            wasm_component,
        } = spec;
        if self.sessions.contains_key(&trigger_id) {
            return Ok(false);
        }

        let component_path = wasm_component.clone().unwrap_or_default();
        let wasm_session = if runtime == ExternalTriggerSessionRuntime::Wasm {
            #[cfg(test)]
            {
                Some(WasmTriggerSession::new_with_config(
                    trigger_id.clone(),
                    plugin_id.clone(),
                    component_path.clone(),
                    now_ms,
                    self.settings.wasm_session,
                ))
            }
            #[cfg(not(test))]
            {
                Some(
                    WasmTriggerSession::try_new_with_config(
                        trigger_id.clone(),
                        plugin_id.clone(),
                        component_path.clone(),
                        now_ms,
                        self.settings.wasm_session,
                    )
                    .map_err(|error| ContractError::TriggerPluginWasmHostFailure {
                        plugin_id: plugin_id.clone(),
                        operation: "compile or instantiate core_v0",
                        detail: error.to_string(),
                    })?,
                )
            }
        } else {
            None
        };
        let component_session = if runtime == ExternalTriggerSessionRuntime::WasmComponent {
            Some(
                ComponentWasmTriggerSession::try_new_with_config(
                    component_path,
                    now_ms,
                    self.settings.wasm_session,
                )
                .map_err(|error| ContractError::TriggerPluginWasmHostFailure {
                    plugin_id: plugin_id.clone(),
                    operation: "compile or instantiate component_v1",
                    detail: error.to_string(),
                })?,
            )
        } else {
            None
        };

        if runtime == ExternalTriggerSessionRuntime::ProcessDaemon && process_session.is_none() {
            return Err(ContractError::TriggerPluginProtocolContractViolation {
                plugin_id,
                detail: "managed process session requires a runtime owner".to_owned(),
            });
        }

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
                process_session,
                wasm_session,
                component_session,
            },
        );
        Ok(true)
    }

    #[cfg(test)]
    fn stop_session(&mut self, trigger_id: &str) -> bool {
        self.try_stop_session(trigger_id).unwrap_or(false)
    }

    fn try_stop_session(&mut self, trigger_id: &str) -> Result<bool, ContractError> {
        let Some(mut session) = self.sessions.remove(trigger_id) else {
            return Ok(false);
        };
        if let Some(process_session) = session.process_session.as_mut() {
            process_session.shutdown("supervisor_reconcile")?;
        }
        Ok(true)
    }

    #[cfg(test)]
    fn record_session_turn(&mut self, trigger_id: &str, now_ms: i64) -> bool {
        if let Some(session) = self.sessions.get_mut(trigger_id) {
            if let Some(wasm_session) = session.wasm_session.as_mut() {
                wasm_session.begin_turn(now_ms);
            }
            return true;
        }

        false
    }

    #[cfg(test)]
    fn record_session_turns(&mut self, now_ms: i64) {
        for session in self.sessions.values_mut() {
            if let Some(wasm_session) = session.wasm_session.as_mut() {
                wasm_session.begin_turn(now_ms);
            }
        }
    }

    fn execute_wasm_guest_turn(
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
            process_session: None,
            wasm_session: None,
            component_session: None,
        };

        let mut host = DurableStagedEventHost {
            session: &session_view,
            state_store,
            decode_to_staged_record: &mut decode_to_staged_record,
        };

        let outcome = match session.runtime {
            ExternalTriggerSessionRuntime::Wasm => session
                .wasm_session
                .as_mut()
                .ok_or(())
                .and_then(|wasm| wasm.execute_guest_turn(now_ms, &mut host).map_err(|_| ())),
            ExternalTriggerSessionRuntime::WasmComponent => session
                .component_session
                .as_mut()
                .ok_or(())
                .and_then(|wasm| wasm.execute_guest_turn(now_ms, &mut host).map_err(|_| ())),
            ExternalTriggerSessionRuntime::Process
            | ExternalTriggerSessionRuntime::ProcessDaemon => Err(()),
        };
        if outcome.is_err() {
            session.state = ExternalTriggerSessionState::Stopping;
        }
        outcome.unwrap_or_else(|_| {
            map_control_flow_source_to_push_outcome(
                HostPushControlFlowSource::DaemonShuttingDown,
            )
        })
    }

    fn stage_wasm_guest_turn(
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

    fn plan_process_polls_for_cycle(&mut self, _now_ms: i64) -> Vec<ExternalTriggerPollBudget> {
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

    #[cfg(test)]
    fn set_push_control_state(
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

    #[cfg(test)]
    fn push_wasm_guest_event<F>(
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

    fn drain_process_sessions(
        &mut self,
        definitions: &[TriggerDefinition],
        state_store: &mut RuntimeStateStore,
        now_ms: i64,
    ) -> Result<usize, ContractError> {
        let definitions = definitions
            .iter()
            .map(|definition| (definition.trigger_id.as_str(), definition))
            .collect::<BTreeMap<_, _>>();
        let mut staged = 0usize;
        for session in self.sessions.values_mut() {
            if session.runtime != ExternalTriggerSessionRuntime::ProcessDaemon {
                continue;
            }
            let definition = definitions.get(session.trigger_id.as_str()).ok_or_else(|| {
                ContractError::InvalidTriggerDefinitionField {
                    trigger_id: session.trigger_id.clone(),
                    field: "trigger.trigger_id",
                    detail: "managed process session is missing trigger definition".to_owned(),
                }
            })?;
            let process_session = session.process_session.as_mut().ok_or_else(|| {
                ContractError::TriggerPluginProtocolContractViolation {
                    plugin_id: session.plugin_id.clone(),
                    detail: "managed process session is missing its runtime owner".to_owned(),
                }
            })?;
            staged = staged.saturating_add(process_session.drain(
                state_store,
                definition,
                now_ms,
            )?);
        }
        Ok(staged)
    }

    fn try_reconcile_with_process<F>(
        &mut self,
        desired: BTreeMap<String, ExternalTriggerSessionSpec>,
        now_ms: i64,
        mut start_process: F,
    ) -> Result<ExternalTriggerReconcileReport, ContractError>
    where
        F: FnMut(&ExternalTriggerSessionSpec, HostCancellation)
            -> Result<Option<ProcessTriggerSession>, ContractError>,
    {
        let existing_ids = self.sessions.keys().cloned().collect::<Vec<_>>();
        let mut report = ExternalTriggerReconcileReport {
            started: Vec::new(),
            stopped: Vec::new(),
            retained: Vec::new(),
        };

        for trigger_id in existing_ids {
            let should_stop_for_disable = !desired.contains_key(&trigger_id);
            let should_stop_for_lease_loss = self.cancellation.is_cancelled()
                || self
                    .sessions
                    .get(&trigger_id)
                    .is_some_and(|session| session_is_lease_lost(session, &self.owner_id));
            if should_stop_for_lease_loss
                && let Some(session) = self.sessions.get_mut(&trigger_id)
            {
                session.state = ExternalTriggerSessionState::Stopping;
                session.push_control_state = ExternalTriggerPushControlState::LeaseLost;
            }
            if (should_stop_for_disable || should_stop_for_lease_loss)
                && self.try_stop_session(&trigger_id)?
            {
                report.stopped.push(trigger_id);
            }
        }

        if self.cancellation.is_cancelled() {
            return Ok(report);
        }

        for (trigger_id, spec) in desired {
            if let Some(session) = self.sessions.get_mut(&trigger_id) {
                if session_matches_spec(session, &spec, &self.owner_id) {
                    if let Some(wasm_session) = session.wasm_session.as_mut() {
                        wasm_session.mark_reconciled(now_ms);
                    }
                    if let Some(component_session) = session.component_session.as_mut() {
                        component_session.mark_reconciled(now_ms);
                    }
                    report.retained.push(trigger_id);
                    continue;
                }
            }

            if self.try_stop_session(&trigger_id)? {
                report.stopped.push(trigger_id.clone());
            }
            let process_session = if spec.runtime == ExternalTriggerSessionRuntime::ProcessDaemon {
                start_process(&spec, self.cancellation.clone())?
            } else {
                None
            };
            if self.try_start_session_with_process(spec, now_ms, process_session)? {
                report.started.push(trigger_id);
            }
        }

        Ok(report)
    }

    #[cfg(test)]
    fn reconcile(
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

            if should_stop_for_lease_loss
                && let Some(session) = self.sessions.get_mut(&trigger_id)
            {
                session.state = ExternalTriggerSessionState::Stopping;
                session.push_control_state = ExternalTriggerPushControlState::LeaseLost;
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
                    if let Some(component_session) = session.component_session.as_mut() {
                        component_session.mark_reconciled(now_ms);
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

    if session.runtime == ExternalTriggerSessionRuntime::ProcessDaemon {
        let expected_identity = spec.wasm_component.as_deref().unwrap_or_default();
        return session
            .process_session
            .as_ref()
            .is_some_and(|process| process.is_active() && process.identity() == expected_identity);
    }
    let expected_component = spec.wasm_component.as_deref().unwrap_or_default();
    if session.runtime == ExternalTriggerSessionRuntime::WasmComponent {
        return session
            .component_session
            .as_ref()
            .map(ComponentWasmTriggerSession::component)
            == Some(expected_component);
    }
    if session.runtime != ExternalTriggerSessionRuntime::Wasm {
        return true;
    }

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
    if !matches!(
        session.runtime,
        ExternalTriggerSessionRuntime::Wasm | ExternalTriggerSessionRuntime::WasmComponent
    ) {
        return Some(HostPushControlFlowSource::LeaseLost);
    }

    let state_source = if session.state == ExternalTriggerSessionState::Stopping {
        Some(HostPushControlFlowSource::DaemonShuttingDown)
    } else {
        None
    };
    state_source.or(match session.push_control_state {
        ExternalTriggerPushControlState::Ready => None,
        #[cfg(test)]
        ExternalTriggerPushControlState::QueueSaturated => {
            Some(HostPushControlFlowSource::QueueSaturated)
        }
        ExternalTriggerPushControlState::BudgetExhausted => {
            Some(HostPushControlFlowSource::BudgetExhausted)
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
        payload: envelope.payload,
        dedup_key: envelope.dedup_key,
        dedup_window_ms: envelope.dedup_window_ms,
        cooldown_key: envelope.cooldown_key,
        cooldown_ms: envelope.cooldown_ms,
        accepted_at_ms: None,
        last_error: None,
    })
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

fn stage_process_external_trigger_sessions<F>(
    definitions: &[TriggerDefinition],
    manifests: &[PluginManifest],
    policy: &TriggerPluginHostPolicy,
    supervisor: &mut ExternalTriggerSupervisor,
    state_store: &mut RuntimeStateStore,
    staged_at_ms: i64,
    on_progress: &mut F,
) -> Result<(), TriggerPlaneError>
where
    F: FnMut() -> Result<(), TriggerPlaneError>,
{
    let process_poll_budget = supervisor.plan_process_polls_for_cycle(staged_at_ms);
    let definitions_by_id: BTreeMap<String, &TriggerDefinition> = definitions
        .iter()
        .map(|definition| (definition.trigger_id.clone(), definition))
        .collect();
    let manifests_by_id: BTreeMap<String, &PluginManifest> = manifests
        .iter()
        .map(|manifest| (manifest.plugin_id.clone(), manifest))
        .collect();

    for poll_budget in &process_poll_budget {
        let session = supervisor
            .sessions()
            .get(&poll_budget.trigger_id)
            .ok_or_else(|| {
                TriggerPlaneError::Contract(ContractError::InvalidTriggerDefinitionField {
                    trigger_id: poll_budget.trigger_id.clone(),
                    field: "trigger.trigger_id",
                    detail: "process polling budget references a missing supervisor session"
                        .to_owned(),
                })
            })?;
        if session.runtime != ExternalTriggerSessionRuntime::Process {
            continue;
        }

        let definition = definitions_by_id.get(&session.trigger_id).ok_or_else(|| {
            TriggerPlaneError::Contract(ContractError::InvalidTriggerDefinitionField {
                trigger_id: session.trigger_id.clone(),
                field: "trigger.trigger_id",
                detail: "process supervisor session is missing trigger definition".to_owned(),
            })
        })?;
        let manifest = manifests_by_id.get(&session.plugin_id).ok_or_else(|| {
            TriggerPlaneError::Contract(ContractError::UnknownTriggerPlugin {
                trigger_id: session.trigger_id.clone(),
                plugin_id: session.plugin_id.clone(),
            })
        })?;

        for poll_ordinal in 0..poll_budget.poll_budget {
            let emissions = collect_external_process_trigger_emissions(
                state_store,
                definition,
                manifest,
                policy,
                on_progress,
            )?;
            for (index, emission) in emissions.into_iter().enumerate() {
                on_progress()?;
                let staged_record = StagedTriggerEventRecord {
                    schema_version: String::from("1.0.0"),
                    staging_id: format!(
                        "process:{}:{}:{}:{poll_ordinal}:{index}",
                        definition.trigger_id, emission.event_id, staged_at_ms
                    ),
                    trigger_id: definition.trigger_id.clone(),
                    workflow_id: definition.workflow_id.clone(),
                    event_id: emission.event_id,
                    source: emission
                        .source
                        .filter(|value| !value.trim().is_empty())
                        .unwrap_or_else(|| definition.source.clone()),
                    occurred_at_ms: emission.occurred_at_ms,
                    staged_at_ms,
                    checkpoint: emission.checkpoint,
                    payload: emission.payload,
                    dedup_key: emission.dedup_key,
                    dedup_window_ms: emission.dedup_window_ms,
                    cooldown_key: emission.cooldown_key,
                    cooldown_ms: emission.cooldown_ms,
                    accepted_at_ms: None,
                    last_error: None,
                };
                let _ = state_store.append_staged_trigger_event_record(&staged_record)?;
            }
        }
    }

    Ok(())
}

fn stage_wasm_external_trigger_sessions<F>(
    definitions: &[TriggerDefinition],
    manifests: &[PluginManifest],
    supervisor: &mut ExternalTriggerSupervisor,
    state_store: &mut RuntimeStateStore,
    staged_at_ms: i64,
    on_progress: &mut F,
) -> Result<(), TriggerPlaneError>
where
    F: FnMut() -> Result<(), TriggerPlaneError>,
{
    let wasm_sessions = supervisor
        .sessions()
        .values()
        .filter(|session| {
            matches!(
                session.runtime,
                ExternalTriggerSessionRuntime::Wasm
                    | ExternalTriggerSessionRuntime::WasmComponent
            )
        })
        .map(|session| (session.trigger_id.clone(), session.plugin_id.clone()))
        .collect::<Vec<_>>();

    for (trigger_id, plugin_id) in wasm_sessions {
        on_progress()?;

        let definition = definitions
            .iter()
            .find(|definition| definition.trigger_id == trigger_id)
            .ok_or_else(|| {
                TriggerPlaneError::Contract(ContractError::InvalidTriggerDefinitionField {
                    trigger_id: trigger_id.clone(),
                    field: "trigger.trigger_id",
                    detail: "wasm supervisor session is missing trigger definition".to_owned(),
                })
            })?;
        let manifest = manifests
            .iter()
            .find(|manifest| manifest.plugin_id == plugin_id)
            .ok_or_else(|| {
                TriggerPlaneError::Contract(ContractError::UnknownTriggerPlugin {
                    trigger_id: trigger_id.clone(),
                    plugin_id: plugin_id.clone(),
                })
            })?;

        if manifest
            .trigger_runtime
            .as_ref()
            .and_then(|runtime| runtime.lifecycle)
            != Some(TriggerRuntimeLifecycle::WasmDaemonPersistentSession)
        {
            return Err(TriggerPlaneError::Contract(
                ContractError::NodePluginInvalidField {
                    plugin_id: manifest.plugin_id.clone(),
                    field: "plugin.trigger_runtime.lifecycle",
                    detail:
                        "wasm supervisor session requires wasm_daemon_persistent_session lifecycle"
                            .to_owned(),
                },
            ));
        }

        let outcome =
            supervisor.stage_wasm_guest_turn(&trigger_id, definition, staged_at_ms, state_store);

        if matches!(
            outcome,
            HostPushOutcome::DurableAck | HostPushOutcome::RetryableBackpressure
        ) {
            continue;
        }
    }

    Ok(())
}

fn build_desired_external_trigger_sessions(
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
            Some(TriggerRuntimeLifecycle::ProcessDaemonSession) => {
                ExternalTriggerSessionRuntime::ProcessDaemon
            }
            Some(TriggerRuntimeLifecycle::WasmDaemonPersistentSession) => {
                if plugin
                    .trigger_runtime
                    .as_ref()
                    .and_then(|runtime| runtime.abi)
                    == Some(WasmTriggerAbi::ComponentV1)
                {
                    ExternalTriggerSessionRuntime::WasmComponent
                } else {
                    ExternalTriggerSessionRuntime::Wasm
                }
            }
            _ => ExternalTriggerSessionRuntime::Process,
        };

        let wasm_component = if runtime == ExternalTriggerSessionRuntime::ProcessDaemon {
            Some(process_session_identity(definition, plugin))
        } else {
            resolve_wasm_module_for_session(plugin)?
        };

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

fn process_session_identity(
    definition: &TriggerDefinition,
    plugin: &PluginManifest,
) -> String {
    format!("{definition:?}|{plugin:?}")
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

    use crate::infrastructure::config::{RuntimeStorageBackend, RuntimeStorageConfig};

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
    fn process_daemon_session_requires_runtime_owner() {
        let mut supervisor = ExternalTriggerSupervisor::new("daemon-owner");
        let error = supervisor
            .try_start_session_with_process(
                ExternalTriggerSessionSpec {
                    trigger_id: String::from("tr-process-daemon"),
                    plugin_id: String::from("plugin-process"),
                    runtime: ExternalTriggerSessionRuntime::ProcessDaemon,
                    wasm_component: None,
                },
                100,
                None,
            )
            .expect_err("managed process session without owner should be rejected");

        assert!(error.to_string().contains("runtime owner"));
        assert!(supervisor.sessions().is_empty());
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
                store_limiter:
                    crate::app::runtime::external_triggers::wasmtime::WasmStoreLimiterConfig {
                        memory_size_bytes: 512 * 1024,
                        table_elements: 32,
                        instances: 2,
                        tables: 4,
                        memories: 4,
                        trap_on_grow_failure: true,
                    },
                fuel_per_turn: 1_000_000,
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
    fn build_desired_sessions_selects_component_v1_explicitly() {
        let definitions = vec![trigger_definition(
            "tr-component",
            true,
            Some("plugin-component"),
        )];
        let mut manifest = plugin_manifest(
            "plugin-component",
            TriggerRuntimeLifecycle::WasmDaemonPersistentSession,
        );
        manifest
            .trigger_runtime
            .as_mut()
            .expect("wasm fixture should have runtime")
            .abi = Some(WasmTriggerAbi::ComponentV1);

        let desired = build_desired_external_trigger_sessions(&definitions, &[manifest])
            .expect("component desired state should build");

        assert_eq!(
            desired
                .get("tr-component")
                .expect("component session should exist")
                .runtime,
            ExternalTriggerSessionRuntime::WasmComponent
        );
    }

    #[test]
    fn build_desired_sessions_matches_daemon_external_trigger_composition() {
        let definitions = vec![
            trigger_definition("tr-process", true, Some("plugin-process")),
            trigger_definition("tr-managed", true, Some("plugin-managed")),
            trigger_definition("tr-wasm", true, Some("plugin-wasm")),
            trigger_definition("tr-disabled", false, Some("plugin-process")),
        ];
        let plugin_manifests = vec![
            plugin_manifest("plugin-process", TriggerRuntimeLifecycle::ProcessShortLived),
            plugin_manifest(
                "plugin-managed",
                TriggerRuntimeLifecycle::ProcessDaemonSession,
            ),
            plugin_manifest(
                "plugin-wasm",
                TriggerRuntimeLifecycle::WasmDaemonPersistentSession,
            ),
        ];

        let desired = build_desired_external_trigger_sessions(&definitions, &plugin_manifests)
            .expect("desired external trigger sessions should build");

        assert_eq!(desired.len(), 3);
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
                .get("tr-managed")
                .expect("managed trigger should be included")
                .runtime,
            ExternalTriggerSessionRuntime::ProcessDaemon
        );
        assert!(desired
            .get("tr-managed")
            .expect("managed trigger should be included")
            .wasm_component
            .as_deref()
            .is_some_and(|identity| identity.contains("tr-managed")));
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
            TriggerRuntimeLifecycle::ProcessDaemonSession => (
                "process_daemon_session",
                "inline_response",
                "after_store_persist",
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
