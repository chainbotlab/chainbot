//! [INPUT]
//! State-root filesystem paths, SQLite lease coordination, and runtime run, log, and trigger payloads.
//!
//! [OUTPUT]
//! Persists deterministic run summaries, append-only runtime artifacts, and serve-coordination records with restart recovery support.
//!
//! [ROLE]
//! Owns durable runtime state and coordination for the ChainBot execution lifecycle.
//!
//! [INVARIANTS]
//! Persisted run and trigger artifacts remain append-only, and state paths stay deterministic across restarts.

use std::error::Error;
use std::fmt::{Display, Formatter};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::config::RootLayout;
use crate::errors::{assert_supported_major, ContractError};

pub const CURRENT_SCHEMA_MAJOR: u64 = 1;
pub const COORDINATION_DB_FILE_NAME: &str = "coordination.sqlite3";
pub const RUNS_DIR_NAME: &str = "runs";
pub const WORKFLOW_LOGS_DIR_NAME: &str = "workflow-logs";
pub const TRIGGER_RECORDS_DIR_NAME: &str = "trigger-records";
pub const TRIGGER_CHECKPOINTS_DIR_NAME: &str = "trigger-checkpoints";
pub const SERVE_OWNER_ID_PREFIX: &str = "chainbot-serve-pid-";

