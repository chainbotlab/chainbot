//! [INPUT]
//! Runtime storage backend configs, temporary SQLite roots, optional PostgreSQL test URL, and structured runtime-state records.
//!
//! [OUTPUT]
//! Verifies SQLite and PostgreSQL `RuntimeStateStore` backends share the same lease, run summary, trigger snapshot, checkpoint, and trigger-history semantics.
//!
//! [ROLE]
//! Guards DB-primary runtime-state parity across supported backends.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use chainbot::config::{
    RuntimeHistoryRetentionPolicy, RuntimeStorageBackend, RuntimeStorageConfig,
};
use chainbot::state::{
    IngressInboxRecord, LeaseAcquireResult, RunRecordSummary, RunStatus, TriggerCheckpointRecord,
    TriggerEventRecord, TriggerSnapshotRecord,
};
use chainbot::state_db::RuntimeStateStore;
use postgres::{Client, NoTls};

const TEST_POSTGRES_URL_ENV: &str = "CHAINBOT_TEST_POSTGRES_URL";

#[test]
fn runtime_state_backends_share_core_semantics() {
    let _guard = backend_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());

    for backend in available_backends() {
        backend.ensure_initialized();
        backend.reset();
        let mut store =
            RuntimeStateStore::open(&backend.config, 1_710_900_000_000).unwrap_or_else(|error| {
                panic!("{} store should reopen after reset: {error}", backend.name)
            });

        assert_eq!(
            store
                .inspect_serve_lease(1_710_900_000_000)
                .expect("lease inspection should succeed")
                .state,
            chainbot::state::ServeLeaseState::Idle,
            "{} lease should start idle",
            backend.name,
        );

        let lease_result = store
            .try_acquire_serve_lease("owner-a", 1_710_900_000_000, 30_000)
            .expect("lease acquisition should succeed");
        assert!(matches!(lease_result, LeaseAcquireResult::Acquired));
        store
            .register_daemon_start("owner-a", Some(321), 1_710_900_000_000, 1_710_900_030_000)
            .expect("daemon session start should persist");
        let daemon_status = store
            .inspect_daemon_status(1_710_900_000_500)
            .expect("daemon status should inspect after start");
        assert_eq!(
            daemon_status.state,
            chainbot::state::ServeLeaseState::Active
        );
        assert_eq!(daemon_status.owner_id.as_deref(), Some("owner-a"));
        assert_eq!(daemon_status.pid, Some(321));
        let rejected = store
            .try_acquire_serve_lease("owner-b", 1_710_900_001_000, 30_000)
            .expect("conflicting lease acquisition should succeed with rejection result");
        assert!(matches!(rejected, LeaseAcquireResult::Rejected { .. }));
        store
            .request_daemon_stop(1_710_900_001_100)
            .expect("daemon stop request should persist");
        assert!(store
            .daemon_stop_requested("owner-a")
            .expect("daemon stop request should be visible"));
        store
            .mark_daemon_stopped("owner-a", 1_710_900_001_200)
            .expect("daemon stop completion should persist");
        let stopped_status = store
            .inspect_daemon_status(1_710_900_001_300)
            .expect("daemon status should inspect after stop");
        assert_eq!(stopped_status.state, chainbot::state::ServeLeaseState::Idle);
        assert!(store
            .release_serve_lease("owner-a")
            .expect("lease release should succeed"));

        store
            .write_run_summary(&RunRecordSummary {
                schema_version: "1.0.0".to_owned(),
                run_id: format!("{}-run-1", backend.slug),
                workflow_id: "wf-alpha".to_owned(),
                status: RunStatus::Running,
                started_at_ms: 1_710_900_002_000,
                finished_at_ms: None,
            })
            .expect("run summary upsert should succeed");
        store
            .write_run_summary(&RunRecordSummary {
                schema_version: "1.0.0".to_owned(),
                run_id: format!("{}-run-1", backend.slug),
                workflow_id: "wf-alpha".to_owned(),
                status: RunStatus::Succeeded,
                started_at_ms: 1_710_900_002_000,
                finished_at_ms: Some(1_710_900_003_000),
            })
            .expect("run summary update should succeed");
        let summaries = store
            .list_run_summaries()
            .expect("run summaries should list");
        assert_eq!(
            summaries.len(),
            1,
            "{} should keep one upserted run",
            backend.name
        );
        assert_eq!(summaries[0].status, RunStatus::Succeeded);

        let record = TriggerEventRecord {
            schema_version: "1.0.0".to_owned(),
            run_id: format!("{}-trigger-run-1", backend.slug),
            sequence: 1,
            trigger_id: "tr-alpha".to_owned(),
            workflow_id: "wf-alpha".to_owned(),
            event_id: format!("{}-event-1", backend.slug),
            checkpoint: Some("cp-1".to_owned()),
            source: "builtin.market".to_owned(),
            accepted_at_ms: 1_710_900_004_000,
            payload: serde_json::json!({"price": 101}),
            dedup_key: Some(format!("{}-dedup", backend.slug)),
            dedup_expires_at_ms: Some(1_710_900_014_000),
            cooldown_key: Some(format!("{}-cooldown", backend.slug)),
            cooldown_expires_at_ms: Some(1_710_900_024_000),
        };
        store
            .write_trigger_record(&record)
            .expect("trigger record write should succeed");
        let records = store
            .load_trigger_records_after_sequence("tr-alpha", 0)
            .expect("trigger records should load after sequence");
        assert_eq!(
            records.len(),
            1,
            "{} should return one trigger record",
            backend.name
        );
        assert_eq!(records[0].event_id, record.event_id);
        assert!(!store
            .dedup_is_ready(record.dedup_key.as_deref().unwrap(), 1_710_900_005_000)
            .expect("dedup readiness query should succeed"));
        assert!(store
            .dedup_is_ready(record.dedup_key.as_deref().unwrap(), 1_710_900_015_000)
            .expect("expired dedup readiness query should succeed"));
        assert!(!store
            .cooldown_is_ready(record.cooldown_key.as_deref().unwrap(), 1_710_900_005_000)
            .expect("cooldown readiness query should succeed"));
        let replayable_records = store
            .list_replayable_trigger_records(10)
            .expect("replayable trigger records should list");
        assert_eq!(replayable_records.len(), 1);
        assert_eq!(replayable_records[0].run_id, record.run_id);

        let mut snapshot = TriggerSnapshotRecord::new("tr-alpha");
        snapshot.apply_record(&record);
        store
            .write_trigger_snapshot(&snapshot)
            .expect("trigger snapshot write should succeed");
        let loaded_snapshot = store
            .read_trigger_snapshot("tr-alpha")
            .expect("trigger snapshot read should succeed")
            .expect("trigger snapshot should exist");
        assert_eq!(
            loaded_snapshot, snapshot,
            "{} snapshot should roundtrip",
            backend.name
        );
        assert_eq!(
            store
                .list_trigger_snapshots()
                .expect("trigger snapshots should list")
                .len(),
            1,
            "{} should list one trigger snapshot",
            backend.name,
        );

        let checkpoint = TriggerCheckpointRecord {
            schema_version: "1.0.0".to_owned(),
            trigger_id: "tr-alpha".to_owned(),
            checkpoint: "cp-1".to_owned(),
            acked_at_ms: 1_710_900_004_000,
        };
        store
            .write_trigger_checkpoint(&checkpoint)
            .expect("trigger checkpoint write should succeed");
        let loaded_checkpoint = store
            .read_trigger_checkpoint("tr-alpha")
            .expect("trigger checkpoint read should succeed")
            .expect("trigger checkpoint should exist");
        assert_eq!(
            loaded_checkpoint, checkpoint,
            "{} checkpoint should roundtrip",
            backend.name
        );
        store
            .write_run_summary(&RunRecordSummary {
                schema_version: String::from("1.0.0"),
                run_id: record.run_id.clone(),
                workflow_id: record.workflow_id.clone(),
                status: RunStatus::Failed,
                started_at_ms: record.accepted_at_ms,
                finished_at_ms: Some(record.accepted_at_ms.saturating_add(1_000)),
            })
            .expect("run summary for replay suppression should persist");
        assert!(store
            .list_replayable_trigger_records(10)
            .expect("replayable trigger records should be empty when run summary exists")
            .is_empty());

        let inbox_record = IngressInboxRecord {
            schema_version: String::from("1.0.0"),
            inbox_id: format!("{}-inbox-1", backend.slug),
            trigger_id: String::from("tr-alpha"),
            workflow_id: String::from("wf-alpha"),
            transport_kind: String::from("webhook"),
            ingress_event_id: format!("{}-evt-1", backend.slug),
            source: String::from("webhook"),
            route_path: String::from("/hook"),
            http_method: Some(String::from("POST")),
            received_at_ms: 1_710_900_005_000,
            payload: serde_json::json!({"ok": true}),
            headers: std::collections::BTreeMap::from([(
                String::from("x-test"),
                String::from("1"),
            )]),
            remote_addr: Some(String::from("127.0.0.1:4000")),
            processed_at_ms: None,
            last_error: None,
        };
        assert!(store
            .append_ingress_inbox_record(&inbox_record)
            .expect("ingress inbox insert should succeed"));
        assert!(!store
            .append_ingress_inbox_record(&inbox_record)
            .expect("duplicate ingress inbox insert should be ignored"));
        let pending = store
            .list_pending_ingress_inbox_records("tr-alpha", 10)
            .expect("pending ingress inbox records should list");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].ingress_event_id, inbox_record.ingress_event_id);
        store
            .mark_ingress_inbox_processed(&inbox_record.inbox_id, 1_710_900_006_000)
            .expect("mark ingress inbox processed should succeed");
        assert!(store
            .list_pending_ingress_inbox_records("tr-alpha", 10)
            .expect("processed ingress inbox rows should no longer be pending")
            .is_empty());

        backend.reset();
    }
}

