//! [INPUT]
//! Temporary root layouts, SQLite coordination state, and file-backed runtime records.
//!
//! [OUTPUT]
//! Verifies leases, append-only runtime artifacts, deterministic listing, and crash recovery behavior.
//!
//! [ROLE]
//! Covers the durable runtime-state boundary as an integration test.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use chainbot::config::RootLayout;
use chainbot::state::{
    CoordinationStore, FileBackedStateStore, FileStateError, LeaseAcquireResult, RunRecordSummary,
    RunStatus, ServeLeaseState, StateLayout, TriggerEventRecord, TriggerSnapshotRecord,
    WorkflowRuntimeLogEntry, SERVE_OWNER_ID_PREFIX,
};
use rusqlite::Connection;

#[test]
fn sqlite_coordination_migrations() {
    let layout = unique_state_layout("sqlite-coordination-migrations");
    let store =
        CoordinationStore::open(&layout, 1_710_000_001_000).expect("sqlite store should open");

    assert!(layout.coordination_db_path.exists());
    assert_eq!(
        store
            .applied_migration_versions()
            .expect("migration versions should load"),
        vec![1]
    );

    let sqlite = Connection::open(store.database_path()).expect("sqlite db should be readable");
    let mut statement = sqlite
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
        .expect("table listing query should prepare");
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .expect("table listing query should execute");

    let mut table_names = BTreeSet::new();
    for row in rows {
        table_names.insert(row.expect("table name row should decode"));
    }

    assert!(table_names.contains("schema_migrations"));
    assert!(table_names.contains("serve_leases"));
    assert!(table_names.contains("coordination_tokens"));
    assert!(!table_names.contains("run_summaries"));
    assert!(!table_names.contains("workflow_runtime_logs"));
    assert!(!table_names.contains("trigger_event_records"));

    drop(store);

    let reopened = CoordinationStore::open(&layout, 1_710_000_002_000)
        .expect("reopening sqlite store should keep migration idempotent");
    assert_eq!(
        reopened
            .applied_migration_versions()
            .expect("migration versions should still load"),
        vec![1]
    );
}

#[test]
fn serve_single_owner_lease() {
    let layout = unique_state_layout("serve-single-owner-lease");
    let mut store =
        CoordinationStore::open(&layout, 1_710_000_010_000).expect("sqlite store should open");

    let first = store
        .try_acquire_serve_lease("owner-alpha", 1_710_000_010_000, 5_000)
        .expect("first owner should acquire lease");
    assert!(matches!(first, LeaseAcquireResult::Acquired));

    let second = store
        .try_acquire_serve_lease("owner-beta", 1_710_000_012_000, 5_000)
        .expect("second owner attempt should be evaluated");
    assert!(matches!(
        second,
        LeaseAcquireResult::Rejected {
            current_owner,
            expires_at_ms
        } if current_owner == "owner-alpha" && expires_at_ms == 1_710_000_015_000
    ));

    let renewed = store
        .try_acquire_serve_lease("owner-alpha", 1_710_000_013_000, 5_000)
        .expect("same owner should be able to renew lease");
    assert!(matches!(renewed, LeaseAcquireResult::Renewed));

    assert!(!store
        .release_serve_lease("owner-beta")
        .expect("wrong owner release should be non-fatal"));
    assert!(store
        .release_serve_lease("owner-alpha")
        .expect("current owner should release lease"));

    let third = store
        .try_acquire_serve_lease("owner-beta", 1_710_000_014_000, 5_000)
        .expect("lease should be free after release");
    assert!(matches!(third, LeaseAcquireResult::Acquired));
}

#[test]
fn inspect_serve_lease_snapshot_tracks_idle_active_and_stale() {
    let layout = unique_state_layout("inspect-serve-lease-snapshot-tracks-states");
    let mut store =
        CoordinationStore::open(&layout, 1_710_000_020_000).expect("sqlite store should open");

    let idle = store
        .inspect_serve_lease(1_710_000_020_000)
        .expect("idle lease snapshot should load");
    assert_eq!(idle.state, ServeLeaseState::Idle);
    assert_eq!(idle.owner_id, None);
    assert_eq!(idle.expires_at_ms, None);

    let owner_id = format!("{SERVE_OWNER_ID_PREFIX}{}", std::process::id());
    store
        .try_acquire_serve_lease(&owner_id, 1_710_000_020_000, 5_000)
        .expect("serve lease should be acquired");

    let active = store
        .inspect_serve_lease(1_710_000_021_000)
        .expect("active lease snapshot should load");
    assert_eq!(active.state, ServeLeaseState::Active);
    assert_eq!(active.owner_id.as_deref(), Some(owner_id.as_str()));
    assert_eq!(active.expires_at_ms, Some(1_710_000_025_000));

    let stale = store
        .inspect_serve_lease(1_710_000_026_000)
        .expect("stale lease snapshot should load");
    assert_eq!(stale.state, ServeLeaseState::Stale);
    assert_eq!(stale.owner_id.as_deref(), Some(owner_id.as_str()));
    assert_eq!(stale.expires_at_ms, Some(1_710_000_025_000));
}