const RUN_SUMMARY_FILE_NAME: &str = "summary.json";
const STAGED_FILE_SUFFIX: &str = ".next";
const SERVE_LEASE_KEY: &str = "serve";
const SQLITE_COORDINATION_SCHEMA_VERSION: i64 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunRecordSummary {
    pub schema_version: String,
    pub run_id: String,
    pub workflow_id: String,
    pub status: RunStatus,
    pub started_at_ms: i64,
    pub finished_at_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowRuntimeLogEntry {
    pub run_id: String,
    pub sequence: u64,
    pub event: String,
    pub message: String,
    pub occurred_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerEventRecord {
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    pub run_id: String,
    pub sequence: u64,
    pub trigger_id: String,
    #[serde(default)]
    pub workflow_id: String,
    pub event_id: String,
    #[serde(default)]
    pub checkpoint: Option<String>,
    pub source: String,
    pub accepted_at_ms: i64,
    #[serde(default)]
    pub payload: serde_json::Value,
    #[serde(default)]
    pub dedup_key: Option<String>,
    #[serde(default)]
    pub dedup_expires_at_ms: Option<i64>,
    #[serde(default)]
    pub cooldown_key: Option<String>,
    #[serde(default)]
    pub cooldown_expires_at_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerCheckpointRecord {
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    pub trigger_id: String,
    pub checkpoint: String,
    pub acked_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateLayout {
    pub state_root: PathBuf,
    pub coordination_db_path: PathBuf,
    pub runs_dir: PathBuf,
    pub workflow_logs_dir: PathBuf,
    pub trigger_records_dir: PathBuf,
    pub trigger_checkpoints_dir: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoordinationTokenKind {
    Dedup,
    Cooldown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeaseAcquireResult {
    Acquired,
    Renewed,
    Rejected {
        current_owner: String,
        expires_at_ms: i64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServeLeaseState {
    Idle,
    Active,
    Stale,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServeLeaseSnapshot {
    pub state: ServeLeaseState,
    pub owner_id: Option<String>,
    pub expires_at_ms: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RunSummaryRecoveryReport {
    pub promoted_staged_files: usize,
    pub removed_staged_files: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FileArtifactRecoveryReport {
    pub promoted_staged_files: usize,
    pub removed_staged_files: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RuntimeStateRecoveryReport {
    pub run_summaries: RunSummaryRecoveryReport,
    pub workflow_logs: FileArtifactRecoveryReport,
    pub trigger_records: FileArtifactRecoveryReport,
    pub failed_incomplete_runs: usize,
}

#[derive(Debug)]
pub struct FileBackedStateStore {
    layout: StateLayout,
}

#[derive(Debug)]
pub struct CoordinationStore {
    db_path: PathBuf,
    connection: Connection,
}

#[derive(Debug)]
pub enum FileStateError {
    Io {
        path: PathBuf,
        operation: &'static str,
        source: std::io::Error,
    },
    JsonEncode {
        path: PathBuf,
        source: serde_json::Error,
    },
    JsonDecode {
        path: PathBuf,
        source: serde_json::Error,
    },
    MissingRunSummary {
        path: PathBuf,
    },
    InvalidRunSummary {
        path: PathBuf,
        source: ContractError,
    },
    ImmutableFileExists {
        path: PathBuf,
        operation: &'static str,
    },
}

#[derive(Debug)]
pub enum CoordinationError {
    Io {
        path: PathBuf,
        operation: &'static str,
        source: std::io::Error,
    },
    Sqlite {
        path: PathBuf,
        operation: &'static str,
        source: rusqlite::Error,
    },
}

impl RunRecordSummary {
    pub fn validate(&self) -> Result<(), ContractError> {
        assert_supported_major(
            "run_record_summary.schema_version",
            &self.schema_version,
            CURRENT_SCHEMA_MAJOR,
        )
    }
}

fn default_schema_version() -> String {
    String::from("1.0.0")
}

impl StateLayout {
    pub fn from_root_layout(root_layout: &RootLayout) -> Self {
        Self::from_state_root(root_layout.state_dir.clone())
    }

    pub fn from_state_root(state_root: PathBuf) -> Self {
        Self {
            coordination_db_path: state_root.join(COORDINATION_DB_FILE_NAME),
            runs_dir: state_root.join(RUNS_DIR_NAME),
            workflow_logs_dir: state_root.join(WORKFLOW_LOGS_DIR_NAME),
            trigger_records_dir: state_root.join(TRIGGER_RECORDS_DIR_NAME),
            trigger_checkpoints_dir: state_root.join(TRIGGER_CHECKPOINTS_DIR_NAME),
            state_root,
        }
    }

    pub fn ensure_state_tree(&self) -> Result<(), FileStateError> {
        create_dir_all_file_state(&self.state_root)?;
        create_dir_all_file_state(&self.runs_dir)?;
        create_dir_all_file_state(&self.workflow_logs_dir)?;
        create_dir_all_file_state(&self.trigger_records_dir)?;
        create_dir_all_file_state(&self.trigger_checkpoints_dir)?;
        Ok(())
    }

    pub fn run_dir(&self, run_id: &str) -> PathBuf {
        self.runs_dir.join(sanitize_path_component(run_id))
    }

    pub fn run_summary_path(&self, run_id: &str) -> PathBuf {
        self.run_dir(run_id).join(RUN_SUMMARY_FILE_NAME)
    }

    pub fn staged_run_summary_path(&self, run_id: &str) -> PathBuf {
        staged_path(&self.run_summary_path(run_id))
    }

    pub fn workflow_log_entry_path(&self, run_id: &str, sequence: u64) -> PathBuf {
        self.workflow_logs_dir
            .join(sanitize_path_component(run_id))
            .join(format!("{sequence:020}.json"))
    }

    pub fn staged_workflow_log_entry_path(&self, run_id: &str, sequence: u64) -> PathBuf {
        staged_path(&self.workflow_log_entry_path(run_id, sequence))
    }

    pub fn trigger_record_path(
        &self,
        run_id: &str,
        sequence: u64,
        trigger_id: &str,
        event_id: &str,
    ) -> PathBuf {
        self.trigger_records_dir
            .join(sanitize_path_component(run_id))
            .join(trigger_record_file_name(sequence, trigger_id, event_id))
    }

    pub fn staged_trigger_record_path(
        &self,
        run_id: &str,
        sequence: u64,
        trigger_id: &str,
        event_id: &str,
    ) -> PathBuf {
        staged_path(&self.trigger_record_path(run_id, sequence, trigger_id, event_id))
    }

    pub fn trigger_checkpoint_path(&self, trigger_id: &str) -> PathBuf {
        self.trigger_checkpoints_dir
            .join(format!("{}.json", sanitize_path_component(trigger_id)))
    }

    pub fn staged_trigger_checkpoint_path(&self, trigger_id: &str) -> PathBuf {
        staged_path(&self.trigger_checkpoint_path(trigger_id))
    }
}

impl CoordinationTokenKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Dedup => "dedup",
            Self::Cooldown => "cooldown",
        }
    }
}

impl FileBackedStateStore {
    pub fn new(layout: StateLayout) -> Self {
        Self { layout }
    }

    pub fn layout(&self) -> &StateLayout {
        &self.layout
    }

    pub fn initialize(&self) -> Result<(), FileStateError> {
        self.layout.ensure_state_tree()
    }

    pub fn recover_runtime_state(
        &self,
        recovered_at_ms: i64,
    ) -> Result<RuntimeStateRecoveryReport, FileStateError> {
        self.layout.ensure_state_tree()?;

        let run_summaries = self.recover_run_summaries()?;
        let workflow_logs = self.recover_workflow_logs()?;
        let trigger_records = self.recover_trigger_records()?;
        let failed_incomplete_runs = self.fail_incomplete_runs(recovered_at_ms)?;

        Ok(RuntimeStateRecoveryReport {
            run_summaries,
            workflow_logs,
            trigger_records,
            failed_incomplete_runs,
        })
    }

    pub fn write_run_summary(&self, summary: &RunRecordSummary) -> Result<PathBuf, FileStateError> {
        summary
            .validate()
            .map_err(|source| FileStateError::InvalidRunSummary {
                path: self.layout.run_summary_path(&summary.run_id),
                source,
            })?;

        let summary_path = self.layout.run_summary_path(&summary.run_id);
        let staged_path = staged_path(&summary_path);
        atomic_write_json(&summary_path, &staged_path, summary)?;
        Ok(summary_path)
    }

    pub fn read_run_summary(&self, run_id: &str) -> Result<RunRecordSummary, FileStateError> {
        let path = self.layout.run_summary_path(run_id);
        load_run_summary_file(&path)
    }

    pub fn list_committed_run_summaries(&self) -> Result<Vec<RunRecordSummary>, FileStateError> {
        let mut summary_paths = Vec::new();
        collect_existing_json_files(&self.layout.runs_dir, &mut summary_paths)?;
        summary_paths.retain(|path| {
            path.file_name().and_then(|value| value.to_str()) == Some(RUN_SUMMARY_FILE_NAME)
        });
        summary_paths.sort();

        let mut summaries = Vec::with_capacity(summary_paths.len());
        for path in summary_paths {
            summaries.push(load_run_summary_file(&path)?);
        }

        summaries.sort_by(|left, right| {
            left.run_id
                .cmp(&right.run_id)
                .then(left.started_at_ms.cmp(&right.started_at_ms))
        });

        Ok(summaries)
    }

    pub fn list_run_summaries(&self) -> Result<Vec<RunRecordSummary>, FileStateError> {
        self.layout.ensure_state_tree()?;
        let _ = self.recover_run_summaries()?;

        self.list_committed_run_summaries()
    }

    pub fn write_workflow_log_entry(
        &self,
        entry: &WorkflowRuntimeLogEntry,
    ) -> Result<PathBuf, FileStateError> {
        let path = self
            .layout
            .workflow_log_entry_path(&entry.run_id, entry.sequence);
        let staged = staged_path(&path);
        atomic_write_json_append_only(&path, &staged, entry, "append workflow log entry")?;
        Ok(path)
    }

    pub fn append_workflow_log_entry(
        &self,
        run_id: &str,
        event: &str,
        message: &str,
        occurred_at_ms: i64,
    ) -> Result<PathBuf, FileStateError> {
        self.layout.ensure_state_tree()?;

        let next_sequence = self.next_workflow_log_sequence(run_id)?;
        self.write_workflow_log_entry(&WorkflowRuntimeLogEntry {
            run_id: run_id.to_owned(),
            sequence: next_sequence,
            event: event.to_owned(),
            message: message.to_owned(),
            occurred_at_ms,
        })
    }

    pub fn write_trigger_record(
        &self,
        entry: &TriggerEventRecord,
    ) -> Result<PathBuf, FileStateError> {
        self.layout.ensure_state_tree()?;
        let path = self.layout.trigger_record_path(
            &entry.run_id,
            entry.sequence,
            &entry.trigger_id,
            &entry.event_id,
        );
        let staged = staged_path(&path);
        atomic_write_json_append_only(&path, &staged, entry, "append trigger record")?;
        Ok(path)
    }

    pub fn write_trigger_checkpoint(
        &self,
        entry: &TriggerCheckpointRecord,
    ) -> Result<PathBuf, FileStateError> {
        self.layout.ensure_state_tree()?;
        let path = self.layout.trigger_checkpoint_path(&entry.trigger_id);
        let staged = self
            .layout
            .staged_trigger_checkpoint_path(&entry.trigger_id);
        atomic_write_json_file(&path, &staged, entry, "write trigger checkpoint")?;
        Ok(path)
    }

    pub fn read_trigger_checkpoint(
        &self,
        trigger_id: &str,
    ) -> Result<Option<TriggerCheckpointRecord>, FileStateError> {
        self.layout.ensure_state_tree()?;
        let path = self.layout.trigger_checkpoint_path(trigger_id);
        if !path.exists() {
            return Ok(None);
        }
        load_json_file(&path).map(Some)
    }

    pub fn recover_workflow_logs(&self) -> Result<FileArtifactRecoveryReport, FileStateError> {
        self.layout.ensure_state_tree()?;
        recover_append_only_entries(&self.layout.workflow_logs_dir, load_workflow_log_entry_file)
    }

    pub fn recover_trigger_records(&self) -> Result<FileArtifactRecoveryReport, FileStateError> {
        self.layout.ensure_state_tree()?;
        recover_append_only_entries(&self.layout.trigger_records_dir, load_trigger_record_file)
    }

    pub fn load_trigger_records(&self) -> Result<Vec<TriggerEventRecord>, FileStateError> {
        self.layout.ensure_state_tree()?;
        let _ = self.recover_trigger_records()?;

        self.load_committed_trigger_records()
    }

    pub fn load_committed_trigger_records(
        &self,
    ) -> Result<Vec<TriggerEventRecord>, FileStateError> {
        let mut record_paths = Vec::new();
        collect_existing_json_files(&self.layout.trigger_records_dir, &mut record_paths)?;
        record_paths.sort();

        let mut records = Vec::with_capacity(record_paths.len());
        for path in record_paths {
            records.push(load_trigger_record_file(&path)?);
        }

        Ok(records)
    }

    pub fn load_accepted_trigger_keys(&self) -> Result<Vec<String>, FileStateError> {
        let records = self.load_trigger_records()?;

        let mut keys = Vec::with_capacity(records.len());
        for record in records {
            keys.push(accepted_trigger_key(&record.trigger_id, &record.event_id));
        }

        Ok(keys)
    }

    pub fn recover_run_summaries(&self) -> Result<RunSummaryRecoveryReport, FileStateError> {
        self.layout.ensure_state_tree()?;

        let entries = fs::read_dir(&self.layout.runs_dir).map_err(|source| FileStateError::Io {
            path: self.layout.runs_dir.clone(),
            operation: "read directory",
            source,
        })?;

        let mut report = RunSummaryRecoveryReport::default();

        for entry in entries {
            let entry = entry.map_err(|source| FileStateError::Io {
                path: self.layout.runs_dir.clone(),
                operation: "read directory entry",
                source,
            })?;

            let run_dir = entry.path();
            if !run_dir.is_dir() {
                continue;
            }

            let summary_path = run_dir.join(RUN_SUMMARY_FILE_NAME);
            let staged_summary_path = staged_path(&summary_path);

            if !staged_summary_path.exists() {
                continue;
            }

            if summary_path.exists() {
                fs::remove_file(&staged_summary_path).map_err(|source| FileStateError::Io {
                    path: staged_summary_path.clone(),
                    operation: "remove staged run summary",
                    source,
                })?;
                report.removed_staged_files += 1;
                continue;
            }

            let _ = load_run_summary_file(&staged_summary_path)?;
            fs::rename(&staged_summary_path, &summary_path).map_err(|source| {
                FileStateError::Io {
                    path: staged_summary_path.clone(),
                    operation: "promote staged run summary",
                    source,
                }
            })?;

            if let Some(parent) = summary_path.parent() {
                sync_directory(parent)?;
            }
            report.promoted_staged_files += 1;
        }

        Ok(report)
    }

    fn fail_incomplete_runs(&self, recovered_at_ms: i64) -> Result<usize, FileStateError> {
        let summaries = self.list_run_summaries()?;
        let mut recovered_runs = 0;

        for summary in summaries {
            let should_fail = matches!(summary.status, RunStatus::Pending | RunStatus::Running)
                && summary.finished_at_ms.is_none();
            if !should_fail {
                continue;
            }

            self.append_workflow_log_entry(
                &summary.run_id,
                "run_recovered_after_restart",
                "restart recovery marked an incomplete run as failed",
                recovered_at_ms,
            )?;

            self.write_run_summary(&RunRecordSummary {
                status: RunStatus::Failed,
                finished_at_ms: Some(recovered_at_ms),
                ..summary
            })?;
            recovered_runs += 1;
        }

        Ok(recovered_runs)
    }

    fn next_workflow_log_sequence(&self, run_id: &str) -> Result<u64, FileStateError> {
        let run_dir = self
            .layout
            .workflow_log_entry_path(run_id, 1)
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| self.layout.workflow_logs_dir.clone());
        create_dir_all_file_state(&run_dir)?;

        let entries = fs::read_dir(&run_dir).map_err(|source| FileStateError::Io {
            path: run_dir.clone(),
            operation: "read workflow log directory",
            source,
        })?;

        let mut max_sequence = 0_u64;
        for entry in entries {
            let entry = entry.map_err(|source| FileStateError::Io {
                path: run_dir.clone(),
                operation: "read workflow log directory entry",
                source,
            })?;
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }

            let log_entry = load_workflow_log_entry_file(&path)?;
            max_sequence = max_sequence.max(log_entry.sequence);
        }

        Ok(max_sequence.saturating_add(1))
    }
}

impl CoordinationStore {
    pub fn inspect_existing_serve_lease(
        layout: &StateLayout,
        now_ms: i64,
    ) -> Result<ServeLeaseSnapshot, CoordinationError> {
        if !layout.coordination_db_path.exists() {
            return Ok(ServeLeaseSnapshot {
                state: ServeLeaseState::Idle,
                owner_id: None,
                expires_at_ms: None,
            });
        }

        let connection = Connection::open_with_flags(
            &layout.coordination_db_path,
            OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .map_err(|source| CoordinationError::Sqlite {
            path: layout.coordination_db_path.clone(),
            operation: "open sqlite database in read-only mode",
            source,
        })?;

        inspect_serve_lease_snapshot(&connection, &layout.coordination_db_path, now_ms)
    }

    pub fn open(layout: &StateLayout, now_ms: i64) -> Result<Self, CoordinationError> {
        create_dir_all_coordination(&layout.state_root)?;

        let mut connection = Connection::open(&layout.coordination_db_path).map_err(|source| {
            CoordinationError::Sqlite {
                path: layout.coordination_db_path.clone(),
                operation: "open sqlite database",
                source,
            }
        })?;

        connection
            .busy_timeout(Duration::from_millis(1_000))
            .map_err(|source| CoordinationError::Sqlite {
                path: layout.coordination_db_path.clone(),
                operation: "set sqlite busy timeout",
                source,
            })?;

        run_sqlite_migrations(&mut connection, &layout.coordination_db_path, now_ms)?;

        Ok(Self {
            db_path: layout.coordination_db_path.clone(),
            connection,
        })
    }

    pub fn database_path(&self) -> &Path {
        &self.db_path
    }

    pub fn applied_migration_versions(&self) -> Result<Vec<i64>, CoordinationError> {
        let mut statement = self
            .connection
            .prepare("SELECT version FROM schema_migrations ORDER BY version ASC")
            .map_err(|source| CoordinationError::Sqlite {
                path: self.db_path.clone(),
                operation: "prepare migration versions query",
                source,
            })?;

        let rows = statement
            .query_map([], |row| row.get::<_, i64>(0))
            .map_err(|source| CoordinationError::Sqlite {
                path: self.db_path.clone(),
                operation: "query migration versions",
                source,
            })?;

        let mut versions = Vec::new();
        for row in rows {
            let version = row.map_err(|source| CoordinationError::Sqlite {
                path: self.db_path.clone(),
                operation: "read migration version row",
                source,
            })?;
            versions.push(version);
        }

        Ok(versions)
    }

    pub fn inspect_serve_lease(
        &self,
        now_ms: i64,
    ) -> Result<ServeLeaseSnapshot, CoordinationError> {
        inspect_serve_lease_snapshot(&self.connection, &self.db_path, now_ms)
    }

    pub fn try_acquire_serve_lease(
        &mut self,
        owner_id: &str,
        now_ms: i64,
        lease_ttl_ms: i64,
    ) -> Result<LeaseAcquireResult, CoordinationError> {
        let expires_at_ms = now_ms.saturating_add(lease_ttl_ms.max(1));

        let transaction =
            self.connection
                .transaction()
                .map_err(|source| CoordinationError::Sqlite {
                    path: self.db_path.clone(),
                    operation: "start lease transaction",
                    source,
                })?;

        let current_lease = transaction
            .query_row(
                "SELECT owner_id, expires_at_ms FROM serve_leases WHERE lease_key = ?1",
                params![SERVE_LEASE_KEY],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()
            .map_err(|source| CoordinationError::Sqlite {
                path: self.db_path.clone(),
                operation: "read current serve lease",
                source,
            })?;

        let acquire_result = match current_lease {
            Some((current_owner, current_expires_at_ms))
                if current_expires_at_ms > now_ms
                    && current_owner != owner_id
                    && serve_lease_owner_is_active(&current_owner) =>
            {
                LeaseAcquireResult::Rejected {
                    current_owner,
                    expires_at_ms: current_expires_at_ms,
                }
            }
            Some((current_owner, current_expires_at_ms))
                if current_expires_at_ms > now_ms && current_owner == owner_id =>
            {
                write_lease_row(&transaction, owner_id, now_ms, expires_at_ms, &self.db_path)?;
                LeaseAcquireResult::Renewed
            }
            _ => {
                write_lease_row(&transaction, owner_id, now_ms, expires_at_ms, &self.db_path)?;
                LeaseAcquireResult::Acquired
            }
        };

        transaction
            .commit()
            .map_err(|source| CoordinationError::Sqlite {
                path: self.db_path.clone(),
                operation: "commit lease transaction",
                source,
            })?;

        Ok(acquire_result)
    }

    pub fn release_serve_lease(&mut self, owner_id: &str) -> Result<bool, CoordinationError> {
        let rows = self
            .connection
            .execute(
                "DELETE FROM serve_leases WHERE lease_key = ?1 AND owner_id = ?2",
                params![SERVE_LEASE_KEY, owner_id],
            )
            .map_err(|source| CoordinationError::Sqlite {
                path: self.db_path.clone(),
                operation: "release serve lease",
                source,
            })?;

        Ok(rows > 0)
    }

    pub fn mark_dedup_if_new(
        &mut self,
        dedup_key: &str,
        window_ms: i64,
        now_ms: i64,
    ) -> Result<bool, CoordinationError> {
        self.claim_token_if_ready(CoordinationTokenKind::Dedup, dedup_key, window_ms, now_ms)
    }

    pub fn begin_cooldown_if_ready(
        &mut self,
        cooldown_key: &str,
        duration_ms: i64,
        now_ms: i64,
    ) -> Result<bool, CoordinationError> {
        self.claim_token_if_ready(
            CoordinationTokenKind::Cooldown,
            cooldown_key,
            duration_ms,
            now_ms,
        )
    }

    pub fn dedup_is_ready(&self, dedup_key: &str, now_ms: i64) -> Result<bool, CoordinationError> {
        self.token_is_ready(CoordinationTokenKind::Dedup, dedup_key, now_ms)
    }

    pub fn cooldown_is_ready(
        &self,
        cooldown_key: &str,
        now_ms: i64,
    ) -> Result<bool, CoordinationError> {
        self.token_is_ready(CoordinationTokenKind::Cooldown, cooldown_key, now_ms)
    }

    pub fn rebuild_trigger_record_coordination(
        &mut self,
        records: &[TriggerEventRecord],
        now_ms: i64,
    ) -> Result<(), CoordinationError> {
        let transaction =
            self.connection
                .transaction()
                .map_err(|source| CoordinationError::Sqlite {
                    path: self.db_path.clone(),
                    operation: "start trigger coordination rebuild transaction",
                    source,
                })?;

        transaction
            .execute("DELETE FROM coordination_tokens", [])
            .map_err(|source| CoordinationError::Sqlite {
                path: self.db_path.clone(),
                operation: "clear coordination tokens before rebuild",
                source,
            })?;

        for record in records {
            upsert_trigger_record_coordination(&transaction, &self.db_path, record, now_ms)?;
        }

        transaction
            .commit()
            .map_err(|source| CoordinationError::Sqlite {
                path: self.db_path.clone(),
                operation: "commit trigger coordination rebuild transaction",
                source,
            })
    }

    pub fn apply_trigger_record_coordination(
        &mut self,
        record: &TriggerEventRecord,
        now_ms: i64,
    ) -> Result<(), CoordinationError> {
        let transaction =
            self.connection
                .transaction()
                .map_err(|source| CoordinationError::Sqlite {
                    path: self.db_path.clone(),
                    operation: "start trigger coordination write transaction",
                    source,
                })?;

        upsert_trigger_record_coordination(&transaction, &self.db_path, record, now_ms)?;

        transaction
            .commit()
            .map_err(|source| CoordinationError::Sqlite {
                path: self.db_path.clone(),
                operation: "commit trigger coordination write transaction",
                source,
            })
    }

    fn claim_token_if_ready(
        &mut self,
        token_kind: CoordinationTokenKind,
        token_key: &str,
        duration_ms: i64,
        now_ms: i64,
    ) -> Result<bool, CoordinationError> {
        let next_expires_at_ms = now_ms.saturating_add(duration_ms.max(0));

        let transaction =
            self.connection
                .transaction()
                .map_err(|source| CoordinationError::Sqlite {
                    path: self.db_path.clone(),
                    operation: "start token transaction",
                    source,
                })?;

        let current_expires_at = transaction
            .query_row(
                "SELECT expires_at_ms FROM coordination_tokens WHERE token_kind = ?1 AND token_key = ?2",
                params![token_kind.as_str(), token_key],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(|source| CoordinationError::Sqlite {
                path: self.db_path.clone(),
                operation: "read coordination token",
                source,
            })?;

        if let Some(expires_at_ms) = current_expires_at {
            if expires_at_ms > now_ms {
                transaction
                    .commit()
                    .map_err(|source| CoordinationError::Sqlite {
                        path: self.db_path.clone(),
                        operation: "commit token read transaction",
                        source,
                    })?;
                return Ok(false);
            }
        }

        transaction
            .execute(
                "INSERT INTO coordination_tokens (token_kind, token_key, expires_at_ms, updated_at_ms)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(token_kind, token_key)
                 DO UPDATE SET expires_at_ms = excluded.expires_at_ms, updated_at_ms = excluded.updated_at_ms",
                params![token_kind.as_str(), token_key, next_expires_at_ms, now_ms],
            )
            .map_err(|source| CoordinationError::Sqlite {
                path: self.db_path.clone(),
                operation: "upsert coordination token",
                source,
            })?;

        transaction
            .commit()
            .map_err(|source| CoordinationError::Sqlite {
                path: self.db_path.clone(),
                operation: "commit token transaction",
                source,
            })?;

        Ok(true)
    }

    fn token_is_ready(
        &self,
        token_kind: CoordinationTokenKind,
        token_key: &str,
        now_ms: i64,
    ) -> Result<bool, CoordinationError> {
        let current_expires_at = self
            .connection
            .query_row(
                "SELECT expires_at_ms FROM coordination_tokens WHERE token_kind = ?1 AND token_key = ?2",
                params![token_kind.as_str(), token_key],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(|source| CoordinationError::Sqlite {
                path: self.db_path.clone(),
                operation: "read coordination token readiness",
                source,
            })?;

        Ok(current_expires_at.is_none_or(|expires_at_ms| expires_at_ms <= now_ms))
    }
}

impl Display for FileStateError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io {
                path,
                operation,
                source,
            } => write!(f, "failed to {operation} at {}: {source}", path.display()),
            Self::JsonEncode { path, source } => {
                write!(f, "failed to encode JSON for {}: {source}", path.display())
            }
            Self::JsonDecode { path, source } => {
                write!(f, "failed to decode JSON at {}: {source}", path.display())
            }
            Self::MissingRunSummary { path } => {
                write!(f, "missing run summary file: {}", path.display())
            }
            Self::InvalidRunSummary { path, source } => write!(
                f,
                "run summary validation failed at {}: {source}",
                path.display()
            ),
            Self::ImmutableFileExists { path, operation } => write!(
                f,
                "failed to {operation} because append-only path already exists: {}",
                path.display()
            ),
        }
    }
}

impl Error for FileStateError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::JsonEncode { source, .. } => Some(source),
            Self::JsonDecode { source, .. } => Some(source),
            Self::InvalidRunSummary { source, .. } => Some(source),
            Self::MissingRunSummary { .. } | Self::ImmutableFileExists { .. } => None,
        }
    }
}

impl Display for CoordinationError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io {
                path,
                operation,
                source,
            } => write!(f, "failed to {operation} at {}: {source}", path.display()),
            Self::Sqlite {
                path,
                operation,
                source,
            } => write!(f, "failed to {operation} at {}: {source}", path.display()),
        }
    }
}

impl Error for CoordinationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Sqlite { source, .. } => Some(source),
        }
    }
}

