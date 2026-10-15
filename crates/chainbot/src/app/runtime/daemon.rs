//! [INPUT]
//! Resolved root layouts, daemon owner identifiers, runtime definition bundles, and state/trigger/ingress supervisors.
//!
//! [OUTPUT]
//! Returns daemon lifecycle outcomes and completed trigger requests while coordinating lease-bound serve-loop execution.
//!
//! [ROLE]
//! Owns daemon runtime control paths separate from CLI parse/help and read-model rendering.

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Stdio;
use std::thread;

use super::context::{
    current_time_ms, map_ingress_error, map_runtime_state_error, resolve_existing_root, RuntimeContext,
};
use super::external_triggers::ExternalTriggerSupervisor;
use super::lease::{DaemonLeaseSession, SERVE_LEASE_TTL_MS};
use super::plugin_activation::parse_trigger_plugin_activation_bindings;
use super::run::execute_single_run_with_fence_and_cancellation;
use crate::app::definitions::load_root_definition_bundle;
use crate::domain::runtime::NormalizedRunRequest;
use crate::domain::state::{
    LeaseAcquireResult, ServeLeaseState, TriggerEventRecord,
    SERVE_OWNER_ID_PREFIX,
};
use crate::domain::trigger::{
    TriggerPlane, TriggerPlaneError, TriggerPluginHostPolicy, TriggerRunRequest,
    REQUIRED_TRIGGER_PLUGIN_CAPABILITY,
};
use crate::errors::{ContractError, UserFacingError};
use crate::infrastructure::config::{RootConfigDefinition, RootLayout, RuntimeStorageConfig};
use crate::infrastructure::state::RuntimeStateStore;
use crate::ingress::{
    build_desired_ingress_state, drain_ingress_emissions, DesiredIngressState,
    TriggerIngressSupervisor,
};
use crate::plugin::{PluginKind, PluginManifest};

const SERVE_IDLE_POLL_INTERVAL_MS: u64 = 250;
const SERVE_ERROR_BACKOFF_MS: u64 = 1_000;
const SERVE_START_ACK_TIMEOUT_MS: u64 = 5_000;
const SERVE_STOP_TIMEOUT_MS: u64 = 5_000;
const INGRESS_INBOX_BATCH_LIMIT: usize = 256;
const REPLAYABLE_TRIGGER_BATCH_LIMIT: usize = 256;
const CHAINBOT_TEST_DAEMON_START_DELAY_MS_ENV: &str = "CHAINBOT_TEST_DAEMON_START_DELAY_MS";
const CHAINBOT_TEST_DAEMON_START_ACK_TIMEOUT_MS_ENV: &str =
    "CHAINBOT_TEST_DAEMON_START_ACK_TIMEOUT_MS";