#[test]
fn serve_lease_can_transfer_after_expiry_across_restart_without_manual_release() {
    let layout = unique_state_layout("serve-lease-transfer-after-expiry-restart");
    let mut first_store =
        CoordinationStore::open(&layout, 1_710_000_030_000).expect("sqlite store should open");

    let first = first_store
        .try_acquire_serve_lease("owner-alpha", 1_710_000_030_000, 5_000)
        .expect("first owner should acquire lease");
    assert!(matches!(first, LeaseAcquireResult::Acquired));

    let rejected = first_store
        .try_acquire_serve_lease("owner-beta", 1_710_000_030_100, 5_000)
        .expect("second owner should be rejected before lease expiry");
    assert!(matches!(rejected, LeaseAcquireResult::Rejected { .. }));
    drop(first_store);

    let mut reopened_store = CoordinationStore::open(&layout, 1_710_000_036_001)
        .expect("sqlite store should reopen for post-expiry transfer");
    let transferred = reopened_store
        .try_acquire_serve_lease("owner-beta", 1_710_000_036_001, 5_000)
        .expect("new owner should acquire after previous lease expiry");
    assert!(matches!(transferred, LeaseAcquireResult::Acquired));

    let snapshot = reopened_store
        .inspect_serve_lease(1_710_000_036_002)
        .expect("lease snapshot should load after transfer");
    assert_eq!(snapshot.state, ServeLeaseState::Active);
    assert_eq!(snapshot.owner_id.as_deref(), Some("owner-beta"));
    assert_eq!(snapshot.expires_at_ms, Some(1_710_000_041_001));
}

#[test]
fn file_backed_runtime_logs() {
    let layout = unique_state_layout("file-backed-runtime-logs");
    let store = FileBackedStateStore::new(layout.clone());
    store
        .initialize()
        .expect("file-backed state tree should initialize");

    let summary = RunRecordSummary {
        schema_version: "1.0.0".to_string(),
        run_id: "run-alpha".to_string(),
        workflow_id: "wf-alpha".to_string(),
        status: RunStatus::Running,
        started_at_ms: 1_710_000_100_000,
        finished_at_ms: None,
    };
    let summary_path = store
        .write_run_summary(&summary)
        .expect("run summary should persist to files");
    assert_eq!(summary_path, layout.run_summary_path("run-alpha"));
    assert!(summary_path.exists());

    let first_log = WorkflowRuntimeLogEntry {
        run_id: "run-alpha".to_string(),
        sequence: 1,
        event: "run_started".to_string(),
        message: "execution accepted".to_string(),
        occurred_at_ms: 1_710_000_100_010,
    };
    let second_log = WorkflowRuntimeLogEntry {
        run_id: "run-alpha".to_string(),
        sequence: 2,
        event: "node_completed".to_string(),
        message: "node-a succeeded".to_string(),
        occurred_at_ms: 1_710_000_100_030,
    };

    let first_log_path = store
        .write_workflow_log_entry(&first_log)
        .expect("first workflow log should persist");
    let second_log_path = store
        .write_workflow_log_entry(&second_log)
        .expect("second workflow log should persist");

    assert_eq!(
        first_log_path,
        layout.workflow_log_entry_path("run-alpha", 1)
    );
    assert_eq!(
        second_log_path,
        layout.workflow_log_entry_path("run-alpha", 2)
    );
    assert!(first_log_path.exists());
    assert!(second_log_path.exists());

    let trigger_record = TriggerEventRecord {
        schema_version: "1.0.0".to_string(),
        run_id: "run-alpha".to_string(),
        sequence: 7,
        trigger_id: "market.tick".to_string(),
        workflow_id: "wf-alpha".to_string(),
        event_id: "btc/usdt@1m".to_string(),
        checkpoint: None,
        source: "feed-A".to_string(),
        accepted_at_ms: 1_710_000_100_020,
        payload: serde_json::json!({"symbol": "BTCUSDT"}),
        dedup_key: None,
        dedup_expires_at_ms: None,
        cooldown_key: None,
        cooldown_expires_at_ms: None,
    };

    let trigger_path = store
        .write_trigger_record(&trigger_record)
        .expect("trigger record should persist");
    assert_eq!(
        trigger_path,
        layout.trigger_record_path("run-alpha", 7, "market.tick", "btc/usdt@1m")
    );
    assert!(trigger_path.exists());

    let loaded_summary = store
        .read_run_summary("run-alpha")
        .expect("run summary should load");
    assert_eq!(loaded_summary, summary);

    let loaded_first_log: WorkflowRuntimeLogEntry = serde_json::from_str(
        &fs::read_to_string(&first_log_path).expect("first log file should be readable"),
    )
    .expect("first log json should decode");
    let loaded_second_log: WorkflowRuntimeLogEntry = serde_json::from_str(
        &fs::read_to_string(&second_log_path).expect("second log file should be readable"),
    )
    .expect("second log json should decode");
    let loaded_trigger: TriggerEventRecord = serde_json::from_str(
        &fs::read_to_string(&trigger_path).expect("trigger file should be readable"),
    )
    .expect("trigger json should decode");

    assert_eq!(loaded_first_log, first_log);
    assert_eq!(loaded_second_log, second_log);
    assert_eq!(loaded_trigger, trigger_record);
}