fn load_run_summary_file(path: &Path) -> Result<RunRecordSummary, FileStateError> {
    let contents = fs::read_to_string(path).map_err(|source| {
        if source.kind() == std::io::ErrorKind::NotFound {
            return FileStateError::MissingRunSummary {
                path: path.to_path_buf(),
            };
        }

        FileStateError::Io {
            path: path.to_path_buf(),
            operation: "read run summary",
            source,
        }
    })?;

    let summary: RunRecordSummary =
        serde_json::from_str(&contents).map_err(|source| FileStateError::JsonDecode {
            path: path.to_path_buf(),
            source,
        })?;

    summary
        .validate()
        .map_err(|source| FileStateError::InvalidRunSummary {
            path: path.to_path_buf(),
            source,
        })?;

    Ok(summary)
}

fn load_workflow_log_entry_file(path: &Path) -> Result<WorkflowRuntimeLogEntry, FileStateError> {
    load_json_file(path)
}

fn load_trigger_record_file(path: &Path) -> Result<TriggerEventRecord, FileStateError> {
    load_json_file(path)
}

fn load_json_file<T>(path: &Path) -> Result<T, FileStateError>
where
    T: DeserializeOwned,
{
    let contents = fs::read_to_string(path).map_err(|source| FileStateError::Io {
        path: path.to_path_buf(),
        operation: "read JSON file",
        source,
    })?;

    serde_json::from_str(&contents).map_err(|source| FileStateError::JsonDecode {
        path: path.to_path_buf(),
        source,
    })
}

