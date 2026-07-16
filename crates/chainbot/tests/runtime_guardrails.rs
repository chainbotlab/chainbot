//! [INPUT]
//! SQLite runtime store fixtures, seeded run/log/trigger history rows, and read-only query plans.
//!
//! [OUTPUT]
//! Guards hot-path query plans and repeated read-only observation behavior for runtime history.
//!
//! [ROLE]
//! Provides a small, stable performance and soak harness for DB-primary runtime reads.

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use chainbot::domain::state::{RunRecordSummary, RunStatus, TriggerEventRecord};
use chainbot::infrastructure::config::{RuntimeStorageBackend, RuntimeStorageConfig};
use chainbot::infrastructure::state::RuntimeStateStore;
use rusqlite::Connection;

#[test]
fn sqlite_runtime_read_queries_keep_indexed_plans() {
    let database_path = unique_sqlite_path("runtime-guardrails-plans");
    let config = sqlite_config(database_path.clone());
    let mut store = RuntimeStateStore::open(&config, 1_711_000_000_000)
        .expect("runtime store should initialize for query-plan guardrails");

    for index in 0..32_i64 {
        let run_id = format!("guardrail-run-{index}");
        store
            .write_run_summary(&RunRecordSummary {
                schema_version: "1.0.0".to_owned(),
                run_id: run_id.clone(),
                workflow_id: "wf-alpha".to_owned(),
                status: RunStatus::Succeeded,
                started_at_ms: 1_711_000_000_000 + index,
                finished_at_ms: Some(1_711_000_000_100 + index),
                owner_id: None,
                lease_generation: None,
            })
            .expect("seeded run summary should persist");
        store
            .append_workflow_log_entry(
                &run_id,
                "run_finished",
                "guardrail log",
                1_711_000_000_100 + index,
            )
            .expect("seeded workflow log should persist");
        store
            .write_trigger_record(&TriggerEventRecord {
                schema_version: "1.0.0".to_owned(),
                run_id,
                sequence: u64::try_from(index + 1).unwrap_or(1),
                trigger_id: "tr-alpha".to_owned(),
                workflow_id: "wf-alpha".to_owned(),
                event_id: format!("guardrail-event-{index}"),
                checkpoint: None,
                source: "builtin.market".to_owned(),
                accepted_at_ms: 1_711_000_000_050 + index,
                payload: serde_json::json!({"price": 100 + index}),
                dedup_key: None,
                dedup_expires_at_ms: None,
                cooldown_key: None,
                cooldown_expires_at_ms: None,
            })
            .expect("seeded trigger event should persist");
    }

    let connection = Connection::open(&database_path).expect("sqlite connection should open");

    let run_plan = explain_plan(
        &connection,
        "SELECT schema_version, run_id, workflow_id, status, started_at_ms, finished_at_ms
         FROM run_summaries
         ORDER BY started_at_ms DESC, run_id DESC
         LIMIT 5",
    );
    assert!(
        run_plan
            .iter()
            .any(|detail| detail.contains("idx_run_summaries_started_at")),
        "recent run summaries should use started_at index: {run_plan:?}"
    );

    let log_plan = explain_plan(
        &connection,
        "SELECT run_id, sequence, event, message, occurred_at_ms
         FROM workflow_runtime_logs
         ORDER BY occurred_at_ms DESC, run_id DESC, sequence DESC
         LIMIT 5",
    );
    assert!(
        log_plan
            .iter()
            .any(|detail| detail.contains("idx_workflow_runtime_logs_occurred_at")),
        "recent workflow logs should use occurred_at index: {log_plan:?}"
    );

    let trigger_plan = explain_plan(
        &connection,
        "SELECT schema_version, run_id, sequence, trigger_id, workflow_id, event_id,
                checkpoint, source, accepted_at_ms, payload_json,
                dedup_key, dedup_expires_at_ms, cooldown_key, cooldown_expires_at_ms
         FROM trigger_event_records
         ORDER BY accepted_at_ms DESC, trigger_id DESC, sequence DESC
         LIMIT 5",
    );
    assert!(
        trigger_plan
            .iter()
            .any(|detail| detail.contains("idx_trigger_event_records_accepted_at")),
        "recent trigger events should use accepted_at index: {trigger_plan:?}"
    );
}