#[test]
fn runtime_state_backends_archive_expired_history() {
    let _guard = backend_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());

    for backend in available_backends() {
        backend.ensure_initialized();
        backend.reset();
        let mut store =
            RuntimeStateStore::open(&backend.config, 1_710_910_000_000).unwrap_or_else(|error| {
                panic!("{} store should reopen after reset: {error}", backend.name)
            });

        store
            .write_run_summary(&RunRecordSummary {
                schema_version: "1.0.0".to_owned(),
                run_id: format!("{}-old-run", backend.slug),
                workflow_id: "wf-alpha".to_owned(),
                status: RunStatus::Succeeded,
                started_at_ms: 1_710_000_000_000,
                finished_at_ms: Some(1_710_000_010_000),
            })
            .expect("expired run summary should persist");
        store
            .append_workflow_log_entry(
                &format!("{}-old-run", backend.slug),
                "run_finished",
                "expired log",
                1_710_000_010_000,
            )
            .expect("expired log should persist");
        store
            .write_trigger_record(&TriggerEventRecord {
                schema_version: "1.0.0".to_owned(),
                run_id: format!("{}-old-run", backend.slug),
                sequence: 1,
                trigger_id: "tr-alpha".to_owned(),
                workflow_id: "wf-alpha".to_owned(),
                event_id: format!("{}-old-event", backend.slug),
                checkpoint: None,
                source: "builtin.market".to_owned(),
                accepted_at_ms: 1_710_000_005_000,
                payload: serde_json::json!({"price": 88}),
                dedup_key: None,
                dedup_expires_at_ms: None,
                cooldown_key: None,
                cooldown_expires_at_ms: None,
            })
            .expect("expired trigger record should persist");

        let archived = store
            .apply_history_retention(
                &RuntimeHistoryRetentionPolicy {
                    run_retention_ms: Some(60_000),
                    workflow_log_retention_ms: Some(60_000),
                    trigger_event_retention_ms: Some(60_000),
                },
                1_710_910_000_000,
            )
            .expect("retention should archive expired history");
        assert_eq!(
            archived.archived_run_summaries, 1,
            "{} run should archive",
            backend.name
        );
        assert_eq!(
            archived.archived_workflow_logs, 1,
            "{} log should archive",
            backend.name
        );
        assert_eq!(
            archived.archived_trigger_events, 1,
            "{} trigger event should archive",
            backend.name
        );

        assert!(
            store
                .list_run_summaries()
                .expect("active runs should list")
                .is_empty(),
            "{} active runs should be empty after archive",
            backend.name
        );
        assert!(
            store
                .list_recent_workflow_log_entries(5, None)
                .expect("active workflow logs should list")
                .is_empty(),
            "{} active workflow logs should be empty after archive",
            backend.name
        );
        assert!(
            store
                .list_recent_trigger_records(5, None)
                .expect("active trigger records should list")
                .is_empty(),
            "{} active trigger records should be empty after archive",
            backend.name
        );

        let archive_counts = store
            .archived_history_counts()
            .expect("archive counts should load");
        assert_eq!(
            archive_counts.run_summaries, 1,
            "{} archived run count",
            backend.name
        );
        assert_eq!(
            archive_counts.workflow_logs, 1,
            "{} archived log count",
            backend.name
        );
        assert_eq!(
            archive_counts.trigger_events, 1,
            "{} archived trigger count",
            backend.name
        );

        backend.reset();
    }
}