fn create_dir_all_file_state(path: &Path) -> Result<(), FileStateError> {
    fs::create_dir_all(path).map_err(|source| FileStateError::Io {
        path: path.to_path_buf(),
        operation: "create directory",
        source,
    })
}

fn create_dir_all_coordination(path: &Path) -> Result<(), CoordinationError> {
    fs::create_dir_all(path).map_err(|source| CoordinationError::Io {
        path: path.to_path_buf(),
        operation: "create directory",
        source,
    })
}

fn write_lease_row(
    transaction: &rusqlite::Transaction<'_>,
    owner_id: &str,
    acquired_at_ms: i64,
    expires_at_ms: i64,
    db_path: &Path,
) -> Result<(), CoordinationError> {
    transaction
        .execute(
            "INSERT INTO serve_leases (lease_key, owner_id, acquired_at_ms, expires_at_ms)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(lease_key)
             DO UPDATE SET owner_id = excluded.owner_id,
                           acquired_at_ms = excluded.acquired_at_ms,
                           expires_at_ms = excluded.expires_at_ms",
            params![SERVE_LEASE_KEY, owner_id, acquired_at_ms, expires_at_ms],
        )
        .map_err(|source| CoordinationError::Sqlite {
            path: db_path.to_path_buf(),
            operation: "upsert serve lease",
            source,
        })?;
    Ok(())
}