#[test]
fn file_backed_run_summary_recovery() {
    let layout = unique_state_layout("file-backed-run-summary-recovery");
    let store = FileBackedStateStore::new(layout.clone());
    store
        .initialize()
        .expect("file-backed state tree should initialize");

    let staged_only_summary = RunRecordSummary {
        schema_version: "1.0.0".to_string(),
        run_id: "run-recover".to_string(),
        workflow_id: "wf-alpha".to_string(),
        status: RunStatus::Running,
        started_at_ms: 1_710_000_200_000,
        finished_at_ms: None,
    };

    let staged_only_path = layout.staged_run_summary_path("run-recover");
    let staged_only_parent = staged_only_path
        .parent()
        .expect("staged summary should have parent directory");
    fs::create_dir_all(staged_only_parent).expect("run directory should be creatable");
    fs::write(
        &staged_only_path,
        serde_json::to_vec_pretty(&staged_only_summary)
            .expect("staged-only summary should serialize"),
    )
    .expect("staged-only summary should be writable");

    let first_report = store
        .recover_run_summaries()
        .expect("staged-only summary should recover");
    assert_eq!(first_report.promoted_staged_files, 1);
    assert_eq!(first_report.removed_staged_files, 0);

    let recovered = store
        .read_run_summary("run-recover")
        .expect("recovered summary should be readable");
    assert_eq!(recovered.status, RunStatus::Running);

    let committed_summary = RunRecordSummary {
        schema_version: "1.0.0".to_string(),
        run_id: "run-recover".to_string(),
        workflow_id: "wf-alpha".to_string(),
        status: RunStatus::Succeeded,
        started_at_ms: 1_710_000_200_000,
        finished_at_ms: Some(1_710_000_205_000),
    };
    store
        .write_run_summary(&committed_summary)
        .expect("committed summary should persist");

    let stale_staged_summary = RunRecordSummary {
        schema_version: "1.0.0".to_string(),
        run_id: "run-recover".to_string(),
        workflow_id: "wf-alpha".to_string(),
        status: RunStatus::Failed,
        started_at_ms: 1_710_000_200_000,
        finished_at_ms: Some(1_710_000_204_000),
    };
    let stale_staged_path = layout.staged_run_summary_path("run-recover");
    fs::write(
        &stale_staged_path,
        serde_json::to_vec_pretty(&stale_staged_summary)
            .expect("stale staged summary should serialize"),
    )
    .expect("stale staged summary should be writable");

    let second_report = store
        .recover_run_summaries()
        .expect("stale staged summary should be removed");
    assert_eq!(second_report.promoted_staged_files, 0);
    assert_eq!(second_report.removed_staged_files, 1);
    assert!(!stale_staged_path.exists());

    let final_summary = store
        .read_run_summary("run-recover")
        .expect("committed summary should remain authoritative");
    assert_eq!(final_summary.status, RunStatus::Succeeded);
}

