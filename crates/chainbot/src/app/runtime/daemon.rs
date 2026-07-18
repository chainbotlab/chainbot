//! [INPUT]
//! CLI daemon command requests, runtime definition bundles, and state/trigger/ingress supervisors.
//!
//! [OUTPUT]
//! Executes serve/stop/internal-daemon lifecycle flows and lease-bound serve-loop orchestration.
//!
//! [ROLE]
//! Owns daemon runtime control paths separate from CLI parse/help and read-model rendering.

use std::collections::BTreeMap;
use std::process::Stdio;
use std::sync::mpsc::{self, TryRecvError};
use std::thread;
use std::time::Duration;

use crate::app::cli::{
    build_trigger_host_policy, collect_external_trigger_manifests, current_time_ms,
    execute_single_run_with_fence_and_cancellation, load_replayable_trigger_requests,
    map_ingress_error,
    map_runtime_state_error, merge_trigger_requests, normalized_request_from_trigger, CliOutput,
    CliRequest, RuntimeContext,
};
use crate::app::runtime::external_triggers::process_listener::{
    collect_external_process_trigger_emissions, ProcessTriggerSession,
};
use crate::app::runtime::external_triggers::supervisor::{
    build_desired_external_trigger_sessions, ExternalTriggerPollBudget,
    ExternalTriggerSessionRuntime, ExternalTriggerSupervisor,
};
use crate::app::runtime::external_triggers::wasmtime::HostPushOutcome;
use crate::domain::state::{
    LeaseAcquireResult, RunExecutionFence, RunStatus, ServeLeaseGrant, ServeLeaseState,
    StagedTriggerEventRecord, SERVE_OWNER_ID_PREFIX,
};
use crate::domain::trigger::{TriggerDefinition, TriggerPlane, TriggerPlaneError};
use crate::errors::UserFacingError;
use crate::infrastructure::config::RuntimeStorageConfig;
use crate::infrastructure::state::RuntimeStateStore;
use crate::ingress::{
    build_desired_ingress_state, drain_ingress_emissions, DesiredIngressState,
    TriggerIngressSupervisor,
};
use crate::plugin::{
    HostCancellation, HostCancellationReason, PluginManifest, TriggerRuntimeLifecycle,
};

const SERVE_LEASE_TTL_MS: i64 = 30_000;
const SERVE_LEASE_RENEW_INTERVAL_MS: i64 = 10_000;
const SERVE_LEASE_RETRY_INTERVAL_MS: i64 = 1_000;
const SERVE_HEARTBEAT_STOP_POLL_MS: i64 = 250;
const SERVE_IDLE_POLL_INTERVAL_MS: u64 = 250;
const SERVE_ERROR_BACKOFF_MS: u64 = 1_000;
const SERVE_START_ACK_TIMEOUT_MS: u64 = 5_000;
const SERVE_STOP_TIMEOUT_MS: u64 = 5_000;
const INGRESS_INBOX_BATCH_LIMIT: usize = 256;
const REPLAYABLE_TRIGGER_BATCH_LIMIT: usize = 256;
const CHAINBOT_TEST_DAEMON_START_DELAY_MS_ENV: &str = "CHAINBOT_TEST_DAEMON_START_DELAY_MS";
const CHAINBOT_TEST_DAEMON_START_ACK_TIMEOUT_MS_ENV: &str =
    "CHAINBOT_TEST_DAEMON_START_ACK_TIMEOUT_MS";

#[derive(Debug, Clone)]
pub(crate) struct ServeLeaseSupervisor {
    storage_config: RuntimeStorageConfig,
    owner_id: String,
    pid: i64,
    lease_ttl_ms: i64,
    renew_interval_ms: i64,
    next_renew_at_ms: i64,
    grant: Option<ServeLeaseGrant>,
    cancellation: HostCancellation,
    independent_heartbeat: bool,
}

impl ServeLeaseSupervisor {
    pub(crate) fn new(
        storage_config: RuntimeStorageConfig,
        owner_id: String,
        pid: i64,
        acquired_at_ms: i64,
    ) -> Self {
        Self {
            storage_config,
            owner_id,
            pid,
            lease_ttl_ms: SERVE_LEASE_TTL_MS,
            renew_interval_ms: SERVE_LEASE_RENEW_INTERVAL_MS,
            next_renew_at_ms: acquired_at_ms.saturating_add(SERVE_LEASE_RENEW_INTERVAL_MS),
            grant: None,
            cancellation: HostCancellation::default(),
            independent_heartbeat: false,
        }
    }