#[test]
fn runtime_state_backends_preserve_live_state_during_retention() {
    let _guard = backend_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());

    for backend in available_backends() {
        backend.ensure_initialized();
        backend.reset();
        let mut store =
            RuntimeStateStore::open(&backend.config, 1_710_920_000_000).unwrap_or_else(|error| {
                panic!("{} store should reopen after reset: {error}", backend.name)
            });

        store
            .write_run_summary(&RunRecordSummary {
                schema_version: "1.0.0".to_owned(),
                run_id: format!("{}-running-run", backend.slug),
                workflow_id: "wf-alpha".to_owned(),
                status: RunStatus::Running,
                started_at_ms: 1_710_000_000_000,
                finished_at_ms: None,
            })
            .expect("running run should persist");
        store
            .write_run_summary(&RunRecordSummary {
                schema_version: "1.0.0".to_owned(),
                run_id: format!("{}-finished-run", backend.slug),
                workflow_id: "wf-alpha".to_owned(),
                status: RunStatus::Succeeded,
                started_at_ms: 1_710_000_010_000,
                finished_at_ms: Some(1_710_000_020_000),
            })
            .expect("finished run should persist");

        let live_trigger = TriggerEventRecord {
            schema_version: "1.0.0".to_owned(),
            run_id: format!("{}-running-run", backend.slug),
            sequence: 1,
            trigger_id: "tr-alpha".to_owned(),
            workflow_id: "wf-alpha".to_owned(),
            event_id: format!("{}-live-event", backend.slug),
            checkpoint: None,
            source: "builtin.market".to_owned(),
            accepted_at_ms: 1_710_000_030_000,
            payload: serde_json::json!({"price": 99}),
            dedup_key: Some(format!("{}-live-dedup", backend.slug)),
            dedup_expires_at_ms: Some(1_710_920_060_000),
            cooldown_key: Some(format!("{}-live-cooldown", backend.slug)),
            cooldown_expires_at_ms: Some(1_710_920_070_000),
        };
        store
            .write_trigger_record(&live_trigger)
            .expect("live trigger event should persist");
        store
            .write_trigger_record(&TriggerEventRecord {
                schema_version: "1.0.0".to_owned(),
                run_id: format!("{}-finished-run", backend.slug),
                sequence: 2,
                trigger_id: "tr-alpha".to_owned(),
                workflow_id: "wf-alpha".to_owned(),
                event_id: format!("{}-expired-event", backend.slug),
                checkpoint: None,
                source: "builtin.market".to_owned(),
                accepted_at_ms: 1_710_000_040_000,
                payload: serde_json::json!({"price": 88}),
                dedup_key: None,
                dedup_expires_at_ms: None,
                cooldown_key: None,
                cooldown_expires_at_ms: None,
            })
            .expect("expired trigger event should persist");

        let archived = store
            .apply_history_retention(
                &RuntimeHistoryRetentionPolicy {
                    run_retention_ms: Some(60_000),
                    workflow_log_retention_ms: Some(60_000),
                    trigger_event_retention_ms: Some(60_000),
                },
                1_710_920_000_000,
            )
            .expect("retention should archive only fully expired history");
        assert_eq!(
            archived.archived_run_summaries, 1,
            "{} finished run should archive",
            backend.name
        );
        assert_eq!(
            archived.archived_trigger_events, 1,
            "{} only expired trigger event should archive",
            backend.name
        );

        let active_runs = store
            .list_run_summaries()
            .expect("active runs should stay queryable after retention");
        assert_eq!(
            active_runs.len(),
            1,
            "{} should keep only the running run active",
            backend.name
        );
        assert_eq!(active_runs[0].status, RunStatus::Running);

        let active_events = store
            .list_recent_trigger_records(10, Some("tr-alpha"))
            .expect("active trigger events should stay queryable after retention");
        assert_eq!(
            active_events.len(),
            1,
            "{} should keep only the live trigger event active",
            backend.name
        );
        assert_eq!(active_events[0].event_id, live_trigger.event_id);
        assert!(
            !store
                .dedup_is_ready(
                    live_trigger.dedup_key.as_deref().unwrap(),
                    1_710_920_000_001
                )
                .expect("live dedup token should remain active"),
            "{} dedup token should still block duplicate events",
            backend.name,
        );
        assert!(
            !store
                .cooldown_is_ready(
                    live_trigger.cooldown_key.as_deref().unwrap(),
                    1_710_920_000_001,
                )
                .expect("live cooldown token should remain active"),
            "{} cooldown token should still block duplicate events",
            backend.name,
        );

        let second_archived = store
            .apply_history_retention(
                &RuntimeHistoryRetentionPolicy {
                    run_retention_ms: Some(60_000),
                    workflow_log_retention_ms: Some(60_000),
                    trigger_event_retention_ms: Some(60_000),
                },
                1_710_920_000_100,
            )
            .expect("second retention pass should be idempotent");
        assert_eq!(
            second_archived.archived_run_summaries, 0,
            "{} second pass should not duplicate archived runs",
            backend.name
        );
        assert_eq!(
            second_archived.archived_trigger_events, 0,
            "{} second pass should not duplicate archived trigger events",
            backend.name
        );

        backend.reset();
    }
}