fn upsert_trigger_record_coordination(
    transaction: &rusqlite::Transaction<'_>,
    db_path: &Path,
    record: &TriggerEventRecord,
    now_ms: i64,
) -> Result<(), CoordinationError> {
    upsert_coordination_token_if_unexpired(
        transaction,
        db_path,
        CoordinationTokenKind::Dedup,
        record.dedup_key.as_deref(),
        record.dedup_expires_at_ms,
        now_ms,
    )?;
    upsert_coordination_token_if_unexpired(
        transaction,
        db_path,
        CoordinationTokenKind::Cooldown,
        record.cooldown_key.as_deref(),
        record.cooldown_expires_at_ms,
        now_ms,
    )?;
    Ok(())
}

fn upsert_coordination_token_if_unexpired(
    transaction: &rusqlite::Transaction<'_>,
    db_path: &Path,
    token_kind: CoordinationTokenKind,
    token_key: Option<&str>,
    expires_at_ms: Option<i64>,
    now_ms: i64,
) -> Result<(), CoordinationError> {
    let (Some(token_key), Some(expires_at_ms)) = (token_key, expires_at_ms) else {
        return Ok(());
    };
    if expires_at_ms <= now_ms {
        return Ok(());
    }

    transaction
        .execute(
            "INSERT INTO coordination_tokens (token_kind, token_key, expires_at_ms, updated_at_ms)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(token_kind, token_key)
             DO UPDATE SET expires_at_ms = CASE
                                WHEN coordination_tokens.expires_at_ms > excluded.expires_at_ms
                                THEN coordination_tokens.expires_at_ms
                                ELSE excluded.expires_at_ms
                            END,
                            updated_at_ms = excluded.updated_at_ms",
            params![token_kind.as_str(), token_key, expires_at_ms, now_ms],
        )
        .map_err(|source| CoordinationError::Sqlite {
            path: db_path.to_path_buf(),
            operation: "upsert trigger coordination token",
            source,
        })?;

    Ok(())
}