#[test]
fn file_log_recovery_and_rotation_policy() {
    let layout = unique_state_layout("file-log-recovery-and-rotation-policy");
    let store = FileBackedStateStore::new(layout.clone());
    store
        .initialize()
        .expect("file-backed state tree should initialize");

    let log_entry = WorkflowRuntimeLogEntry {
        run_id: "run-log".to_string(),
        sequence: 1,
        event: "run_started".to_string(),
        message: "first entry".to_string(),
        occurred_at_ms: 1_710_500_000_000,
    };
    let trigger_record = TriggerEventRecord {
        schema_version: "1.0.0".to_string(),
        run_id: "run-log".to_string(),
        sequence: 1,
        trigger_id: "trigger-log".to_string(),
        workflow_id: "wf-log".to_string(),
        event_id: "event-log".to_string(),
        checkpoint: None,
        source: "fixture".to_string(),
        accepted_at_ms: 1_710_500_000_010,
        payload: serde_json::json!({"event": 1}),
        dedup_key: None,
        dedup_expires_at_ms: None,
        cooldown_key: None,
        cooldown_expires_at_ms: None,
    };

    store
        .write_workflow_log_entry(&log_entry)
        .expect("first log entry should persist");
    store
        .write_trigger_record(&trigger_record)
        .expect("first trigger record should persist");

    let duplicate_log_error = store
        .write_workflow_log_entry(&log_entry)
        .expect_err("append-only workflow log path should reject duplicate sequence writes");
    assert!(matches!(
        duplicate_log_error,
        FileStateError::ImmutableFileExists { .. }
    ));
    let duplicate_trigger_error = store
        .write_trigger_record(&trigger_record)
        .expect_err("append-only trigger record path should reject duplicate sequence writes");
    assert!(matches!(
        duplicate_trigger_error,
        FileStateError::ImmutableFileExists { .. }
    ));

    write_json_file(
        &layout.staged_workflow_log_entry_path("run-log", 2),
        &WorkflowRuntimeLogEntry {
            run_id: "run-log".to_string(),
            sequence: 2,
            event: "run_progress".to_string(),
            message: "staged log entry".to_string(),
            occurred_at_ms: 1_710_500_000_020,
        },
    );
    write_json_file(
        &layout.staged_trigger_record_path("run-log", 2, "trigger-log", "event-log-2"),
        &TriggerEventRecord {
            schema_version: "1.0.0".to_string(),
            run_id: "run-log".to_string(),
            sequence: 2,
            trigger_id: "trigger-log".to_string(),
            workflow_id: "wf-log".to_string(),
            event_id: "event-log-2".to_string(),
            checkpoint: None,
            source: "fixture".to_string(),
            accepted_at_ms: 1_710_500_000_021,
            payload: serde_json::json!({"event": 2}),
            dedup_key: None,
            dedup_expires_at_ms: None,
            cooldown_key: None,
            cooldown_expires_at_ms: None,
        },
    );

    let workflow_recovery = store
        .recover_workflow_logs()
        .expect("staged workflow log should recover");
    let trigger_recovery = store
        .recover_trigger_records()
        .expect("staged trigger record should recover");
    assert_eq!(workflow_recovery.promoted_staged_files, 1);
    assert_eq!(workflow_recovery.removed_staged_files, 0);
    assert_eq!(trigger_recovery.promoted_staged_files, 1);
    assert_eq!(trigger_recovery.removed_staged_files, 0);
    assert!(layout.workflow_log_entry_path("run-log", 2).exists());
    assert!(layout
        .trigger_record_path("run-log", 2, "trigger-log", "event-log-2")
        .exists());

    write_json_file(
        &layout.staged_workflow_log_entry_path("run-log", 1),
        &WorkflowRuntimeLogEntry {
            run_id: "run-log".to_string(),
            sequence: 1,
            event: "run_started".to_string(),
            message: "stale duplicate".to_string(),
            occurred_at_ms: 1_710_500_000_030,
        },
    );
    write_json_file(
        &layout.staged_trigger_record_path("run-log", 1, "trigger-log", "event-log"),
        &TriggerEventRecord {
            schema_version: "1.0.0".to_string(),
            run_id: "run-log".to_string(),
            sequence: 1,
            trigger_id: "trigger-log".to_string(),
            workflow_id: "wf-log".to_string(),
            event_id: "event-log".to_string(),
            checkpoint: None,
            source: "fixture".to_string(),
            accepted_at_ms: 1_710_500_000_031,
            payload: serde_json::json!({"event": 1}),
            dedup_key: None,
            dedup_expires_at_ms: None,
            cooldown_key: None,
            cooldown_expires_at_ms: None,
        },
    );

    let stale_workflow_recovery = store
        .recover_workflow_logs()
        .expect("stale staged workflow log should be removed");
    let stale_trigger_recovery = store
        .recover_trigger_records()
        .expect("stale staged trigger record should be removed");
    assert_eq!(stale_workflow_recovery.promoted_staged_files, 0);
    assert_eq!(stale_workflow_recovery.removed_staged_files, 1);
    assert_eq!(stale_trigger_recovery.promoted_staged_files, 0);
    assert_eq!(stale_trigger_recovery.removed_staged_files, 1);
    assert!(!layout.staged_workflow_log_entry_path("run-log", 1).exists());
    assert!(!layout
        .staged_trigger_record_path("run-log", 1, "trigger-log", "event-log")
        .exists());

    fs::write(layout.workflow_log_sequence_cursor_path("run-log"), b"2")
        .expect("cursor file should be writable");
    let appended_path = store
        .append_workflow_log_entry(
            "run-log",
            "run_finished",
            "cursor-cached append",
            1_710_500_000_040,
        )
        .expect("append should advance beyond stale cursor and existing files");
    assert_eq!(appended_path, layout.workflow_log_entry_path("run-log", 3));
}