#[test]
fn runtime_state_backends_allow_only_one_serve_lease_winner_under_contention() {
    let _guard = backend_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());

    for backend in available_backends() {
        backend.ensure_initialized();
        backend.reset();

        let config = Arc::new(backend.config.clone());
        let barrier = Arc::new(std::sync::Barrier::new(3));

        let first_config = Arc::clone(&config);
        let first_barrier = Arc::clone(&barrier);
        let first = thread::spawn(move || {
            let mut store = RuntimeStateStore::open(&first_config, 1_710_930_000_000)
                .expect("first contender should open runtime state store");
            first_barrier.wait();
            store
                .try_acquire_serve_lease("owner-a", 1_710_930_000_000, 30_000)
                .expect("first contender lease acquisition should finish")
        });

        let second_config = Arc::clone(&config);
        let second_barrier = Arc::clone(&barrier);
        let second = thread::spawn(move || {
            let mut store = RuntimeStateStore::open(&second_config, 1_710_930_000_000)
                .expect("second contender should open runtime state store");
            second_barrier.wait();
            store
                .try_acquire_serve_lease("owner-b", 1_710_930_000_000, 30_000)
                .expect("second contender lease acquisition should finish")
        });

        barrier.wait();
        let first_result = first.join().expect("first contender should join cleanly");
        let second_result = second.join().expect("second contender should join cleanly");

        let winners = [first_result.clone(), second_result.clone()]
            .into_iter()
            .filter(|result| {
                matches!(
                    result,
                    LeaseAcquireResult::Acquired | LeaseAcquireResult::Renewed
                )
            })
            .count();
        assert_eq!(
            winners, 1,
            "{} should have exactly one lease winner",
            backend.name
        );
        assert!(
            matches!(first_result, LeaseAcquireResult::Rejected { .. })
                || matches!(second_result, LeaseAcquireResult::Rejected { .. }),
            "{} should reject one contender under contention",
            backend.name,
        );

        backend.reset();
    }
}

