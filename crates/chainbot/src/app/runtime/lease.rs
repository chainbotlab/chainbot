//! [INPUT]
//! A daemon store connection, lease owner, process identifier, and observation timestamps.
//!
//! [OUTPUT]
//! Provides lease-fenced daemon turns, background renewal, cancellation, and joined shutdown.
//!
//! [ROLE]
//! Owns one persistent control store and one renewal state shared by daemon turns and heartbeat work.

use std::sync::{mpsc, Arc, Mutex, MutexGuard};
use std::thread;
use std::time::Duration;

use super::context::{current_time_ms, map_runtime_state_error};
use crate::domain::state::{LeaseAcquireResult, RunExecutionFence, ServeLeaseGrant};
use crate::errors::UserFacingError;
use crate::infrastructure::state::RuntimeStateStore;
use crate::plugin::{HostCancellation, HostCancellationReason};

pub(super) const SERVE_LEASE_TTL_MS: i64 = 30_000;
const SERVE_LEASE_RENEW_INTERVAL_MS: i64 = 10_000;
const SERVE_LEASE_RETRY_INTERVAL_MS: i64 = 1_000;
const SERVE_HEARTBEAT_STOP_POLL_MS: u64 = 250;

#[derive(Debug)]
pub(super) struct DaemonLeaseSession {
    state: Arc<Mutex<LeaseState>>,
    fence: RunExecutionFence,
    cancellation: HostCancellation,
    heartbeat: Option<LeaseHeartbeat>,
    closed: bool,
}

#[derive(Debug)]
struct LeaseState {
    store: RuntimeStateStore,
    grant: ServeLeaseGrant,
    pid: i64,
    next_renew_at_ms: i64,
    last_renew_error: Option<String>,
    cancellation: HostCancellation,
}

impl DaemonLeaseSession {
    pub(super) fn acquire(
        mut store: RuntimeStateStore,
        owner_id: &str,
        pid: i64,
        now_ms: i64,
    ) -> Result<Self, UserFacingError> {
        let grant = match store.try_acquire_serve_lease(owner_id, now_ms, SERVE_LEASE_TTL_MS)
            .map_err(|error| map_runtime_state_error("acquire daemon lease", error))?
        {
            LeaseAcquireResult::Acquired { grant } | LeaseAcquireResult::Renewed { grant } => grant,
            LeaseAcquireResult::Rejected { current_owner, expires_at_ms } => {
                return Err(lease_lost(format!(
                    "Serve lease acquisition was rejected (current owner: {current_owner}, expires_at_ms: {expires_at_ms})."
                )));
            }
        };
        let cancellation = HostCancellation::default();
        let fence = RunExecutionFence {
            owner_id: grant.owner_id.clone(),
            lease_generation: grant.generation,
        };
        let session = Self {
            state: Arc::new(Mutex::new(LeaseState {
                store,
                grant,
                pid,
                next_renew_at_ms: now_ms.saturating_add(SERVE_LEASE_RENEW_INTERVAL_MS),
                last_renew_error: None,
                cancellation: cancellation.clone(),
            })),
            fence,
            cancellation,
            heartbeat: None,
            closed: false,
        };
        lock_state(&session.state)?.store.heartbeat_daemon(
            owner_id, Some(pid), now_ms, now_ms.saturating_add(SERVE_LEASE_TTL_MS),
        ).map_err(|error| map_runtime_state_error("acknowledge daemon start", error))?;
        Ok(session)
    }

    pub(super) fn start_heartbeat(&mut self) {
        if self.heartbeat.is_some() {
            return;
        }
        let state = Arc::clone(&self.state);
        let cancellation = self.cancellation.clone();
        let (stop_tx, stop_rx) = mpsc::sync_channel(1);
        let (error_tx, error_rx) = mpsc::sync_channel(1);
        let handle = thread::spawn(move || {
            let result = (|| {
                while !cancellation.is_cancelled() {
                    lock_state(&state)?.renew(current_time_ms()?)?;
                    match stop_rx.recv_timeout(Duration::from_millis(SERVE_HEARTBEAT_STOP_POLL_MS)) {
                        Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                    }
                }
                Ok::<(), UserFacingError>(())
            })();
            if let Err(error) = result {
                cancellation.lease_lost();
                let _ = error_tx.try_send(error);
            }
        });
        self.heartbeat = Some(LeaseHeartbeat { stop_tx, error_rx, handle });
    }