#[derive(Debug)]
pub(crate) struct DaemonStart {
    pub(crate) owner_id: String,
    pub(crate) pid: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StopOutcome {
    NoActiveDaemon,
    StaleStateCleaned,
    ActiveLeaseCleared,
    ShutdownRequested,
}

pub(crate) fn execute_serve(root_layout: &RootLayout) -> Result<DaemonStart, UserFacingError> {
    let definitions = load_root_definition_bundle(root_layout).map_err(UserFacingError::from_contract)?;
    let storage_config = definitions
        .root_config
        .resolve_runtime_storage(&root_layout.root)
        .map_err(UserFacingError::from_contract)?;
    let now_ms = current_time_ms()?;
    let owner_id = format!("{SERVE_OWNER_ID_PREFIX}{}-{now_ms}", std::process::id());
    let mut store = RuntimeStateStore::open(&storage_config, now_ms)
        .map_err(|error| map_runtime_state_error("open runtime state store", error))?;

    match store
        .try_acquire_serve_lease(&owner_id, now_ms, SERVE_LEASE_TTL_MS)
        .map_err(|error| map_runtime_state_error("acquire serve lease", error))?
    {
        LeaseAcquireResult::Acquired { .. } | LeaseAcquireResult::Renewed { .. } => {
            store
                .register_daemon_start(&owner_id, None, now_ms, now_ms + SERVE_LEASE_TTL_MS)
                .map_err(|error| map_runtime_state_error("register daemon start", error))?;
            let mut child = match spawn_serve_daemon_process(&owner_id, root_layout) {
                Ok(child) => child,
                Err(error) => {
                    let cleanup_at_ms = current_time_ms().unwrap_or(now_ms);
                    let _ = store.mark_daemon_stopped(&owner_id, cleanup_at_ms);
                    let _ = store.release_serve_lease(&owner_id);
                    return Err(error);
                }
            };
            if !store
                .attach_daemon_pid(&owner_id, i64::from(child.id()))
                .map_err(|error| map_runtime_state_error("attach daemon pid", error))?
            {
                let cleanup_at_ms = current_time_ms().unwrap_or(now_ms);
                let _ = child.kill();
                let _ = child.wait();
                let _ = store.mark_daemon_stopped(&owner_id, cleanup_at_ms);
                let _ = store.release_serve_lease(&owner_id);
                return Err(UserFacingError::state_with_code(
                    "daemon_start_failed",
                    "The daemon session disappeared before the child PID could be recorded.",
                ));
            }
            wait_for_daemon_start(&mut store, &owner_id, child.id(), now_ms).map_err(
                |error| {
                    let _ = child.kill();
                    let _ = child.wait();
                    let cleanup_at_ms = current_time_ms().unwrap_or(now_ms);
                    let _ = store.mark_daemon_stopped(&owner_id, cleanup_at_ms);
                    let _ = store.release_serve_lease(&owner_id);
                    error
                },
            )?;
            Ok(DaemonStart { owner_id, pid: child.id() })
        }
        LeaseAcquireResult::Rejected {
            current_owner,
            expires_at_ms,
        } => Err(UserFacingError::conflict_with_code(
            "daemon_already_running",
            format!(
                "Serve is already active for this root (owner: {current_owner}, expires_at_ms: {expires_at_ms})."
            ),
        )),
    }
}

pub(crate) fn execute_stop(root_layout: &RootLayout) -> Result<StopOutcome, UserFacingError> {
    let definitions = load_root_definition_bundle(root_layout).map_err(UserFacingError::from_contract)?;
    let storage_config = definitions
        .root_config
        .resolve_runtime_storage(&root_layout.root)
        .map_err(UserFacingError::from_contract)?;
    let now_ms = current_time_ms()?;
    let mut store = RuntimeStateStore::open(&storage_config, now_ms)
        .map_err(|error| map_runtime_state_error("open runtime state store", error))?;
    let status = store
        .inspect_daemon_status(now_ms)
        .map_err(|error| map_runtime_state_error("inspect daemon status", error))?;

    match status.state {
        ServeLeaseState::Idle => {
            return Ok(StopOutcome::NoActiveDaemon);
        }
        ServeLeaseState::Stale => {
            if let Some(owner_id) = status.owner_id.as_deref() {
                let _ = store.mark_daemon_stopped(owner_id, now_ms);
                let _ = store.release_serve_lease(owner_id);
            }
            return Ok(StopOutcome::StaleStateCleaned);
        }
        ServeLeaseState::Active => {}
    }

    if !store
        .request_daemon_stop(now_ms)
        .map_err(|error| map_runtime_state_error("request daemon stop", error))?
    {
        if let Some(owner_id) = status.owner_id.as_deref() {
            let _ = store.mark_daemon_stopped(owner_id, now_ms);
            let _ = store.release_serve_lease(owner_id);
            return Ok(StopOutcome::ActiveLeaseCleared);
        }
        return Ok(StopOutcome::NoActiveDaemon);
    }

    wait_for_daemon_stop(&mut store)?;
    Ok(StopOutcome::ShutdownRequested)
}

pub(crate) fn execute_internal_serve_daemon(
    root_layout: &RootLayout,
    owner_id: String,
) -> Result<(), UserFacingError> {
    let now_ms = current_time_ms()?;
    let definitions = load_root_definition_bundle(root_layout).map_err(UserFacingError::from_contract)?;
    let storage_config = definitions
        .root_config
        .resolve_runtime_storage(&root_layout.root)
        .map_err(UserFacingError::from_contract)?;
    let pid = i64::from(std::process::id());

    let mut store = RuntimeStateStore::open(&storage_config, now_ms)
        .map_err(|error| map_runtime_state_error("open runtime state store", error))?;
    let status = store
        .inspect_daemon_status(now_ms)
        .map_err(|error| map_runtime_state_error("inspect daemon status", error))?;
    if status.owner_id.as_deref() != Some(owner_id.as_str()) {
        return Ok(());
    }
    if status.stop_requested_at_ms.is_some() {
        return Ok(());
    }
    maybe_delay_daemon_start_for_tests()?;
    let status = store
        .inspect_daemon_status(current_time_ms()?)
        .map_err(|error| map_runtime_state_error("inspect daemon status", error))?;
    if status.owner_id.as_deref() != Some(owner_id.as_str()) {
        return Ok(());
    }
    if status.stop_requested_at_ms.is_some() {
        return Ok(());
    }
    let acked_at_ms = status
        .started_at_ms
        .map(|started_at_ms| {
            current_time_ms()
                .unwrap_or(now_ms)
                .max(started_at_ms.saturating_add(1))
        })
        .unwrap_or_else(|| current_time_ms().unwrap_or(now_ms));
    let mut lease_session = DaemonLeaseSession::acquire(store, &owner_id, pid, acked_at_ms)?;
    if !lease_session.poll_turn(acked_at_ms)? {
        return Ok(());
    }
    run_internal_serve_daemon_loop(root_layout, &storage_config, &owner_id, &mut lease_session)
}

fn spawn_serve_daemon_process(owner_id: &str, root_layout: &RootLayout) -> Result<std::process::Child, UserFacingError> {
    std::process::Command::new(std::env::current_exe().map_err(|source| {
        UserFacingError::state_with_code(
            "daemon_start_failed",
            format!("Failed to resolve the running chainbot binary for daemon spawn: {source}"),
        )
    })?)
    .args(["__serve-daemon", "--owner-id", owner_id])
    .env(crate::infrastructure::config::CHAINBOT_CONFIG_DIR_ENV, &root_layout.root)
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .spawn()
    .map_err(|source| {
        UserFacingError::state_with_code(
            "daemon_start_failed",
            format!("Failed to spawn the background daemon process: {source}"),
        )
    })
}

fn wait_for_daemon_start(
    store: &mut RuntimeStateStore,
    owner_id: &str,
    child_pid: u32,
    started_at_ms: i64,
) -> Result<(), UserFacingError> {
    let deadline_ms = started_at_ms.saturating_add(serve_start_ack_timeout_ms() as i64);
    while current_time_ms()? <= deadline_ms {
        let observed_at_ms = current_time_ms()?;
        let daemon_status = store
            .inspect_daemon_status(observed_at_ms)
            .map_err(|error| map_runtime_state_error("inspect daemon status", error))?;
        if daemon_status.owner_id.as_deref() == Some(owner_id)
            && daemon_status.pid == Some(i64::from(child_pid))
            && daemon_status.started_at_ms == Some(started_at_ms)
            && daemon_status
                .last_heartbeat_at_ms
                .is_some_and(|heartbeat_at_ms| heartbeat_at_ms > started_at_ms)
        {
            return Ok(());
        }
        thread::sleep(std::time::Duration::from_millis(50));
    }

    Err(UserFacingError::unavailable_with_code(
        "daemon_start_failed",
        "The background daemon did not become observable before the start acknowledgement timeout.",
    ))
}

fn wait_for_daemon_stop(store: &mut RuntimeStateStore) -> Result<(), UserFacingError> {
    let started_wait_ms = current_time_ms()?;
    let deadline_ms = started_wait_ms.saturating_add(SERVE_STOP_TIMEOUT_MS as i64);
    while current_time_ms()? <= deadline_ms {
        let observed_at_ms = current_time_ms()?;
        let daemon_status = store
            .inspect_daemon_status(observed_at_ms)
            .map_err(|error| map_runtime_state_error("inspect daemon status", error))?;
        if daemon_status.state == ServeLeaseState::Idle {
            return Ok(());
        }
        thread::sleep(std::time::Duration::from_millis(100));
    }

    Err(UserFacingError::unavailable_with_code(
        "daemon_stop_timeout",
        "Timed out while waiting for the daemon to stop gracefully.",
    ))
}

pub(crate) fn run_internal_serve_daemon_loop(
    root_layout: &RootLayout,
    storage_config: &RuntimeStorageConfig,
    owner_id: &str,
    lease_session: &mut DaemonLeaseSession,
) -> Result<(), UserFacingError> {
    let ingress_supervisor =
        TriggerIngressSupervisor::start(storage_config.clone()).map_err(map_ingress_error)?;
    let cancellation = lease_session.cancellation();
    let mut external_trigger_supervisor = ExternalTriggerSupervisor::new(owner_id.to_owned())
        .with_cancellation(cancellation.clone());
    lease_session.start_heartbeat();

    let loop_result = (|| loop {
        let observed_at_ms = current_time_ms()?;
        if !lease_session.poll_turn(observed_at_ms)? {
            return Ok(());
        }

        let mut runtime = match resolve_existing_root(root_layout).and_then(RuntimeContext::load) {
            Ok(runtime) => runtime,
            Err(error) => {
                let _ = ingress_supervisor.reconcile(DesiredIngressState::default());
                teardown_external_trigger_sessions(
                    &mut external_trigger_supervisor,
                    observed_at_ms,
                )?;
                lease_session.record_error(&error)?;
                thread::sleep(std::time::Duration::from_millis(SERVE_ERROR_BACKOFF_MS));
                continue;
            }
        };
        let desired_ingress = build_desired_ingress_state(&runtime.definitions.triggers)
            .map_err(UserFacingError::from_contract)?;
        ingress_supervisor
            .reconcile(desired_ingress)
            .map_err(map_ingress_error)?;
        lease_session.mark_reload(observed_at_ms)?;

        match serve_once_with_lease(
            &mut runtime,
            observed_at_ms,
            lease_session,
            &mut external_trigger_supervisor,
        ) {
            Ok(_) => {
                thread::sleep(std::time::Duration::from_millis(
                    SERVE_IDLE_POLL_INTERVAL_MS,
                ));
            }
            Err(error) => {
                if matches!(error, UserFacingError::Conflict { .. }) {
                    return Err(error);
                }
                lease_session.record_error(&error)?;
                thread::sleep(std::time::Duration::from_millis(SERVE_ERROR_BACKOFF_MS));
            }
        }
    })();

    cancellation.shutdown();
    let teardown_result = teardown_external_trigger_sessions(
        &mut external_trigger_supervisor,
        current_time_ms().unwrap_or_default(),
    );
    let shutdown_result = ingress_supervisor.shutdown().map_err(map_ingress_error);
    let lease_result = lease_session.shutdown(current_time_ms().unwrap_or_default());
    loop_result
        .and(teardown_result)
        .and(shutdown_result)
        .and(lease_result)
}

pub(crate) fn teardown_external_trigger_sessions(
    supervisor: &mut ExternalTriggerSupervisor,
    observed_at_ms: i64,
) -> Result<(), UserFacingError> {
    supervisor
        .shutdown(observed_at_ms)
        .map_err(UserFacingError::from_contract)
}

pub(crate) fn serve_once_with_lease(
    runtime: &mut RuntimeContext,
    accepted_at_ms: i64,
    lease_session: &DaemonLeaseSession,
    external_trigger_supervisor: &mut ExternalTriggerSupervisor,
) -> Result<Vec<TriggerRunRequest>, UserFacingError> {
    lease_session.check(accepted_at_ms)?;
    let replay_requests =
        load_replayable_trigger_requests(runtime, REPLAYABLE_TRIGGER_BATCH_LIMIT)?;
    let trigger_definitions = runtime.definitions.triggers.clone();
    let trigger_manifests = collect_external_trigger_manifests(&runtime.definitions.plugins);
    let policy = build_trigger_host_policy(
        &runtime.definitions.root_config,
        &trigger_manifests,
        &runtime.root_layout.plugins_dir,
        &runtime.root_layout.secrets_dir,
    )
    .map_err(UserFacingError::from_contract)?;
    let lease_error = std::cell::RefCell::new(None);
    let mut renew_progress = || {
        current_time_ms().and_then(|now_ms| lease_session.check(now_ms)).map_err(|error| {
            *lease_error.borrow_mut() = Some(error);
            TriggerPlaneError::runtime_state("The daemon lease session stopped.")
        })
    };
    external_trigger_supervisor
        .run_cycle(
            &trigger_definitions,
            &trigger_manifests,
            &policy,
            &mut runtime.state_store,
            accepted_at_ms,
            &mut renew_progress,
        )
        .map_err(|error| lease_error.borrow_mut().take().unwrap_or_else(|| map_trigger_error(error)))?;

    let builtin_events = crate::builtins::build_builtin_trigger_emissions(
        &trigger_definitions,
        accepted_at_ms,
    )
    .map_err(UserFacingError::from_contract)?;
    let drained_ingress = drain_ingress_emissions(
        &mut runtime.state_store,
        &trigger_definitions,
        INGRESS_INBOX_BATCH_LIMIT,
    )
    .map_err(map_trigger_error)?;
    let mut builtin_events = builtin_events;
    for (trigger_id, mut ingress_events) in drained_ingress.emissions {
        builtin_events
            .entry(trigger_id)
            .or_default()
            .append(&mut ingress_events);
    }

    let trigger_store = RuntimeStateStore::open(&runtime.storage_config, accepted_at_ms)
        .map_err(|error| map_runtime_state_error("open trigger runtime state store", error))?;

    let mut trigger_plane =
        TriggerPlane::open_with_store(trigger_store, trigger_definitions, builtin_events)
    .map_err(map_trigger_error)?;

    let run_requests = trigger_plane
        .collect_run_requests_with_progress(accepted_at_ms, &mut renew_progress)
        .map_err(|error| lease_error.borrow_mut().take().unwrap_or_else(|| map_trigger_error(error)))?;
    for inbox_id in drained_ingress.inbox_ids {
        runtime
            .state_store
            .mark_ingress_inbox_processed(&inbox_id, accepted_at_ms)
            .map_err(|error| map_runtime_state_error("mark ingress inbox processed", error))?;
    }
    if run_requests.is_empty() && replay_requests.is_empty() {
        return Ok(Vec::new());
    }

    let run_requests = merge_trigger_requests(replay_requests, run_requests);

    let mut failures = Vec::new();

    for trigger_request in &run_requests {
        lease_session.check(current_time_ms()?)?;
        let normalized_request = normalized_request_from_trigger(trigger_request);
        let fence = lease_session.fence();
        match execute_single_run_with_fence_and_cancellation(
            runtime,
            normalized_request,
            accepted_at_ms,
            Some(&fence),
            &lease_session.cancellation(),
        ) {
            Ok(_) => {}
            Err(error) => failures.push(format!(
                "trigger_id={} event_id={} workflow_id={} error={error}",
                trigger_request.trigger_id, trigger_request.event_id, trigger_request.workflow_id
            )),
        }
    }

    if !failures.is_empty() {
        return Err(UserFacingError::unavailable(format!(
            "Serve evaluated {} accepted trigger event(s) and observed {} failure(s): {}",
            run_requests.len(),
            failures.len(),
            failures.join("; ")
        )));
    }

    Ok(run_requests)
}

fn map_trigger_error(error: TriggerPlaneError) -> UserFacingError {
    match error {
        TriggerPlaneError::Contract(source) => UserFacingError::from_contract(source),
        TriggerPlaneError::RuntimeState(source) => UserFacingError::state(format!(
            "Failed to evaluate trigger-plane runtime state: {source}"
        )),
    }
}

fn maybe_delay_daemon_start_for_tests() -> Result<(), UserFacingError> {
    let Some(delay_ms) = std::env::var(CHAINBOT_TEST_DAEMON_START_DELAY_MS_ENV)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
    else {
        return Ok(());
    };
    thread::sleep(std::time::Duration::from_millis(delay_ms));
    Ok(())
}

fn serve_start_ack_timeout_ms() -> u64 {
    std::env::var(CHAINBOT_TEST_DAEMON_START_ACK_TIMEOUT_MS_ENV)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(SERVE_START_ACK_TIMEOUT_MS)
}

fn build_trigger_host_policy(
    root_config: &RootConfigDefinition,
    manifests: &[PluginManifest],
    plugins_root_dir: &Path,
    secrets_root_dir: &Path,
) -> Result<TriggerPluginHostPolicy, ContractError> {
    let allowlisted_plugin_ids = manifests
        .iter()
        .filter_map(|manifest| {
            manifest
                .kind()
                .ok()
                .filter(|kind| *kind == PluginKind::ExternalTrigger)
                .map(|_| manifest.plugin_id.clone())
        })
        .collect();

    Ok(TriggerPluginHostPolicy {
        allowlisted_plugin_ids,
        allowed_capabilities: BTreeSet::from([REQUIRED_TRIGGER_PLUGIN_CAPABILITY.to_owned()]),
        plugin_root_dir: plugins_root_dir.to_path_buf(),
        plugin_activation: parse_trigger_plugin_activation_bindings(root_config)?,
        secrets_root_dir: secrets_root_dir.to_path_buf(),
    })
}

fn collect_external_trigger_manifests(
    plugin_manifests: &[PluginManifest],
) -> Vec<PluginManifest> {
    plugin_manifests
        .iter()
        .filter_map(|manifest| {
            manifest
                .kind()
                .ok()
                .filter(|kind| *kind == PluginKind::ExternalTrigger)
                .map(|_| manifest.clone())
        })
        .collect()
}

fn normalized_request_from_trigger(
    trigger_request: &TriggerRunRequest,
) -> NormalizedRunRequest {
    let mut request = NormalizedRunRequest::new(
        trigger_request.run_id.clone(),
        trigger_request.workflow_id.clone(),
    );

    match &trigger_request.payload {
        serde_json::Value::Object(values) => {
            request.trigger_payload_mapping.extend(
                values
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone())),
            );
        }
        other => {
            request
                .trigger_payload_mapping
                .insert(String::from("payload"), other.clone());
        }
    }

    request
}