struct BackendFixture {
    name: &'static str,
    slug: &'static str,
    config: RuntimeStorageConfig,
    ensure_initialized: Box<dyn Fn() + Send + Sync>,
    reset: Box<dyn Fn() + Send + Sync>,
}

impl BackendFixture {
    fn ensure_initialized(&self) {
        (self.ensure_initialized)();
    }

    fn reset(&self) {
        (self.reset)();
    }
}

fn available_backends() -> Vec<BackendFixture> {
    let mut backends = vec![sqlite_backend_fixture()];
    if let Some(postgres_backend) = postgres_backend_fixture() {
        backends.push(postgres_backend);
    }
    backends
}

fn sqlite_backend_fixture() -> BackendFixture {
    let database_path = unique_sqlite_path("runtime-state-parity");
    let reset_path = database_path.clone();
    BackendFixture {
        name: "sqlite-local",
        slug: "sqlite",
        config: RuntimeStorageConfig {
            backend: RuntimeStorageBackend::Local {
                database_path: database_path.clone(),
            },
            history_retention: None,
            raw_debug_enabled: false,
            raw_debug_artifacts_dir: None,
        },
        ensure_initialized: Box::new(|| {}),
        reset: Box::new(move || {
            let _ = fs::remove_file(&reset_path);
        }),
    }
}