    pub(super) fn cancellation(&self) -> HostCancellation {
        self.cancellation.clone()
    }

    pub(super) fn fence(&self) -> RunExecutionFence {
        self.fence.clone()
    }

    pub(super) fn check(&self, now_ms: i64) -> Result<(), UserFacingError> {
        if let Some(heartbeat) = &self.heartbeat {
            match heartbeat.error_rx.try_recv() {
                Ok(error) => return Err(error),
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) if !self.cancellation.is_cancelled() => {
                    self.cancellation.lease_lost();
                    return Err(UserFacingError::unavailable_with_code(
                        "serve_heartbeat_stopped", "The serve lease heartbeat stopped unexpectedly.",
                    ));
                }
                Err(mpsc::TryRecvError::Disconnected) => {}
            }
        }
        match self.cancellation.reason() {
            Some(HostCancellationReason::LeaseLost) => return Err(lease_lost(
                "Serve lease ownership was lost; active runtime work has been cancelled.",
            )),
            Some(HostCancellationReason::ShuttingDown) => return Err(UserFacingError::unavailable_with_code(
                "daemon_shutting_down", "The daemon is shutting down; active runtime work has been cancelled.",
            )),
            None => {}
        }
        lock_state(&self.state)?.renew(now_ms)
    }

    pub(super) fn poll_turn(&self, now_ms: i64) -> Result<bool, UserFacingError> {
        self.check(now_ms)?;
        lock_state(&self.state)?.store.daemon_stop_requested(&self.fence.owner_id)
            .map(|stop_requested| !stop_requested)
            .map_err(|error| map_runtime_state_error("inspect daemon stop request", error))
    }

    pub(super) fn mark_reload(&self, now_ms: i64) -> Result<(), UserFacingError> {
        lock_state(&self.state)?.store.mark_daemon_reload(&self.fence.owner_id, now_ms)
            .map_err(|error| map_runtime_state_error("mark daemon reload", error))
    }

    pub(super) fn record_error(&self, error: &UserFacingError) -> Result<(), UserFacingError> {
        lock_state(&self.state)?.store.record_daemon_error(
            &self.fence.owner_id, error.error_code(), &error.to_string(),
        ).map_err(|error| map_runtime_state_error("record daemon error", error))
    }

    pub(super) fn shutdown(&mut self, now_ms: i64) -> Result<(), UserFacingError> {
        if self.closed {
            return Ok(());
        }
        self.closed = true;
        self.cancellation.shutdown();
        let heartbeat_result = self.heartbeat.take().map_or(Ok(()), |heartbeat| {
            let _ = heartbeat.stop_tx.try_send(());
            heartbeat.handle.join().map_err(|_| UserFacingError::unavailable_with_code(
                "serve_heartbeat_join_failed", "The serve lease heartbeat thread panicked during shutdown.",
            ))
        });
        let mut state = lock_state(&self.state)?;
        let stopped = state.store.mark_daemon_stopped(&self.fence.owner_id, now_ms)
            .map_err(|error| map_runtime_state_error("mark daemon stopped", error));
        let released = state.store.release_serve_lease(&self.fence.owner_id)
            .map_err(|error| map_runtime_state_error("release daemon lease", error));
        heartbeat_result.and(stopped.map(|_| ())).and(released.map(|_| ()))
    }
}

impl Drop for DaemonLeaseSession {
    fn drop(&mut self) {
        let _ = self.shutdown(current_time_ms().unwrap_or_default());
    }
}

impl LeaseState {
    fn renew(&mut self, now_ms: i64) -> Result<(), UserFacingError> {
        if now_ms >= self.grant.expires_at_ms {
            self.cancellation.lease_lost();
            let message = self.last_renew_error.take().map_or_else(
                || String::from("The serve lease expired before it could be renewed."),
                |error| format!("The serve lease expired after a storage failure: {error}"),
            );
            return Err(lease_lost(message));
        }
        if now_ms < self.next_renew_at_ms {
            return Ok(());
        }
        let result = (|| {
            let grant = match self.store.try_acquire_serve_lease(
                &self.grant.owner_id, now_ms, SERVE_LEASE_TTL_MS,
            )? {
                LeaseAcquireResult::Renewed { grant } if grant.generation == self.grant.generation => grant,
                _ => {
                    self.cancellation.lease_lost();
                    return Ok(false);
                }
            };
            self.store.heartbeat_daemon(&grant.owner_id, Some(self.pid), now_ms, grant.expires_at_ms)?;
            self.grant = grant;
            Ok::<bool, crate::infrastructure::state::RuntimeStateError>(true)
        })();
        match result {
            Ok(true) => {
                self.last_renew_error = None;
                self.next_renew_at_ms = now_ms.saturating_add(SERVE_LEASE_RENEW_INTERVAL_MS);
                Ok(())
            }
            Ok(false) => Err(lease_lost("Serve lease renewal was rejected; active runtime work has been cancelled.")),
            Err(error) => {
                self.last_renew_error = Some(error.to_string());
                self.next_renew_at_ms = now_ms.saturating_add(SERVE_LEASE_RETRY_INTERVAL_MS)
                    .min(self.grant.expires_at_ms);
                Ok(())
            }
        }
    }
}