#[test]
fn trigger_snapshot_roundtrip_and_recovery() {
    let layout = unique_state_layout("trigger-snapshot-roundtrip-and-recovery");
    let store = FileBackedStateStore::new(layout.clone());
    store
        .initialize()
        .expect("file-backed state tree should initialize");

    let trigger_record = TriggerEventRecord {
        schema_version: "1.0.0".to_string(),
        run_id: "run-snapshot".to_string(),
        sequence: 7,
        trigger_id: "trigger-snapshot".to_string(),
        workflow_id: "wf-snapshot".to_string(),
        event_id: "event-snapshot".to_string(),
        checkpoint: Some("cp-7".to_string()),
        source: "fixture".to_string(),
        accepted_at_ms: 1_710_500_100_000,
        payload: serde_json::json!({"event": 7}),
        dedup_key: Some("dedup-snapshot".to_string()),
        dedup_expires_at_ms: Some(1_710_500_110_000),
        cooldown_key: Some("cooldown-snapshot".to_string()),
        cooldown_expires_at_ms: Some(1_710_500_120_000),
    };
    let mut snapshot = TriggerSnapshotRecord::new("trigger-snapshot");
    snapshot.apply_record(&trigger_record);

    let snapshot_path = store
        .write_trigger_snapshot(&snapshot)
        .expect("trigger snapshot should persist");
    assert_eq!(
        snapshot_path,
        layout.trigger_snapshot_path("trigger-snapshot")
    );

    let loaded = store
        .read_trigger_snapshot("trigger-snapshot")
        .expect("trigger snapshot should load")
        .expect("trigger snapshot should exist");
    assert_eq!(loaded, snapshot);

    let staged_snapshot_path = layout.staged_trigger_snapshot_path("trigger-snapshot");
    fs::remove_file(&snapshot_path)
        .expect("committed snapshot should be removable for staged recovery");
    write_json_file(&staged_snapshot_path, &snapshot);

    let recovery = store
        .recover_trigger_snapshots()
        .expect("staged trigger snapshot should recover");
    assert_eq!(recovery.promoted_staged_files, 1);
    assert_eq!(recovery.removed_staged_files, 0);
    assert!(snapshot_path.exists());
}

fn write_json_file<T>(path: &Path, value: &T)
where
    T: serde::Serialize,
{
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("staged file parent should be creatable");
    }
    fs::write(
        path,
        serde_json::to_vec_pretty(value).expect("JSON payload should serialize"),
    )
    .expect("JSON payload should be writable");
}

fn unique_state_layout(prefix: &str) -> StateLayout {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after UNIX_EPOCH")
        .as_nanos();
    let root = workspace_root()
        .join("target")
        .join("test-roots")
        .join(format!("{prefix}-{now}"));
    let root_layout = RootLayout::from_root(root);
    StateLayout::from_root_layout(&root_layout)
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