    pub(crate) fn with_grant(
        storage_config: RuntimeStorageConfig,
        pid: i64,
        grant: ServeLeaseGrant,
        acquired_at_ms: i64,
    ) -> Self {
        let mut supervisor = Self::new(storage_config, grant.owner_id.clone(), pid, acquired_at_ms);
        supervisor.grant = Some(grant);
        supervisor
    }

    fn with_grant_and_cancellation(
        storage_config: RuntimeStorageConfig,
        pid: i64,
        grant: ServeLeaseGrant,
        acquired_at_ms: i64,
        cancellation: HostCancellation,
    ) -> Self {
        let mut supervisor = Self::with_grant(storage_config, pid, grant, acquired_at_ms);
        supervisor.cancellation = cancellation;
        supervisor.independent_heartbeat = true;
        supervisor
    }

    pub(crate) fn cancellation(&self) -> HostCancellation {
        self.cancellation.clone()
    }

    pub(crate) fn execution_fence(&self) -> Option<RunExecutionFence> {
        self.grant.as_ref().map(|grant| RunExecutionFence {
            owner_id: grant.owner_id.clone(),
            lease_generation: grant.generation,
        })
    }

    pub(crate) fn maybe_renew(&mut self, now_ms: i64) -> Result<(), UserFacingError> {
        self.ensure_active()?;
        if self.independent_heartbeat || now_ms < self.next_renew_at_ms {
            return Ok(());
        }

        let mut store = RuntimeStateStore::open(&self.storage_config, now_ms)
            .map_err(|error| map_runtime_state_error("open runtime state store", error))?;
        match store
            .try_acquire_serve_lease(&self.owner_id, now_ms, self.lease_ttl_ms)
            .map_err(|error| map_runtime_state_error("renew serve lease", error))?
        {
            LeaseAcquireResult::Acquired { grant } | LeaseAcquireResult::Renewed { grant } => {
                let expires_at_ms = grant.expires_at_ms;
                self.grant = Some(grant);
                store
                    .heartbeat_daemon(
                        &self.owner_id,
                        Some(self.pid),
                        now_ms,
                        expires_at_ms,
                    )
                    .map_err(|error| map_runtime_state_error("heartbeat daemon session", error))?;
                self.next_renew_at_ms = now_ms.saturating_add(self.renew_interval_ms);
                Ok(())
            }
            LeaseAcquireResult::Rejected {
                current_owner,
                expires_at_ms,
            } => {
                self.cancellation.lease_lost();
                Err(UserFacingError::conflict(format!(
                    "Serve lease renewal was rejected (current owner: {current_owner}, expires_at_ms: {expires_at_ms})."
                )))
            }
        }
    }

    fn ensure_active(&self) -> Result<(), UserFacingError> {
        match self.cancellation.reason() {
            None => Ok(()),
            Some(HostCancellationReason::LeaseLost) => Err(UserFacingError::conflict_with_code(
                "serve_lease_lost",
                "Serve lease ownership was lost; active runtime work has been cancelled.",
            )),
            Some(HostCancellationReason::ShuttingDown) => Err(UserFacingError::unavailable_with_code(
                "daemon_shutting_down",
                "The daemon is shutting down; active runtime work has been cancelled.",
            )),
        }
    }
}

#[derive(Debug)]
struct ServeLeaseHeartbeat {
    stop_tx: mpsc::SyncSender<()>,
    error_rx: mpsc::Receiver<UserFacingError>,
    handle: Option<thread::JoinHandle<()>>,
}

impl ServeLeaseHeartbeat {
    fn start(
        storage_config: RuntimeStorageConfig,
        owner_id: String,
        pid: i64,
        initial_grant: ServeLeaseGrant,
        cancellation: HostCancellation,
        started_at_ms: i64,
    ) -> Self {
        let (stop_tx, stop_rx) = mpsc::sync_channel(1);
        let (error_tx, error_rx) = mpsc::sync_channel(1);
        let handle = thread::spawn(move || {
            if let Err(error) = run_serve_lease_heartbeat(
                &storage_config,
                &owner_id,
                pid,
                initial_grant,
                &cancellation,
                started_at_ms,
                &stop_rx,
            ) {
                cancellation.lease_lost();
                let _ = error_tx.try_send(error);
            }
        });
        Self {
            stop_tx,
            error_rx,
            handle: Some(handle),
        }
    }