fn run_sqlite_migrations(
    connection: &mut Connection,
    db_path: &Path,
    now_ms: i64,
) -> Result<(), CoordinationError> {
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_migrations (
                version INTEGER PRIMARY KEY,
                applied_at_ms INTEGER NOT NULL
            );",
        )
        .map_err(|source| CoordinationError::Sqlite {
            path: db_path.to_path_buf(),
            operation: "create schema migration table",
            source,
        })?;

    let version_exists = connection
        .query_row(
            "SELECT version FROM schema_migrations WHERE version = ?1",
            params![SQLITE_COORDINATION_SCHEMA_VERSION],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(|source| CoordinationError::Sqlite {
            path: db_path.to_path_buf(),
            operation: "query schema migration version",
            source,
        })?;

    if version_exists.is_some() {
        return Ok(());
    }

    let transaction = connection
        .transaction()
        .map_err(|source| CoordinationError::Sqlite {
            path: db_path.to_path_buf(),
            operation: "start migration transaction",
            source,
        })?;

    transaction
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS serve_leases (
                lease_key TEXT PRIMARY KEY,
                owner_id TEXT NOT NULL,
                acquired_at_ms INTEGER NOT NULL,
                expires_at_ms INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS coordination_tokens (
                token_kind TEXT NOT NULL,
                token_key TEXT NOT NULL,
                expires_at_ms INTEGER NOT NULL,
                updated_at_ms INTEGER NOT NULL,
                PRIMARY KEY (token_kind, token_key)
            );",
        )
        .map_err(|source| CoordinationError::Sqlite {
            path: db_path.to_path_buf(),
            operation: "apply coordination schema migration",
            source,
        })?;

    transaction
        .execute(
            "INSERT INTO schema_migrations (version, applied_at_ms) VALUES (?1, ?2)",
            params![SQLITE_COORDINATION_SCHEMA_VERSION, now_ms],
        )
        .map_err(|source| CoordinationError::Sqlite {
            path: db_path.to_path_buf(),
            operation: "record schema migration",
            source,
        })?;

    transaction
        .commit()
        .map_err(|source| CoordinationError::Sqlite {
            path: db_path.to_path_buf(),
            operation: "commit migration transaction",
            source,
        })
}