fn load_replayable_trigger_requests(
    runtime: &mut RuntimeContext,
    limit: usize,
) -> Result<Vec<TriggerRunRequest>, UserFacingError> {
    runtime
        .state_store
        .list_replayable_trigger_records(limit)
        .map_err(|error| map_runtime_state_error("list replayable trigger records", error))
        .map(|records| {
            records
                .into_iter()
                .map(replay_trigger_request_from_record)
                .collect()
        })
}

fn replay_trigger_request_from_record(record: TriggerEventRecord) -> TriggerRunRequest {
    TriggerRunRequest {
        run_id: record.run_id,
        workflow_id: record.workflow_id,
        trigger_id: record.trigger_id.clone(),
        event_id: record.event_id,
        source: record.source,
        accepted_at_ms: record.accepted_at_ms,
        payload: record.payload,
        trigger_record_ref: format!(
            "db://trigger_event_records/{}/{}",
            record.trigger_id, record.sequence
        ),
    }
}

fn merge_trigger_requests(
    replay_requests: Vec<TriggerRunRequest>,
    new_requests: Vec<TriggerRunRequest>,
) -> Vec<TriggerRunRequest> {
    let mut merged = Vec::with_capacity(replay_requests.len().saturating_add(new_requests.len()));
    let mut seen_run_ids = BTreeSet::new();

    for request in replay_requests.into_iter().chain(new_requests) {
        if seen_run_ids.insert(request.run_id.clone()) {
            merged.push(request);
        }
    }

    merged
}