    fn check(&self) -> Result<(), UserFacingError> {
        match self.error_rx.try_recv() {
            Ok(error) => Err(error),
            Err(TryRecvError::Empty) => Ok(()),
            Err(TryRecvError::Disconnected) if self.handle.is_some() => {
                Err(UserFacingError::unavailable_with_code(
                    "serve_heartbeat_stopped",
                    "The serve lease heartbeat stopped unexpectedly.",
                ))
            }
            Err(TryRecvError::Disconnected) => Ok(()),
        }
    }

    fn shutdown(&mut self) -> Result<(), UserFacingError> {
        let _ = self.stop_tx.try_send(());
        let Some(handle) = self.handle.take() else {
            return Ok(());
        };
        handle.join().map_err(|_| {
            UserFacingError::unavailable_with_code(
                "serve_heartbeat_join_failed",
                "The serve lease heartbeat thread panicked during shutdown.",
            )
        })
    }
}

fn run_serve_lease_heartbeat(
    storage_config: &RuntimeStorageConfig,
    owner_id: &str,
    pid: i64,
    initial_grant: ServeLeaseGrant,
    cancellation: &HostCancellation,
    started_at_ms: i64,
    stop_rx: &mpsc::Receiver<()>,
) -> Result<(), UserFacingError> {
    let mut expires_at_ms = initial_grant.expires_at_ms;
    let mut next_renew_at_ms = started_at_ms.saturating_add(SERVE_LEASE_RENEW_INTERVAL_MS);

    loop {
        let now_ms = current_time_ms()?;
        if now_ms >= next_renew_at_ms {
            match renew_serve_lease_once(storage_config, owner_id, pid, now_ms) {
                Ok(grant) => {
                    expires_at_ms = grant.expires_at_ms;
                    next_renew_at_ms = now_ms.saturating_add(SERVE_LEASE_RENEW_INTERVAL_MS);
                }
                Err(HeartbeatRenewalError::Rejected(error)) => return Err(error),
                Err(HeartbeatRenewalError::Storage(error)) => {
                    if now_ms >= expires_at_ms {
                        return Err(error);
                    }
                    next_renew_at_ms = now_ms
                        .saturating_add(SERVE_LEASE_RETRY_INTERVAL_MS)
                        .min(expires_at_ms);
                }
            }
        }

        if cancellation.is_cancelled() {
            return Ok(());
        }
        let wait_ms = next_renew_at_ms
            .saturating_sub(current_time_ms()?)
            .clamp(1, SERVE_HEARTBEAT_STOP_POLL_MS);
        match stop_rx.recv_timeout(Duration::from_millis(wait_ms as u64)) {
            Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
}

#[derive(Debug)]
enum HeartbeatRenewalError {
    Rejected(UserFacingError),
    Storage(UserFacingError),
}

fn renew_serve_lease_once(
    storage_config: &RuntimeStorageConfig,
    owner_id: &str,
    pid: i64,
    now_ms: i64,
) -> Result<ServeLeaseGrant, HeartbeatRenewalError> {
    let mut store = RuntimeStateStore::open(storage_config, now_ms).map_err(|error| {
        HeartbeatRenewalError::Storage(map_runtime_state_error(
            "open heartbeat runtime state store",
            error,
        ))
    })?;
    let grant = match store
        .try_acquire_serve_lease(owner_id, now_ms, SERVE_LEASE_TTL_MS)
        .map_err(|error| {
            HeartbeatRenewalError::Storage(map_runtime_state_error(
                "renew serve lease from heartbeat",
                error,
            ))
        })?
    {
        LeaseAcquireResult::Acquired { grant } | LeaseAcquireResult::Renewed { grant } => grant,
        LeaseAcquireResult::Rejected {
            current_owner,
            expires_at_ms,
        } => {
            return Err(HeartbeatRenewalError::Rejected(
                UserFacingError::conflict_with_code(
                    "serve_lease_lost",
                    format!(
                        "Serve lease heartbeat was rejected (current owner: {current_owner}, expires_at_ms: {expires_at_ms})."
                    ),
                ),
            ));
        }
    };
    store
        .heartbeat_daemon(owner_id, Some(pid), now_ms, grant.expires_at_ms)
        .map_err(|error| {
            HeartbeatRenewalError::Storage(map_runtime_state_error(
                "persist daemon heartbeat",
                error,
            ))
        })?;
    Ok(grant)
}

pub(crate) fn execute_serve(request: &CliRequest) -> Result<CliOutput, UserFacingError> {
    let root_layout = request.resolve_existing_root()?;
    let definitions = request.load_root_definition_bundle()?;
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
            let mut child = match spawn_serve_daemon_process(&owner_id) {
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
            wait_for_daemon_start(&storage_config, &owner_id, child.id(), now_ms).map_err(
                |error| {
                    let _ = child.kill();
                    let _ = child.wait();
                    let cleanup_at_ms = current_time_ms().unwrap_or(now_ms);
                    let _ = store.mark_daemon_stopped(&owner_id, cleanup_at_ms);
                    let _ = store.release_serve_lease(&owner_id);
                    error
                },
            )?;
            Ok(CliOutput::text(format!(
                "serve started: owner={} pid={} root={}\nuse `chainbot status --json` for health and `chainbot observe` for persisted runtime output",
                owner_id,
                child.id(),
                root_layout.root.display()
            )))
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

pub(crate) fn execute_stop(request: &CliRequest) -> Result<CliOutput, UserFacingError> {
    let root_layout = request.resolve_existing_root()?;
    let definitions = request.load_root_definition_bundle()?;
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
            return Ok(CliOutput::text("stop completed: no active daemon"));
        }
        ServeLeaseState::Stale => {
            if let Some(owner_id) = status.owner_id.as_deref() {
                let _ = store.mark_daemon_stopped(owner_id, now_ms);
                let _ = store.release_serve_lease(owner_id);
            }
            return Ok(CliOutput::text(
                "stop completed: stale daemon state cleaned up",
            ));
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
            return Ok(CliOutput::text("stop completed: active lease cleared"));
        }
        return Ok(CliOutput::text("stop completed: no active daemon"));
    }

    wait_for_daemon_stop(&storage_config)?;
    Ok(CliOutput::text("stop completed: daemon shutdown requested"))
}

pub(crate) fn execute_internal_serve_daemon(
    request: &CliRequest,
) -> Result<CliOutput, UserFacingError> {
    let owner_id = request.daemon_owner_id.clone().ok_or_else(|| {
        UserFacingError::state_with_code(
            "daemon_start_failed",
            "Internal daemon start is missing the daemon owner identifier.",
        )
    })?;
    let now_ms = current_time_ms()?;
    let root_layout = request.resolve_existing_root()?;
    let definitions = request.load_root_definition_bundle()?;
    let storage_config = definitions
        .root_config
        .resolve_runtime_storage(&root_layout.root)
        .map_err(UserFacingError::from_contract)?;
    let pid = i64::from(std::process::id());

    let initial_grant = {
        let mut store = RuntimeStateStore::open(&storage_config, now_ms)
            .map_err(|error| map_runtime_state_error("open runtime state store", error))?;
        let status = store
            .inspect_daemon_status(now_ms)
            .map_err(|error| map_runtime_state_error("inspect daemon status", error))?;
        if status.owner_id.as_deref() != Some(owner_id.as_str()) {
            return Ok(CliOutput::text(String::new()));
        }
        if status.stop_requested_at_ms.is_some() {
            return Ok(CliOutput::text(String::new()));
        }
        maybe_delay_daemon_start_for_tests()?;
        let status = store
            .inspect_daemon_status(current_time_ms()?)
            .map_err(|error| map_runtime_state_error("inspect daemon status", error))?;
        if status.owner_id.as_deref() != Some(owner_id.as_str()) {
            return Ok(CliOutput::text(String::new()));
        }
        if status.stop_requested_at_ms.is_some() {
            return Ok(CliOutput::text(String::new()));
        }
        let acked_at_ms = status
            .started_at_ms
            .map(|started_at_ms| {
                current_time_ms()
                    .unwrap_or(now_ms)
                    .max(started_at_ms.saturating_add(1))
            })
            .unwrap_or_else(|| current_time_ms().unwrap_or(now_ms));
        let grant = match store
            .try_acquire_serve_lease(&owner_id, acked_at_ms, SERVE_LEASE_TTL_MS)
            .map_err(|error| map_runtime_state_error("renew daemon start lease", error))?
        {
            LeaseAcquireResult::Acquired { grant } | LeaseAcquireResult::Renewed { grant } => grant,
            LeaseAcquireResult::Rejected { .. } => return Ok(CliOutput::text(String::new())),
        };
        store
            .heartbeat_daemon(
                &owner_id,
                Some(pid),
                acked_at_ms,
                grant.expires_at_ms,
            )
            .map_err(|error| map_runtime_state_error("acknowledge daemon start", error))?;
        if store
            .daemon_stop_requested(&owner_id)
            .map_err(|error| map_runtime_state_error("inspect daemon stop request", error))?
        {
            return Ok(CliOutput::text(String::new()));
        }
        grant
    };

    let loop_result = run_internal_serve_daemon_loop(
        request,
        &storage_config,
        &owner_id,
        pid,
        initial_grant,
    );
    let stopped_at_ms = current_time_ms().unwrap_or(now_ms);
    if let Ok(mut store) = RuntimeStateStore::open(&storage_config, stopped_at_ms) {
        let _ = store.mark_daemon_stopped(&owner_id, stopped_at_ms);
        let _ = store.release_serve_lease(&owner_id);
    }

    loop_result?;
    Ok(CliOutput::text(String::new()))
}

fn spawn_serve_daemon_process(owner_id: &str) -> Result<std::process::Child, UserFacingError> {
    std::process::Command::new(std::env::current_exe().map_err(|source| {
        UserFacingError::state_with_code(
            "daemon_start_failed",
            format!("Failed to resolve the running chainbot binary for daemon spawn: {source}"),
        )
    })?)
    .args(["__serve-daemon", "--owner-id", owner_id])
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
    storage_config: &RuntimeStorageConfig,
    owner_id: &str,
    child_pid: u32,
    started_at_ms: i64,
) -> Result<(), UserFacingError> {
    let deadline_ms = started_at_ms.saturating_add(serve_start_ack_timeout_ms() as i64);
    while current_time_ms()? <= deadline_ms {
        let observed_at_ms = current_time_ms()?;
        let mut store = RuntimeStateStore::open(storage_config, observed_at_ms)
            .map_err(|error| map_runtime_state_error("open runtime state store", error))?;
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

fn wait_for_daemon_stop(storage_config: &RuntimeStorageConfig) -> Result<(), UserFacingError> {
    let started_wait_ms = current_time_ms()?;
    let deadline_ms = started_wait_ms.saturating_add(SERVE_STOP_TIMEOUT_MS as i64);
    while current_time_ms()? <= deadline_ms {
        let observed_at_ms = current_time_ms()?;
        let mut store = RuntimeStateStore::open(storage_config, observed_at_ms)
            .map_err(|error| map_runtime_state_error("open runtime state store", error))?;
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
    request: &CliRequest,
    storage_config: &RuntimeStorageConfig,
    owner_id: &str,
    pid: i64,
    initial_grant: ServeLeaseGrant,
) -> Result<(), UserFacingError> {
    let ingress_supervisor =
        TriggerIngressSupervisor::start(storage_config.clone()).map_err(map_ingress_error)?;
    let cancellation = HostCancellation::default();
    let mut external_trigger_supervisor = ExternalTriggerSupervisor::new(owner_id.to_owned())
        .with_cancellation(cancellation.clone());
    let heartbeat_started_at_ms = current_time_ms()?;
    let mut lease_heartbeat = ServeLeaseHeartbeat::start(
        storage_config.clone(),
        owner_id.to_owned(),
        pid,
        initial_grant.clone(),
        cancellation.clone(),
        heartbeat_started_at_ms,
    );
    let mut lease_supervisor = ServeLeaseSupervisor::with_grant_and_cancellation(
        storage_config.clone(),
        pid,
        initial_grant,
        heartbeat_started_at_ms,
        cancellation.clone(),
    );

    let loop_result = (|| loop {
        lease_heartbeat.check()?;
        lease_supervisor.ensure_active()?;
        let observed_at_ms = current_time_ms()?;
        let mut control_store = RuntimeStateStore::open(storage_config, observed_at_ms)
            .map_err(|error| map_runtime_state_error("open runtime state store", error))?;
        if control_store
            .daemon_stop_requested(owner_id)
            .map_err(|error| map_runtime_state_error("inspect daemon stop request", error))?
        {
            return Ok(());
        }

        lease_supervisor.maybe_renew(observed_at_ms)?;

        let mut runtime = match request.load_runtime_context() {
            Ok(runtime) => runtime,
            Err(error) => {
                let _ = ingress_supervisor.reconcile(DesiredIngressState::default());
                teardown_external_trigger_sessions(
                    &mut external_trigger_supervisor,
                    observed_at_ms,
                )?;
                let mut error_store = RuntimeStateStore::open(storage_config, current_time_ms()?)
                    .map_err(|open_error| {
                    map_runtime_state_error("open runtime state store", open_error)
                })?;
                error_store
                    .record_daemon_error(owner_id, error.error_code(), &error.to_string())
                    .map_err(|store_error| {
                        map_runtime_state_error("record daemon error", store_error)
                    })?;
                thread::sleep(std::time::Duration::from_millis(SERVE_ERROR_BACKOFF_MS));
                continue;
            }
        };
        let desired_ingress = build_desired_ingress_state(&runtime.definitions.triggers)
            .map_err(UserFacingError::from_contract)?;
        ingress_supervisor
            .reconcile(desired_ingress)
            .map_err(map_ingress_error)?;
        let mut iteration_store = RuntimeStateStore::open(&runtime.storage_config, observed_at_ms)
            .map_err(|error| map_runtime_state_error("open runtime state store", error))?;
        iteration_store
            .mark_daemon_reload(owner_id, observed_at_ms)
            .map_err(|error| map_runtime_state_error("mark daemon reload", error))?;

        match serve_once_with_lease(
            &mut runtime,
            observed_at_ms,
            &mut lease_supervisor,
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
                let mut error_store = RuntimeStateStore::open(storage_config, current_time_ms()?)
                    .map_err(|open_error| {
                    map_runtime_state_error("open runtime state store", open_error)
                })?;
                error_store
                    .record_daemon_error(owner_id, error.error_code(), &error.to_string())
                    .map_err(|store_error| {
                        map_runtime_state_error("record daemon error", store_error)
                    })?;
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
    let heartbeat_result = lease_heartbeat.shutdown();
    loop_result
        .and(teardown_result)
        .and(shutdown_result)
        .and(heartbeat_result)
}

pub(crate) fn teardown_external_trigger_sessions(
    supervisor: &mut ExternalTriggerSupervisor,
    observed_at_ms: i64,
) -> Result<(), UserFacingError> {
    supervisor
        .try_reconcile_with_process(BTreeMap::new(), observed_at_ms, |_, _| Ok(None))
        .map(|_| ())
        .map_err(UserFacingError::from_contract)
}

pub(crate) fn serve_once_with_lease(
    runtime: &mut RuntimeContext,
    accepted_at_ms: i64,
    lease_supervisor: &mut ServeLeaseSupervisor,
    external_trigger_supervisor: &mut ExternalTriggerSupervisor,
) -> Result<CliOutput, UserFacingError> {
    lease_supervisor.maybe_renew(accepted_at_ms)?;
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
    let desired_external_sessions = build_desired_external_trigger_sessions(
        &trigger_definitions,
        &trigger_manifests,
    )
    .map_err(UserFacingError::from_contract)?;
    let definitions_by_id = trigger_definitions
        .iter()
        .map(|definition| (definition.trigger_id.as_str(), definition))
        .collect::<BTreeMap<_, _>>();
    let manifests_by_id = trigger_manifests
        .iter()
        .map(|manifest| (manifest.plugin_id.as_str(), manifest))
        .collect::<BTreeMap<_, _>>();
    external_trigger_supervisor
        .try_reconcile_with_process(
            desired_external_sessions,
            accepted_at_ms,
            |spec, cancellation| {
                if spec.runtime != ExternalTriggerSessionRuntime::ProcessDaemon {
                    return Ok(None);
                }
                let definition = definitions_by_id.get(spec.trigger_id.as_str()).ok_or_else(|| {
                    crate::errors::ContractError::InvalidTriggerDefinitionField {
                        trigger_id: spec.trigger_id.clone(),
                        field: "trigger.trigger_id",
                        detail: "managed process session is missing trigger definition".to_owned(),
                    }
                })?;
                let manifest = manifests_by_id.get(spec.plugin_id.as_str()).ok_or_else(|| {
                    crate::errors::ContractError::UnknownTriggerPlugin {
                        trigger_id: spec.trigger_id.clone(),
                        plugin_id: spec.plugin_id.clone(),
                    }
                })?;
                ProcessTriggerSession::start(
                    &mut runtime.state_store,
                    definition,
                    manifest,
                    &policy,
                    spec.wasm_component.clone().unwrap_or_default(),
                    cancellation,
                )
                .map(Some)
            },
        )
        .map_err(UserFacingError::from_contract)?;
    external_trigger_supervisor
        .drain_process_sessions(
            &trigger_definitions,
            &mut runtime.state_store,
            accepted_at_ms,
        )
        .map_err(UserFacingError::from_contract)?;
    let process_poll_budget =
        external_trigger_supervisor.plan_process_polls_for_cycle(accepted_at_ms);
    let mut renew_progress = || {
        let now_ms = current_time_ms().map_err(|error| {
            TriggerPlaneError::Contract(crate::errors::ContractError::InvalidTriggerEmission {
                trigger_id: String::from("serve"),
                detail: error.to_string(),
            })
        })?;
        lease_supervisor.maybe_renew(now_ms).map_err(|error| {
            TriggerPlaneError::Contract(crate::errors::ContractError::InvalidTriggerEmission {
                trigger_id: String::from("serve"),
                detail: error.to_string(),
            })
        })
    };
    stage_process_external_trigger_sessions(
        &trigger_definitions,
        &trigger_manifests,
        &policy,
        external_trigger_supervisor,
        &process_poll_budget,
        &mut runtime.state_store,
        accepted_at_ms,
        &mut renew_progress,
    )
    .map_err(map_trigger_error)?;
    stage_wasm_external_trigger_sessions(
        &trigger_definitions,
        &trigger_manifests,
        external_trigger_supervisor,
        &mut runtime.state_store,
        accepted_at_ms,
        &mut renew_progress,
    )
    .map_err(map_trigger_error)?;

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

    let mut trigger_plane = TriggerPlane::open_with_store_acceptance_only(
        trigger_store,
        trigger_definitions,
        trigger_manifests,
        policy,
        builtin_events,
    )
    .map_err(map_trigger_error)?;

    let run_requests = trigger_plane
        .collect_run_requests_with_progress(accepted_at_ms, &mut renew_progress)
        .map_err(map_trigger_error)?;
    for inbox_id in drained_ingress.inbox_ids {
        runtime
            .state_store
            .mark_ingress_inbox_processed(&inbox_id, accepted_at_ms)
            .map_err(|error| map_runtime_state_error("mark ingress inbox processed", error))?;
    }
    if run_requests.is_empty() && replay_requests.is_empty() {
        return Ok(CliOutput::text(
            "serve completed: no accepted trigger events",
        ));
    }

    let run_requests = merge_trigger_requests(replay_requests, run_requests);

    let mut completed_runs = Vec::with_capacity(run_requests.len());
    let mut failures = Vec::new();

    for trigger_request in &run_requests {
        lease_supervisor.maybe_renew(current_time_ms()?)?;
        let normalized_request = normalized_request_from_trigger(trigger_request);
        let fence = lease_supervisor.execution_fence();
        match execute_single_run_with_fence_and_cancellation(
            runtime,
            normalized_request,
            accepted_at_ms,
            fence.as_ref(),
            &lease_supervisor.cancellation(),
        ) {
            Ok(run_result) => completed_runs.push((trigger_request, run_result)),
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

    let mut output_lines = Vec::with_capacity(completed_runs.len().saturating_add(1));
    output_lines.push(format!(
        "serve completed: executed {} accepted trigger event(s)",
        completed_runs.len()
    ));
    for (trigger_request, run_result) in completed_runs {
        output_lines.push(format!(
            "serve run: run_id={} workflow_id={} status={} trigger_id={} event_id={}",
            run_result.run_id,
            run_result.workflow_id,
            render_run_status(run_result.status),
            trigger_request.trigger_id,
            trigger_request.event_id
        ));
    }

    Ok(CliOutput::text(output_lines.join("\n")))
}

fn stage_process_external_trigger_sessions<F>(
    definitions: &[TriggerDefinition],
    manifests: &[PluginManifest],
    policy: &crate::domain::trigger::TriggerPluginHostPolicy,
    supervisor: &ExternalTriggerSupervisor,
    process_poll_budget: &[ExternalTriggerPollBudget],
    state_store: &mut RuntimeStateStore,
    staged_at_ms: i64,
    on_progress: &mut F,
) -> Result<(), TriggerPlaneError>
where
    F: FnMut() -> Result<(), TriggerPlaneError>,
{
    let definitions_by_id: BTreeMap<String, &TriggerDefinition> = definitions
        .iter()
        .map(|definition| (definition.trigger_id.clone(), definition))
        .collect();
    let manifests_by_id: BTreeMap<String, &PluginManifest> = manifests
        .iter()
        .map(|manifest| (manifest.plugin_id.clone(), manifest))
        .collect();

    for poll_budget in process_poll_budget {
        let session = supervisor
            .sessions()
            .get(&poll_budget.trigger_id)
            .ok_or_else(|| {
                TriggerPlaneError::Contract(
                    crate::errors::ContractError::InvalidTriggerDefinitionField {
                        trigger_id: poll_budget.trigger_id.clone(),
                        field: "trigger.trigger_id",
                        detail: "process polling budget references a missing supervisor session"
                            .to_owned(),
                    },
                )
            })?;
        if session.runtime != ExternalTriggerSessionRuntime::Process {
            continue;
        }

        let definition = definitions_by_id.get(&session.trigger_id).ok_or_else(|| {
            TriggerPlaneError::Contract(
                crate::errors::ContractError::InvalidTriggerDefinitionField {
                    trigger_id: session.trigger_id.clone(),
                    field: "trigger.trigger_id",
                    detail: "process supervisor session is missing trigger definition".to_owned(),
                },
            )
        })?;
        let manifest = manifests_by_id.get(&session.plugin_id).ok_or_else(|| {
            TriggerPlaneError::Contract(crate::errors::ContractError::UnknownTriggerPlugin {
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
                TriggerPlaneError::Contract(
                    crate::errors::ContractError::InvalidTriggerDefinitionField {
                        trigger_id: trigger_id.clone(),
                        field: "trigger.trigger_id",
                        detail: "wasm supervisor session is missing trigger definition".to_owned(),
                    },
                )
            })?;
        let manifest = manifests
            .iter()
            .find(|manifest| manifest.plugin_id == plugin_id)
            .ok_or_else(|| {
                TriggerPlaneError::Contract(crate::errors::ContractError::UnknownTriggerPlugin {
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
                crate::errors::ContractError::NodePluginInvalidField {
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

fn map_trigger_error(error: TriggerPlaneError) -> UserFacingError {
    match error {
        TriggerPlaneError::Contract(source) => UserFacingError::from_contract(source),
        TriggerPlaneError::RuntimeState(source) => UserFacingError::state(format!(
            "Failed to evaluate trigger-plane runtime state: {source}"
        )),
    }
}

fn render_run_status(status: RunStatus) -> &'static str {
    match status {
        RunStatus::Pending => "pending",
        RunStatus::Running => "running",
        RunStatus::Succeeded => "succeeded",
        RunStatus::Failed => "failed",
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{Instant, SystemTime, UNIX_EPOCH};

    use super::*;
    use crate::infrastructure::config::RuntimeStorageBackend;

    #[test]
    fn heartbeat_publishes_lease_loss_when_a_successor_owns_the_lease() {
        let now_ms = current_time_ms().expect("test clock should be available");
        let sqlite_path = unique_sqlite_path("heartbeat-lease-loss");
        let storage_config = RuntimeStorageConfig {
            backend: RuntimeStorageBackend::Local {
                database_path: sqlite_path.clone(),
            },
            history_retention: None,
            raw_debug_enabled: false,
            raw_debug_artifacts_dir: None,
        };
        let mut store = RuntimeStateStore::open(&storage_config, now_ms)
            .expect("heartbeat test store should open");
        let initial_grant = match store
            .try_acquire_serve_lease("owner-a", now_ms, SERVE_LEASE_TTL_MS)
            .expect("initial lease should acquire")
        {
            LeaseAcquireResult::Acquired { grant } => grant,
            other => panic!("expected initial grant, got {other:?}"),
        };
        assert!(store
            .release_serve_lease("owner-a")
            .expect("initial lease should release"));
        assert!(matches!(
            store
                .try_acquire_serve_lease("owner-b", now_ms + 1, SERVE_LEASE_TTL_MS)
                .expect("successor lease should acquire"),
            LeaseAcquireResult::Acquired { .. }
        ));
        drop(store);

        let cancellation = HostCancellation::default();
        let mut heartbeat = ServeLeaseHeartbeat::start(
            storage_config,
            String::from("owner-a"),
            i64::from(std::process::id()),
            initial_grant,
            cancellation.clone(),
            now_ms.saturating_sub(SERVE_LEASE_RENEW_INTERVAL_MS),
        );
        let deadline = Instant::now() + Duration::from_secs(1);
        let error = loop {
            match heartbeat.check() {
                Err(error) => break error,
                Ok(()) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
                Ok(()) => panic!("heartbeat should report lease loss"),
            }
        };

        assert_eq!(error.error_code(), "serve_lease_lost");
        assert_eq!(
            cancellation.reason(),
            Some(HostCancellationReason::LeaseLost)
        );
        heartbeat
            .shutdown()
            .expect("heartbeat should join after lease loss");
        let _ = std::fs::remove_file(sqlite_path);
    }

    fn unique_sqlite_path(prefix: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after UNIX epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("chainbot-{prefix}-{nonce}.sqlite3"))
    }
}
