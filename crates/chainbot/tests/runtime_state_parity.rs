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
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use chainbot::config::{RuntimeStorageBackend, RuntimeStorageConfig};
use chainbot::state::{
    LeaseAcquireResult, RunRecordSummary, RunStatus, TriggerCheckpointRecord, TriggerEventRecord,
    TriggerSnapshotRecord,
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
        let rejected = store
            .try_acquire_serve_lease("owner-b", 1_710_900_001_000, 30_000)
            .expect("conflicting lease acquisition should succeed with rejection result");
        assert!(matches!(rejected, LeaseAcquireResult::Rejected { .. }));
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
            run_id: format!("{}-run-1", backend.slug),
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
            raw_debug_enabled: false,
            raw_debug_artifacts_dir: None,
        },
        ensure_initialized: Box::new(move || {
            RuntimeStateStore::open(
                &RuntimeStorageConfig {
                    backend: RuntimeStorageBackend::Postgres {
                        database_url: init_url.clone(),
                    },
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
                workflow_runtime_logs, \
                trigger_event_records, \
                trigger_checkpoints, \
                trigger_snapshots, \
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