fn postgres_backend_fixture() -> Option<BackendFixture> {
    let database_url = std::env::var(TEST_POSTGRES_URL_ENV).ok()?;
    let init_url = database_url.clone();
    let reset_url = database_url.clone();
    Some(BackendFixture {
        name: "postgres",
        slug: "postgres",
        config: RuntimeStorageConfig {
            backend: RuntimeStorageBackend::Postgres { database_url },
            history_retention: None,
            raw_debug_enabled: false,
            raw_debug_artifacts_dir: None,
        },
        ensure_initialized: Box::new(move || {
            RuntimeStateStore::open(
                &RuntimeStorageConfig {
                    backend: RuntimeStorageBackend::Postgres {
                        database_url: init_url.clone(),
                    },
                    history_retention: None,
                    raw_debug_enabled: false,
                    raw_debug_artifacts_dir: None,
                },
                1_710_900_000_000,
            )
            .expect("postgres runtime store should initialize before cleanup");
        }),
        reset: Box::new(move || reset_postgres_tables(&reset_url)),
    })
}

fn reset_postgres_tables(database_url: &str) {
    let mut client = Client::connect(database_url, NoTls)
        .expect("postgres test database should be connectable for cleanup");
    client
        .batch_execute(
            "TRUNCATE TABLE \
                daemon_sessions, \
                workflow_runtime_logs, \
                trigger_event_records, \
                trigger_checkpoints, \
                trigger_snapshots, \
                archived_workflow_runtime_logs, \
                archived_trigger_event_records, \
                archived_run_summaries, \
                run_summaries, \
                serve_leases \
             RESTART IDENTITY",
        )
        .expect("postgres test tables should truncate cleanly");
}

fn unique_sqlite_path(prefix: &str) -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after UNIX_EPOCH")
        .as_nanos();
    std::env::temp_dir().join(format!("chainbot-{prefix}-{timestamp}.sqlite3"))
}

fn backend_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}
