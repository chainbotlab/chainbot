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
use std::sync::mpsc::{self, TryRecvError};
use std::thread;
use std::time::Duration;

use super::context::{
    current_time_ms, map_ingress_error, map_runtime_state_error, resolve_existing_root, RuntimeContext,
};
use super::external_triggers::ExternalTriggerSupervisor;
use super::plugin_activation::parse_trigger_plugin_activation_bindings;
use super::run::execute_single_run_with_fence_and_cancellation;
use crate::app::definitions::load_root_definition_bundle;
use crate::domain::runtime::NormalizedRunRequest;
use crate::domain::state::{
    LeaseAcquireResult, RunExecutionFence, ServeLeaseGrant, ServeLeaseState, TriggerEventRecord,
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
use crate::plugin::{HostCancellation, HostCancellationReason, PluginKind, PluginManifest};

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

    wait_for_daemon_stop(&storage_config)?;
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

    let initial_grant = {
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
        let grant = match store
            .try_acquire_serve_lease(&owner_id, acked_at_ms, SERVE_LEASE_TTL_MS)
            .map_err(|error| map_runtime_state_error("renew daemon start lease", error))?
        {
            LeaseAcquireResult::Acquired { grant } | LeaseAcquireResult::Renewed { grant } => grant,
            LeaseAcquireResult::Rejected { .. } => return Ok(()),
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
            return Ok(());
        }
        grant
    };

    let loop_result = run_internal_serve_daemon_loop(
        root_layout,
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
    Ok(())
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
    root_layout: &RootLayout,
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

        let mut runtime = match resolve_existing_root(root_layout).and_then(RuntimeContext::load) {
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
        .shutdown(observed_at_ms)
        .map_err(UserFacingError::from_contract)
}

pub(crate) fn serve_once_with_lease(
    runtime: &mut RuntimeContext,
    accepted_at_ms: i64,
    lease_supervisor: &mut ServeLeaseSupervisor,
    external_trigger_supervisor: &mut ExternalTriggerSupervisor,
) -> Result<Vec<TriggerRunRequest>, UserFacingError> {
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
    external_trigger_supervisor
        .run_cycle(
            &trigger_definitions,
            &trigger_manifests,
            &policy,
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

    let mut trigger_plane =
        TriggerPlane::open_with_store(trigger_store, trigger_definitions, builtin_events)
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
        return Ok(Vec::new());
    }

    let run_requests = merge_trigger_requests(replay_requests, run_requests);

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

#[cfg(test)]
mod runtime_tests {
    use super::*;

    use std::fs;
    use std::sync::{Mutex, OnceLock};

    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use super::super::context::resolve_existing_root;
    use crate::domain::state::StagedTriggerEventRecord;
    use crate::infrastructure::config::RuntimeStorageBackend;

    const CHAINBOT_SECRET_DECRYPTOR_ENV: &str = "CHAINBOT_SECRET_DECRYPTOR";
    const SECRET_DECRYPTOR_PLAINTEXT: &str = "plaintext";

    const TEST_SERVE_LEASE_TTL_MS: i64 = 30_000;

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

        let mut lease_supervisor = ServeLeaseSupervisor::new(
            runtime.storage_config.clone(),
            String::from("test-owner"),
            i64::from(std::process::id()),
            1_710_300_000_000,
        );
        let mut external_trigger_supervisor = ExternalTriggerSupervisor::new("test-owner");
        let serve_output = serve_once_with_lease(
            &mut runtime,
            1_710_300_000_000,
            &mut lease_supervisor,
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
        let mut lease_supervisor = ServeLeaseSupervisor::new(
            runtime.storage_config.clone(),
            String::from("test-owner"),
            i64::from(std::process::id()),
            1_710_300_100_000,
        );
        let mut external_trigger_supervisor = ExternalTriggerSupervisor::new("test-owner");
        let serve_output = serve_once_with_lease(
            &mut runtime,
            1_710_300_100_000,
            &mut lease_supervisor,
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

        let mut lease_supervisor = ServeLeaseSupervisor::new(
            runtime.storage_config.clone(),
            String::from("test-owner"),
            i64::from(std::process::id()),
            1_710_300_150_000,
        );
        let mut external_trigger_supervisor = ExternalTriggerSupervisor::new("test-owner");
        let serve_output = serve_once_with_lease(
            &mut runtime,
            1_710_300_150_010,
            &mut lease_supervisor,
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
    fn serve_lease_supervisor_renews_same_owner_lease() {
        let root = prepare_fixture_root("success", "cli-serve-lease-renewal");
        let storage_config = RuntimeStorageConfig {
            backend: RuntimeStorageBackend::Local {
                database_path: root.join("state").join("runtime.sqlite3"),
            },
            history_retention: None,
            raw_debug_enabled: false,
            raw_debug_artifacts_dir: None,
        };
        let mut state_store = RuntimeStateStore::open(&storage_config, 1_710_300_200_000)
            .expect("runtime state store should open");
        assert!(matches!(
            state_store
                .try_acquire_serve_lease("owner-renew", 1_710_300_200_000, TEST_SERVE_LEASE_TTL_MS,)
                .expect("initial lease should acquire"),
            crate::domain::state::LeaseAcquireResult::Acquired { .. }
        ));

        let mut supervisor = ServeLeaseSupervisor::new(
            storage_config,
            String::from("owner-renew"),
            i64::from(std::process::id()),
            1_710_300_200_000,
        );
        supervisor
            .maybe_renew(1_710_300_210_100)
            .expect("lease renewal should succeed for same owner");

        let snapshot = state_store
            .inspect_serve_lease(1_710_300_210_100)
            .expect("lease snapshot should load after renewal");
        assert!(matches!(
            snapshot.state,
            crate::domain::state::ServeLeaseState::Active
        ));
        assert_eq!(snapshot.owner_id.as_deref(), Some("owner-renew"));
        assert_eq!(snapshot.expires_at_ms, Some(1_710_300_240_100));
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