#[cfg(test)]
mod runtime_tests {
    use super::*;

    use std::fs;
    use std::sync::{Mutex, OnceLock};

    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use super::super::context::resolve_existing_root;
    use crate::domain::state::StagedTriggerEventRecord;

    const CHAINBOT_SECRET_DECRYPTOR_ENV: &str = "CHAINBOT_SECRET_DECRYPTOR";
    const SECRET_DECRYPTOR_PLAINTEXT: &str = "plaintext";

    #[test]
    fn replay_requests_keep_priority_and_payload_when_new_requests_overlap() {
        let replay = trigger_request("replayed", serde_json::json!({"value": "persisted"}));
        let duplicate = trigger_request("replayed", serde_json::json!({"value": "new"}));
        let fresh = trigger_request("fresh", serde_json::json!({"value": "fresh"}));
        let merged = merge_trigger_requests(vec![replay.clone()], vec![duplicate, fresh]);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].run_id, "replayed");
        assert_eq!(merged[0].payload, replay.payload);
        assert_eq!(merged[1].run_id, "fresh");
    }

    #[test]
    fn trigger_payload_normalization_preserves_objects_and_wraps_other_values() {
        for payload in [serde_json::json!({"symbol": "ETH"}), serde_json::json!([1, 2]), serde_json::Value::Null] {
            let request = normalized_request_from_trigger(&trigger_request("run", payload.clone()));
            assert_eq!(request.run_id, "run");
            assert_eq!(request.workflow_id, "workflow");
            let expected = match payload {
                serde_json::Value::Object(values) => values.into_iter().collect(),
                other => BTreeMap::from([(String::from("payload"), other)]),
            };
            assert_eq!(request.trigger_payload_mapping, expected);
        }
    }

    fn trigger_request(run_id: &str, payload: serde_json::Value) -> TriggerRunRequest {
        TriggerRunRequest {
            run_id: run_id.to_owned(),
            workflow_id: String::from("workflow"),
            trigger_id: String::from("trigger"),
            event_id: run_id.to_owned(),
            source: String::from("test"),
            accepted_at_ms: 1,
            payload,
            trigger_record_ref: String::from("db://trigger_event_records/trigger/1"),
        }
    }

    #[test]
    fn config_reload_requires_restart() {
        let _guard = fixture_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());

        let root = prepare_fixture_root("success", "cli-config-reload-requires-restart");

        unsafe {
            std::env::set_var("CHAINBOT_CONFIG_DIR", &root);
            std::env::set_var(CHAINBOT_SECRET_DECRYPTOR_ENV, SECRET_DECRYPTOR_PLAINTEXT);
        }

        let mut runtime = load_test_runtime(&root)
            .expect("initial runtime load should succeed");

        fs::write(
            root.join("triggers")
                .join("external-trigger-e2e")
                .join("config.toml"),
            "manifest_version = \"2.0.0\"\ntrigger_id = [\n",
        )
        .expect("mutated trigger file should be writable");

        let lease_session = DaemonLeaseSession::acquire(
            RuntimeStateStore::open(&runtime.storage_config, current_time_ms().unwrap()).unwrap(),
            "test-owner",
            i64::from(std::process::id()),
            current_time_ms().unwrap(),
        ).expect("test lease should acquire");
        let mut external_trigger_supervisor = ExternalTriggerSupervisor::new("test-owner");
        let serve_output = serve_once_with_lease(
            &mut runtime,
            1_710_300_000_000,
            &lease_session,
            &mut external_trigger_supervisor,
        )
        .expect("already loaded runtime should ignore on-disk config mutation");
        assert_eq!(serve_output.len(), 1);

        let reload_error = load_test_runtime(&root)
            .expect_err("restarting after config mutation should observe invalid TOML");
        assert!(matches!(reload_error, UserFacingError::Validation { .. }));
        assert!(reload_error
            .to_string()
            .contains("definition file is invalid TOML"));

        unsafe {
            std::env::remove_var("CHAINBOT_CONFIG_DIR");
            std::env::remove_var(CHAINBOT_SECRET_DECRYPTOR_ENV);
        }
    }

    #[test]
    fn serve_drains_all_requests_from_snapshot() {
        let _guard = fixture_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());

        let root = prepare_fixture_root("success", "cli-serve-drains-all-requests");
        fs::create_dir_all(root.join("triggers").join("builtin-trigger-e2e"))
            .expect("builtin trigger package directory should be creatable");
        fs::write(
            root.join("triggers")
                .join("builtin-trigger-e2e")
                .join("config.toml"),
            "manifest_version = \"2.0.0\"\ntrigger_id = \"builtin-trigger-e2e\"\nkind = \"builtin\"\nsource = \"manual\"\nworkflow_id = \"wf-e2e\"\nenabled = true\n",
        )
        .expect("builtin trigger fixture should be writable");


        unsafe {
            std::env::set_var("CHAINBOT_CONFIG_DIR", &root);
            std::env::set_var(CHAINBOT_SECRET_DECRYPTOR_ENV, SECRET_DECRYPTOR_PLAINTEXT);
        }

        let mut runtime = load_test_runtime(&root)
            .expect("runtime load should succeed with canonical builtin trigger");
        let lease_session = DaemonLeaseSession::acquire(
            RuntimeStateStore::open(&runtime.storage_config, current_time_ms().unwrap()).unwrap(),
            "test-owner",
            i64::from(std::process::id()),
            current_time_ms().unwrap(),
        ).expect("test lease should acquire");
        let mut external_trigger_supervisor = ExternalTriggerSupervisor::new("test-owner");
        let serve_output = serve_once_with_lease(
            &mut runtime,
            1_710_300_100_000,
            &lease_session,
            &mut external_trigger_supervisor,
        )
        .expect("serve should execute every accepted request from the snapshot");

        assert_eq!(serve_output.len(), 2);
        assert!(serve_output.iter().any(|request| request.trigger_id == "builtin-trigger-e2e"));
        assert!(serve_output.iter().any(|request| request.trigger_id == "external-trigger-e2e"));

        let run_summaries = runtime
            .state_store
            .list_run_summaries()
            .expect("serve snapshot runs should be persisted");
        assert_eq!(run_summaries.len(), 2);

        unsafe {
            std::env::remove_var("CHAINBOT_CONFIG_DIR");
            std::env::remove_var(CHAINBOT_SECRET_DECRYPTOR_ENV);
        }
    }

    #[test]
    fn serve_bridges_pending_staged_external_events() {
        let _guard = fixture_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());

        let root = prepare_fixture_root("success", "cli-serve-bridges-staged-external-events");

        unsafe {
            std::env::set_var("CHAINBOT_CONFIG_DIR", &root);
            std::env::set_var(CHAINBOT_SECRET_DECRYPTOR_ENV, SECRET_DECRYPTOR_PLAINTEXT);
        }

        let mut runtime = load_test_runtime(&root)
            .expect("runtime load should succeed for staged external bridge coverage");
        runtime
            .state_store
            .append_staged_trigger_event_record(&StagedTriggerEventRecord {
                schema_version: String::from("1.0.0"),
                staging_id: String::from("staging-external-bridge-1"),
                trigger_id: String::from("external-trigger-e2e"),
                workflow_id: String::from("wf-e2e"),
                event_id: String::from("external-trigger-e2e:event-staged"),
                source: String::from("external-trigger-plugin"),
                occurred_at_ms: 1_710_300_150_000,
                staged_at_ms: 1_710_300_150_001,
                checkpoint: Some(String::from("cp-staged-1")),
                payload: serde_json::json!({"symbol": "ETHUSDT"}),
                dedup_key: None,
                dedup_window_ms: None,
                cooldown_key: None,
                cooldown_ms: None,
                accepted_at_ms: None,
                last_error: None,
            })
            .expect("staged external trigger row should persist before serve turn");

        let lease_session = DaemonLeaseSession::acquire(
            RuntimeStateStore::open(&runtime.storage_config, current_time_ms().unwrap()).unwrap(),
            "test-owner",
            i64::from(std::process::id()),
            current_time_ms().unwrap(),
        ).expect("test lease should acquire");
        let mut external_trigger_supervisor = ExternalTriggerSupervisor::new("test-owner");
        let serve_output = serve_once_with_lease(
            &mut runtime,
            1_710_300_150_010,
            &lease_session,
            &mut external_trigger_supervisor,
        )
        .expect("serve should bridge staged external rows through trigger acceptance");

        assert_eq!(serve_output.len(), 2);
        assert!(serve_output.iter().any(|request| request.event_id == "external-trigger-e2e:event-staged"));

        let pending = runtime
            .state_store
            .list_pending_staged_trigger_event_records("external-trigger-e2e", 10)
            .expect("pending staged rows should be queryable after serve bridge");
        assert!(pending.is_empty());

        unsafe {
            std::env::remove_var("CHAINBOT_CONFIG_DIR");
            std::env::remove_var(CHAINBOT_SECRET_DECRYPTOR_ENV);
        }
    }

    #[test]
    fn trigger_progress_preserves_lease_loss_as_a_runtime_conflict() {
        let _guard = fixture_lock().lock().unwrap_or_else(|error| error.into_inner());
        let root = prepare_fixture_root("success", "cli-serve-lease-progress-conflict");
        let mut runtime = load_test_runtime(&root).unwrap();
        let now_ms = current_time_ms().unwrap();
        let accepted_at_ms = now_ms - 10_000;
        let lease_session = DaemonLeaseSession::acquire(
            RuntimeStateStore::open(&runtime.storage_config, now_ms).unwrap(),
            "owner-a", i64::from(std::process::id()), accepted_at_ms,
        ).unwrap();
        runtime.state_store.release_serve_lease("owner-a").unwrap();
        runtime.state_store.try_acquire_serve_lease("owner-b", now_ms, SERVE_LEASE_TTL_MS).unwrap();
        let mut external_trigger_supervisor = ExternalTriggerSupervisor::new("owner-a");
        let error = serve_once_with_lease(
            &mut runtime, accepted_at_ms, &lease_session, &mut external_trigger_supervisor,
        ).unwrap_err();
        assert_eq!(error.error_code(), "serve_lease_lost");
        assert!(matches!(error, UserFacingError::Conflict { .. }));
        assert!(runtime.state_store.list_run_summaries().unwrap().is_empty());
    }

    fn load_test_runtime(root: &Path) -> Result<RuntimeContext, UserFacingError> {
        let layout = resolve_existing_root(&RootLayout::from_root(root.to_path_buf()))?;
        RuntimeContext::load(layout)
    }

    fn copy_directory_recursive(source: &Path, destination: &Path) -> Result<(), std::io::Error> {
        fs::create_dir_all(destination)?;
        let entries = fs::read_dir(source)?;
        for entry in entries {
            let entry = entry?;
            let source_path = entry.path();
            let destination_path = destination.join(entry.file_name());

            if source_path.is_dir() {
                copy_directory_recursive(&source_path, &destination_path)?;
            } else {
                if let Some(parent) = destination_path.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::copy(&source_path, &destination_path)?;
            }
        }

        Ok(())
    }

    fn prepare_fixture_root(case_name: &str, root_name: &str) -> PathBuf {
        let root = workspace_root()
            .join("target")
            .join("test-roots")
            .join(root_name);
        if root.exists() {
            fs::remove_dir_all(&root).expect("existing fixture root should be removable");
        }

        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("e2e")
            .join(case_name);
        copy_directory_recursive(&source, &root).expect("fixture root should copy");
        let root_config_path = root.join("chainbot.toml");
        let root_config = fs::read_to_string(&root_config_path)
            .expect("copied fixture root config should be readable");
        let updated_root_config = root_config
            .lines()
            .map(|line| {
                if line.trim_start().starts_with("chainbot_version = ") {
                    format!("chainbot_version = \"{}\"", env!("CARGO_PKG_VERSION"))
                } else {
                    line.to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(&root_config_path, format!("{updated_root_config}\n"))
            .expect("fixture root config should be rewritten with the running version");

        make_executable(&root.join("plugins").join("bin").join("external_trigger.sh"));
        make_executable(&root.join("plugins").join("bin").join("external_node.sh"));

        root
    }

    fn workspace_root() -> PathBuf {
        let crate_root = Path::new(env!("CARGO_MANIFEST_DIR"));
        crate_root
            .parent()
            .expect("crates directory should exist")
            .parent()
            .expect("workspace root should exist")
            .to_path_buf()
    }

    fn make_executable(path: &Path) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = fs::metadata(path)
                .expect("fixture script metadata should exist")
                .permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(path, permissions).expect("fixture script should be executable");
        }
    }

    fn fixture_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }
}