fn recover_append_only_entries<T>(
    root: &Path,
    loader: fn(&Path) -> Result<T, FileStateError>,
) -> Result<FileArtifactRecoveryReport, FileStateError> {
    let mut staged_paths = Vec::new();
    collect_staged_json_files(root, &mut staged_paths)?;
    staged_paths.sort();

    let mut report = FileArtifactRecoveryReport::default();
    for staged in staged_paths {
        let committed = committed_path_from_staged(&staged);
        if committed.exists() {
            fs::remove_file(&staged).map_err(|source| FileStateError::Io {
                path: staged.clone(),
                operation: "remove stale staged file",
                source,
            })?;
            if let Some(parent) = staged.parent() {
                sync_directory(parent)?;
            }
            report.removed_staged_files += 1;
            continue;
        }

        let _ = loader(&staged)?;
        fs::rename(&staged, &committed).map_err(|source| FileStateError::Io {
            path: staged.clone(),
            operation: "promote staged append-only file",
            source,
        })?;
        if let Some(parent) = committed.parent() {
            sync_directory(parent)?;
        }
        report.promoted_staged_files += 1;
    }

    Ok(report)
}

fn atomic_write_json<T>(path: &Path, staged_path: &Path, value: &T) -> Result<(), FileStateError>
where
    T: Serialize,
{
    if let Some(parent) = path.parent() {
        create_dir_all_file_state(parent)?;
    }

    let payload =
        serde_json::to_vec_pretty(value).map_err(|source| FileStateError::JsonEncode {
            path: path.to_path_buf(),
            source,
        })?;

    let mut file = File::create(staged_path).map_err(|source| FileStateError::Io {
        path: staged_path.to_path_buf(),
        operation: "create staged file",
        source,
    })?;

    file.write_all(&payload)
        .map_err(|source| FileStateError::Io {
            path: staged_path.to_path_buf(),
            operation: "write staged file",
            source,
        })?;

    file.sync_all().map_err(|source| FileStateError::Io {
        path: staged_path.to_path_buf(),
        operation: "sync staged file",
        source,
    })?;

    fs::rename(staged_path, path).map_err(|source| FileStateError::Io {
        path: staged_path.to_path_buf(),
        operation: "promote staged file",
        source,
    })?;

    if let Some(parent) = path.parent() {
        sync_directory(parent)?;
    }

    Ok(())
}

fn atomic_write_json_append_only<T>(
    path: &Path,
    staged_path: &Path,
    value: &T,
    operation: &'static str,
) -> Result<(), FileStateError>
where
    T: Serialize,
{
    if path.exists() {
        return Err(FileStateError::ImmutableFileExists {
            path: path.to_path_buf(),
            operation,
        });
    }

    atomic_write_json(path, staged_path, value)
}