fn lock_state(state: &Mutex<LeaseState>) -> Result<MutexGuard<'_, LeaseState>, UserFacingError> {
    state.lock().map_err(|_| lease_lost("The serve lease state was poisoned by a failed heartbeat."))
}

fn lease_lost(message: impl Into<String>) -> UserFacingError {
    UserFacingError::conflict_with_code("serve_lease_lost", message)
}

#[derive(Debug)]
struct LeaseHeartbeat {
    stop_tx: mpsc::SyncSender<()>,
    error_rx: mpsc::Receiver<UserFacingError>,
    handle: thread::JoinHandle<()>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::config::{RuntimeStorageBackend, RuntimeStorageConfig};
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_store() -> (RuntimeStateStore, RuntimeStorageConfig) {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let config = RuntimeStorageConfig {
            backend: RuntimeStorageBackend::Local {
                database_path: std::env::temp_dir().join(format!("chainbot-lease-{nonce}.sqlite3")),
            },
            history_retention: None,
            raw_debug_enabled: false,
            raw_debug_artifacts_dir: None,
        };
        (RuntimeStateStore::open(&config, 1_000).unwrap(), config)
    }

    fn database_path(config: &RuntimeStorageConfig) -> &PathBuf {
        let RuntimeStorageBackend::Local { database_path } = &config.backend else { unreachable!() };
        database_path
    }

    #[test]
    fn rejected_acquisition_preserves_the_active_owner() {
        let (mut store, config) = test_store();
        store.try_acquire_serve_lease("owner-b", 1_000, SERVE_LEASE_TTL_MS).unwrap();
        let error = DaemonLeaseSession::acquire(store, "owner-a", 7, 2_000).unwrap_err();
        assert_eq!(error.error_code(), "serve_lease_lost");
        let mut observer = RuntimeStateStore::open(&config, 2_000).unwrap();
        assert_eq!(observer.inspect_serve_lease(2_000).unwrap().owner_id.as_deref(), Some("owner-b"));
    }

    #[test]
    fn dropping_a_session_releases_its_lease() {
        let (store, config) = test_store();
        let session = DaemonLeaseSession::acquire(store, "owner-a", 7, 1_000).unwrap();
        let cancellation = session.cancellation();
        drop(session);
        assert_eq!(cancellation.reason(), Some(HostCancellationReason::ShuttingDown));
        let mut observer = RuntimeStateStore::open(&config, 2_000).unwrap();
        assert!(matches!(
            observer.try_acquire_serve_lease("owner-b", 2_000, SERVE_LEASE_TTL_MS).unwrap(),
            LeaseAcquireResult::Acquired { .. }
        ));
    }

    #[test]
    fn poll_turn_renews_the_same_generation_and_observes_stop() {
        let (store, config) = test_store();
        let mut session = DaemonLeaseSession::acquire(store, "owner-a", 7, 1_000).unwrap();
        let fence = session.fence();
        let mut observer = RuntimeStateStore::open(&config, 1_000).unwrap();
        observer.register_daemon_start("owner-a", Some(7), 1_000, 31_000).unwrap();
        assert!(session.poll_turn(11_000).unwrap());
        let snapshot = observer.inspect_serve_lease(11_000).unwrap();
        assert_eq!(snapshot.owner_id.as_deref(), Some(fence.owner_id.as_str()));
        let connection = rusqlite::Connection::open(database_path(&config)).unwrap();
        let generation: i64 = connection.query_row("SELECT generation FROM serve_leases", [], |row| row.get(0)).unwrap();
        assert_eq!(generation as u64, fence.lease_generation);
        assert_eq!(snapshot.expires_at_ms, Some(41_000));
        assert_eq!(observer.inspect_daemon_status(11_000).unwrap().last_heartbeat_at_ms, Some(11_000));
        observer.request_daemon_stop(11_001).unwrap();
        assert!(!session.poll_turn(11_001).unwrap());
        session.shutdown(11_002).unwrap();
        assert_eq!(observer.inspect_serve_lease(11_002).unwrap().state, crate::domain::state::ServeLeaseState::Idle);
    }