#[test]
fn repeated_runtime_observation_remains_read_only() {
    let database_path = unique_sqlite_path("runtime-guardrails-soak");
    let config = sqlite_config(database_path);
    let mut store = RuntimeStateStore::open(&config, 1_711_100_000_000)
        .expect("runtime store should initialize for read-only soak");

    store
        .write_run_summary(&RunRecordSummary {
            schema_version: "1.0.0".to_owned(),
            run_id: "soak-run-1".to_owned(),
            workflow_id: "wf-alpha".to_owned(),
            status: RunStatus::Succeeded,
            started_at_ms: 1_711_100_000_000,
            finished_at_ms: Some(1_711_100_000_100),
            owner_id: None,
            lease_generation: None,
        })
        .expect("run summary should persist");
    store
        .append_workflow_log_entry(
            "soak-run-1",
            "run_finished",
            "soak completed",
            1_711_100_000_100,
        )
        .expect("workflow log should persist");
    store
        .write_trigger_record(&TriggerEventRecord {
            schema_version: "1.0.0".to_owned(),
            run_id: "soak-run-1".to_owned(),
            sequence: 1,
            trigger_id: "tr-alpha".to_owned(),
            workflow_id: "wf-alpha".to_owned(),
            event_id: "soak-event-1".to_owned(),
            checkpoint: None,
            source: "builtin.market".to_owned(),
            accepted_at_ms: 1_711_100_000_050,
            payload: serde_json::json!({"price": 111}),
            dedup_key: None,
            dedup_expires_at_ms: None,
            cooldown_key: None,
            cooldown_expires_at_ms: None,
        })
        .expect("trigger event should persist");

    for _ in 0..128 {
        assert_eq!(
            store
                .list_recent_run_summaries(5)
                .expect("recent runs should stay readable")
                .len(),
            1
        );
        assert_eq!(
            store
                .list_recent_workflow_log_entries(5, None)
                .expect("recent workflow logs should stay readable")
                .len(),
            1
        );
        assert_eq!(
            store
                .list_recent_trigger_records(5, None)
                .expect("recent trigger events should stay readable")
                .len(),
            1
        );
        assert_eq!(
            store
                .archived_history_counts()
                .expect("archive counts should stay readable"),
            chainbot::infrastructure::state::RuntimeHistoryArchiveCounts::default()
        );
    }

    assert_eq!(
        store
            .list_run_summaries()
            .expect("active runs should remain intact")
            .len(),
        1
    );
    assert_eq!(
        store
            .list_recent_workflow_log_entries(5, None)
            .expect("active logs should remain intact")
            .len(),
        1
    );
    assert_eq!(
        store
            .list_recent_trigger_records(5, None)
            .expect("active trigger events should remain intact")
            .len(),
        1
    );
}

fn explain_plan(connection: &Connection, sql: &str) -> Vec<String> {
    let query = format!("EXPLAIN QUERY PLAN {sql}");
    let mut statement = connection
        .prepare(&query)
        .expect("query plan statement should prepare");
    let rows = statement
        .query_map([], |row| row.get::<_, String>(3))
        .expect("query plan should execute");

    rows.map(|row| row.expect("query plan row should decode"))
        .collect()
}

fn sqlite_config(database_path: PathBuf) -> RuntimeStorageConfig {
    RuntimeStorageConfig {
        backend: RuntimeStorageBackend::Local { database_path },
        history_retention: None,
        raw_debug_enabled: false,
        raw_debug_artifacts_dir: None,
    }
}

fn unique_sqlite_path(prefix: &str) -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after UNIX_EPOCH")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("chainbot-{prefix}-{timestamp}.sqlite3"));
    let _ = fs::remove_file(&path);
    path
}