fn atomic_write_json_file<T>(
    path: &Path,
    staged_path: &Path,
    value: &T,
    _operation: &'static str,
) -> Result<(), FileStateError>
where
    T: Serialize,
{
    atomic_write_json(path, staged_path, value)
}

fn sync_directory(path: &Path) -> Result<(), FileStateError> {
    let dir = File::open(path).map_err(|source| FileStateError::Io {
        path: path.to_path_buf(),
        operation: "open directory for sync",
        source,
    })?;

    dir.sync_all().map_err(|source| FileStateError::Io {
        path: path.to_path_buf(),
        operation: "sync directory",
        source,
    })
}

fn staged_path(path: &Path) -> PathBuf {
    match path.file_name() {
        Some(file_name) => {
            let mut staged_name = file_name.to_os_string();
            staged_name.push(STAGED_FILE_SUFFIX);
            path.with_file_name(staged_name)
        }
        None => {
            let mut fallback = path.as_os_str().to_os_string();
            fallback.push(STAGED_FILE_SUFFIX);
            PathBuf::from(fallback)
        }
    }
}

fn committed_path_from_staged(path: &Path) -> PathBuf {
    let Some(file_name) = path.file_name().and_then(|value| value.to_str()) else {
        return path.to_path_buf();
    };

    let committed_name = file_name
        .strip_suffix(STAGED_FILE_SUFFIX)
        .unwrap_or(file_name);
    path.with_file_name(committed_name)
}

fn collect_staged_json_files(root: &Path, output: &mut Vec<PathBuf>) -> Result<(), FileStateError> {
    let entries = fs::read_dir(root).map_err(|source| FileStateError::Io {
        path: root.to_path_buf(),
        operation: "read directory",
        source,
    })?;

    for entry in entries {
        let entry = entry.map_err(|source| FileStateError::Io {
            path: root.to_path_buf(),
            operation: "read directory entry",
            source,
        })?;
        let path = entry.path();
        if path.is_dir() {
            collect_staged_json_files(&path, output)?;
            continue;
        }

        let Some(file_name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if file_name.ends_with(&format!(".json{STAGED_FILE_SUFFIX}")) {
            output.push(path);
        }
    }

    Ok(())
}

fn collect_json_files(root: &Path, output: &mut Vec<PathBuf>) -> Result<(), FileStateError> {
    let entries = fs::read_dir(root).map_err(|source| FileStateError::Io {
        path: root.to_path_buf(),
        operation: "read directory",
        source,
    })?;

    for entry in entries {
        let entry = entry.map_err(|source| FileStateError::Io {
            path: root.to_path_buf(),
            operation: "read directory entry",
            source,
        })?;
        let path = entry.path();
        if path.is_dir() {
            collect_json_files(&path, output)?;
            continue;
        }
        if path.extension().and_then(|value| value.to_str()) == Some("json") {
            output.push(path);
        }
    }

    Ok(())
}

fn collect_existing_json_files(
    root: &Path,
    output: &mut Vec<PathBuf>,
) -> Result<(), FileStateError> {
    match fs::metadata(root) {
        Ok(metadata) if metadata.is_dir() => collect_json_files(root, output),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(FileStateError::Io {
            path: root.to_path_buf(),
            operation: "inspect metadata",
            source,
        }),
    }
}

fn inspect_serve_lease_snapshot(
    connection: &Connection,
    db_path: &Path,
    now_ms: i64,
) -> Result<ServeLeaseSnapshot, CoordinationError> {
    let current_lease = connection
        .query_row(
            "SELECT owner_id, expires_at_ms FROM serve_leases WHERE lease_key = ?1",
            params![SERVE_LEASE_KEY],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()
        .map_err(|source| CoordinationError::Sqlite {
            path: db_path.to_path_buf(),
            operation: "inspect current serve lease",
            source,
        })?;

    let snapshot = match current_lease {
        None => ServeLeaseSnapshot {
            state: ServeLeaseState::Idle,
            owner_id: None,
            expires_at_ms: None,
        },
        Some((owner_id, expires_at_ms))
            if expires_at_ms > now_ms && serve_lease_owner_is_active(&owner_id) =>
        {
            ServeLeaseSnapshot {
                state: ServeLeaseState::Active,
                owner_id: Some(owner_id),
                expires_at_ms: Some(expires_at_ms),
            }
        }
        Some((owner_id, expires_at_ms)) => ServeLeaseSnapshot {
            state: ServeLeaseState::Stale,
            owner_id: Some(owner_id),
            expires_at_ms: Some(expires_at_ms),
        },
    };

    Ok(snapshot)
}

fn accepted_trigger_key(trigger_id: &str, event_id: &str) -> String {
    format!("{trigger_id}:::{event_id}")
}

fn serve_lease_owner_is_active(owner_id: &str) -> bool {
    let Some(pid_raw) = owner_id.strip_prefix(SERVE_OWNER_ID_PREFIX) else {
        return true;
    };
    let Ok(pid) = pid_raw.parse::<u32>() else {
        return true;
    };

    let Ok(output) = Command::new("ps").arg("-p").arg(pid.to_string()).output() else {
        return true;
    };

    output.status.success() && String::from_utf8_lossy(&output.stdout).lines().count() > 1
}

pub(crate) fn sanitize_path_component(value: &str) -> String {
    let mut sanitized = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
            sanitized.push(ch);
        } else {
            sanitized.push('_');
        }
    }

    if sanitized.is_empty() {
        sanitized.push('_');
    }

    sanitized
}

pub(crate) fn trigger_record_file_name(sequence: u64, trigger_id: &str, event_id: &str) -> String {
    format!(
        "{sequence:020}-{}-{}.json",
        sanitize_path_component(trigger_id),
        sanitize_path_component(event_id)
    )
}