    #[test]
    fn successor_lease_cancels_work_and_survives_old_session_shutdown() {
        let (store, config) = test_store();
        let mut session = DaemonLeaseSession::acquire(store, "owner-a", 7, 1_000).unwrap();
        let cancellation = session.cancellation();
        let mut successor = RuntimeStateStore::open(&config, 1_000).unwrap();
        successor.release_serve_lease("owner-a").unwrap();
        successor.try_acquire_serve_lease("owner-b", 2_000, SERVE_LEASE_TTL_MS).unwrap();
        let error = session.check(11_000).unwrap_err();
        assert_eq!(error.error_code(), "serve_lease_lost");
        assert_eq!(cancellation.reason(), Some(HostCancellationReason::LeaseLost));
        session.shutdown(11_001).unwrap();
        assert_eq!(successor.inspect_serve_lease(11_001).unwrap().owner_id.as_deref(), Some("owner-b"));
    }

    #[test]
    fn expired_session_cannot_reacquire_a_new_generation() {
        let (store, config) = test_store();
        let mut session = DaemonLeaseSession::acquire(store, "owner-a", 7, 1_000).unwrap();
        let fence = session.fence();
        assert_eq!(session.check(31_000).unwrap_err().error_code(), "serve_lease_lost");
        let connection = rusqlite::Connection::open(database_path(&config)).unwrap();
        let generation: i64 = connection.query_row("SELECT generation FROM serve_leases", [], |row| row.get(0)).unwrap();
        assert_eq!(generation as u64, fence.lease_generation);
        session.shutdown(31_001).unwrap();
    }

    #[test]
    fn storage_failure_retries_until_the_existing_lease_expires() {
        let (store, config) = test_store();
        let mut session = DaemonLeaseSession::acquire(store, "owner-a", 7, 1_000).unwrap();
        let connection = rusqlite::Connection::open(database_path(&config)).unwrap();
        connection.execute("ALTER TABLE serve_leases RENAME TO unavailable_leases", []).unwrap();
        session.check(11_000).expect("transient failure must retain the valid lease");
        assert!(!session.cancellation().is_cancelled());
        connection.execute("ALTER TABLE unavailable_leases RENAME TO serve_leases", []).unwrap();
        session.check(12_000).expect("renewal must recover on the next retry");
        let mut observer = RuntimeStateStore::open(&config, 12_000).unwrap();
        assert_eq!(observer.inspect_serve_lease(12_000).unwrap().expires_at_ms, Some(42_000));
        connection.execute("ALTER TABLE serve_leases RENAME TO unavailable_leases", []).unwrap();
        session.check(22_000).unwrap();
        let error = session.check(42_000).unwrap_err();
        assert_eq!(error.error_code(), "serve_lease_lost");
        assert!(error.to_string().contains("serve_leases"));
        assert_eq!(session.cancellation().reason(), Some(HostCancellationReason::LeaseLost));
        connection.execute("ALTER TABLE unavailable_leases RENAME TO serve_leases", []).unwrap();
        session.shutdown(42_001).unwrap();
    }

    #[test]
    fn background_renewal_cancels_work_without_a_daemon_turn() {
        let (store, config) = test_store();
        let now_ms = current_time_ms().unwrap();
        let mut session = DaemonLeaseSession::acquire(store, "owner-a", 7, now_ms - SERVE_LEASE_RENEW_INTERVAL_MS).unwrap();
        let mut successor = RuntimeStateStore::open(&config, now_ms).unwrap();
        successor.release_serve_lease("owner-a").unwrap();
        successor.try_acquire_serve_lease("owner-b", now_ms, SERVE_LEASE_TTL_MS).unwrap();
        session.start_heartbeat();
        let error = session.heartbeat.as_ref().unwrap().error_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(error.error_code(), "serve_lease_lost");
        assert_eq!(session.cancellation().reason(), Some(HostCancellationReason::LeaseLost));
        session.shutdown(now_ms).unwrap();
    }
}
