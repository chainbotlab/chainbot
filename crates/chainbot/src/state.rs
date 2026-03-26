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

use std::collections::{BTreeMap, BTreeSet};
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
pub const CANONICAL_TRIGGERS_DIR_NAME: &str = "triggers";
pub const CANONICAL_TRIGGER_RECORDS_DIR_NAME: &str = "records";
pub const CANONICAL_TRIGGER_CHECKPOINT_FILE_NAME: &str = "checkpoint.json";
pub const CANONICAL_TRIGGER_SNAPSHOT_FILE_NAME: &str = "snapshot.json";
pub const SERVE_OWNER_ID_PREFIX: &str = "chainbot-serve-pid-";

const RUN_SUMMARY_FILE_NAME: &str = "summary.json";
const STAGED_FILE_SUFFIX: &str = ".next";
const SERVE_LEASE_KEY: &str = "serve";
const SQLITE_COORDINATION_SCHEMA_VERSION: i64 = 1;
const WORKFLOW_LOG_SEQUENCE_CURSOR_FILE_NAME: &str = "workflow-log-sequence.cursor";

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
pub struct IngressInboxRecord {
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    pub inbox_id: String,
    pub trigger_id: String,
    pub workflow_id: String,
    pub transport_kind: String,
    pub ingress_event_id: String,
    pub source: String,
    pub route_path: String,
    #[serde(default)]
    pub http_method: Option<String>,
    pub received_at_ms: i64,
    #[serde(default)]
    pub payload: serde_json::Value,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub remote_addr: Option<String>,
    #[serde(default)]
    pub processed_at_ms: Option<i64>,
    #[serde(default)]
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StagedTriggerEventRecord {
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    pub staging_id: String,
    pub trigger_id: String,
    pub workflow_id: String,
    pub event_id: String,
    pub source: String,
    pub occurred_at_ms: i64,
    pub staged_at_ms: i64,
    #[serde(default)]
    pub checkpoint: Option<String>,
    #[serde(default)]
    pub payload: serde_json::Value,
    #[serde(default)]
    pub dedup_key: Option<String>,
    #[serde(default)]
    pub dedup_window_ms: Option<i64>,
    #[serde(default)]
    pub cooldown_key: Option<String>,
    #[serde(default)]
    pub cooldown_ms: Option<i64>,
    #[serde(default)]
    pub accepted_at_ms: Option<i64>,
    #[serde(default)]
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerCheckpointRecord {
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    pub trigger_id: String,
    pub checkpoint: String,
    pub acked_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerTokenSnapshot {
    pub key: String,
    pub expires_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerSnapshotRecord {
    #[serde(default = "default_schema_version")]
    pub schema_version: String,
    pub trigger_id: String,
    #[serde(default)]
    pub last_event_id: Option<String>,
    #[serde(default)]
    pub last_accepted_at_ms: Option<i64>,
    #[serde(default)]
    pub last_sequence: u64,
    #[serde(default)]
    pub accepted_event_ids: BTreeSet<String>,
    #[serde(default)]
    pub dedup_tokens: Vec<TriggerTokenSnapshot>,
    #[serde(default)]
    pub cooldown_tokens: Vec<TriggerTokenSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateLayout {
    pub state_root: PathBuf,
    pub coordination_db_path: PathBuf,
    pub runs_dir: PathBuf,
    pub trigger_state_dir: PathBuf,
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
    pub trigger_snapshots: FileArtifactRecoveryReport,
    pub failed_incomplete_runs: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MixedCheckpointConflict {
    pub trigger_id: String,
    pub canonical_path: PathBuf,
    pub legacy_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DuplicateTriggerHistoryConflict {
    pub trigger_id: String,
    pub event_id: String,
    pub canonical_path: PathBuf,
    pub legacy_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LegacyStateInspection {
    pub legacy_workflow_log_paths: Vec<PathBuf>,
    pub legacy_trigger_record_paths: Vec<PathBuf>,
    pub legacy_trigger_checkpoint_paths: Vec<PathBuf>,
    pub dual_checkpoint_conflicts: Vec<MixedCheckpointConflict>,
    pub duplicate_trigger_history_conflicts: Vec<DuplicateTriggerHistoryConflict>,
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

impl TriggerSnapshotRecord {
    pub fn new(trigger_id: impl Into<String>) -> Self {
        Self {
            schema_version: default_schema_version(),
            trigger_id: trigger_id.into(),
            last_event_id: None,
            last_accepted_at_ms: None,
            last_sequence: 0,
            accepted_event_ids: BTreeSet::new(),
            dedup_tokens: Vec::new(),
            cooldown_tokens: Vec::new(),
        }
    }

    pub fn apply_record(&mut self, record: &TriggerEventRecord) {
        self.last_event_id = Some(record.event_id.clone());
        self.last_accepted_at_ms = Some(record.accepted_at_ms);
        self.last_sequence = self.last_sequence.max(record.sequence);
        self.accepted_event_ids.insert(record.event_id.clone());
        retain_unexpired_tokens(&mut self.dedup_tokens, record.accepted_at_ms);
        retain_unexpired_tokens(&mut self.cooldown_tokens, record.accepted_at_ms);
        upsert_token_snapshot(
            &mut self.dedup_tokens,
            record.dedup_key.as_deref(),
            record.dedup_expires_at_ms,
            record.accepted_at_ms,
        );
        upsert_token_snapshot(
            &mut self.cooldown_tokens,
            record.cooldown_key.as_deref(),
            record.cooldown_expires_at_ms,
            record.accepted_at_ms,
        );
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
            trigger_state_dir: state_root.join(CANONICAL_TRIGGERS_DIR_NAME),
            workflow_logs_dir: state_root.join(WORKFLOW_LOGS_DIR_NAME),
            trigger_records_dir: state_root.join(TRIGGER_RECORDS_DIR_NAME),
            trigger_checkpoints_dir: state_root.join(TRIGGER_CHECKPOINTS_DIR_NAME),
            state_root,
        }
    }

    pub fn ensure_state_tree(&self) -> Result<(), FileStateError> {
        create_dir_all_file_state(&self.state_root)?;
        create_dir_all_file_state(&self.runs_dir)?;
        create_dir_all_file_state(&self.trigger_state_dir)?;
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
        self.run_dir(run_id)
            .join(WORKFLOW_LOGS_DIR_NAME)
            .join(format!("{sequence:020}.json"))
    }

    pub fn staged_workflow_log_entry_path(&self, run_id: &str, sequence: u64) -> PathBuf {
        staged_path(&self.workflow_log_entry_path(run_id, sequence))
    }

    pub fn trigger_record_path(
        &self,
        _run_id: &str,
        sequence: u64,
        trigger_id: &str,
        event_id: &str,
    ) -> PathBuf {
        self.trigger_state_dir
            .join(sanitize_path_component(trigger_id))
            .join(CANONICAL_TRIGGER_RECORDS_DIR_NAME)
            .join(canonical_trigger_record_file_name(sequence, event_id))
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
        self.trigger_state_dir
            .join(sanitize_path_component(trigger_id))
            .join(CANONICAL_TRIGGER_CHECKPOINT_FILE_NAME)
    }

    pub fn staged_trigger_checkpoint_path(&self, trigger_id: &str) -> PathBuf {
        staged_path(&self.trigger_checkpoint_path(trigger_id))
    }

    pub fn trigger_snapshot_path(&self, trigger_id: &str) -> PathBuf {
        self.trigger_state_dir
            .join(sanitize_path_component(trigger_id))
            .join(CANONICAL_TRIGGER_SNAPSHOT_FILE_NAME)
    }

    pub fn staged_trigger_snapshot_path(&self, trigger_id: &str) -> PathBuf {
        staged_path(&self.trigger_snapshot_path(trigger_id))
    }

    pub fn workflow_log_sequence_cursor_path(&self, run_id: &str) -> PathBuf {
        self.run_dir(run_id)
            .join(WORKFLOW_LOG_SEQUENCE_CURSOR_FILE_NAME)
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
        let trigger_snapshots = self.recover_trigger_snapshots()?;
        let failed_incomplete_runs = self.fail_incomplete_runs(recovered_at_ms)?;

        Ok(RuntimeStateRecoveryReport {
            run_summaries,
            workflow_logs,
            trigger_records,
            trigger_snapshots,
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
        self.layout.ensure_state_tree()?;
        let mut summary_paths = Vec::new();
        let entries = fs::read_dir(&self.layout.runs_dir).map_err(|source| FileStateError::Io {
            path: self.layout.runs_dir.clone(),
            operation: "read runs directory",
            source,
        })?;
        for entry in entries {
            let entry = entry.map_err(|source| FileStateError::Io {
                path: self.layout.runs_dir.clone(),
                operation: "read runs directory entry",
                source,
            })?;
            let run_dir = entry.path();
            if !run_dir.is_dir() {
                continue;
            }
            let summary_path = run_dir.join(RUN_SUMMARY_FILE_NAME);
            if summary_path.exists() {
                summary_paths.push(summary_path);
            }
        }
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

        let mut next_sequence = self.next_workflow_log_sequence(run_id)?;
        loop {
            match self.write_workflow_log_entry(&WorkflowRuntimeLogEntry {
                run_id: run_id.to_owned(),
                sequence: next_sequence,
                event: event.to_owned(),
                message: message.to_owned(),
                occurred_at_ms,
            }) {
                Ok(path) => {
                    let _ = self.write_workflow_log_sequence_cursor(
                        run_id,
                        next_sequence.saturating_add(1),
                    );
                    return Ok(path);
                }
                Err(FileStateError::ImmutableFileExists { .. }) => {
                    next_sequence = next_sequence.saturating_add(1);
                }
                Err(error) => return Err(error),
            }
        }
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

    pub fn write_trigger_snapshot(
        &self,
        entry: &TriggerSnapshotRecord,
    ) -> Result<PathBuf, FileStateError> {
        self.layout.ensure_state_tree()?;
        let path = self.layout.trigger_snapshot_path(&entry.trigger_id);
        let staged = self.layout.staged_trigger_snapshot_path(&entry.trigger_id);
        atomic_write_json_file(&path, &staged, entry, "write trigger snapshot")?;
        Ok(path)
    }

    pub fn read_trigger_checkpoint(
        &self,
        trigger_id: &str,
    ) -> Result<Option<TriggerCheckpointRecord>, FileStateError> {
        self.layout.ensure_state_tree()?;
        let path = self.layout.trigger_checkpoint_path(trigger_id);
        if path.exists() {
            return load_json_file(&path).map(Some);
        }

        Ok(None)
    }

    pub fn read_trigger_snapshot(
        &self,
        trigger_id: &str,
    ) -> Result<Option<TriggerSnapshotRecord>, FileStateError> {
        self.layout.ensure_state_tree()?;
        let path = self.layout.trigger_snapshot_path(trigger_id);
        if path.exists() {
            return load_json_file(&path).map(Some);
        }

        Ok(None)
    }

    pub fn load_committed_trigger_snapshots(
        &self,
    ) -> Result<Vec<TriggerSnapshotRecord>, FileStateError> {
        self.layout.ensure_state_tree()?;
        let mut snapshots = Vec::<TriggerSnapshotRecord>::new();
        for directory in self.trigger_state_dirs()? {
            let path = directory.join(CANONICAL_TRIGGER_SNAPSHOT_FILE_NAME);
            if path.exists() {
                snapshots.push(load_json_file(&path)?);
            }
        }
        snapshots.sort_by(|left, right| left.trigger_id.cmp(&right.trigger_id));
        Ok(snapshots)
    }

    pub fn load_trigger_records_after_sequence(
        &self,
        trigger_id: &str,
        sequence_exclusive: u64,
    ) -> Result<Vec<TriggerEventRecord>, FileStateError> {
        self.layout.ensure_state_tree()?;
        let directory = self
            .layout
            .trigger_state_dir
            .join(sanitize_path_component(trigger_id))
            .join(CANONICAL_TRIGGER_RECORDS_DIR_NAME);
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(source) => {
                return Err(FileStateError::Io {
                    path: directory,
                    operation: "read trigger record directory",
                    source,
                });
            }
        };

        let mut record_paths = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|source| FileStateError::Io {
                path: directory.clone(),
                operation: "read trigger record directory entry",
                source,
            })?;
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let Some(sequence) = parse_numeric_file_stem_prefix(&path) else {
                continue;
            };
            if sequence > sequence_exclusive {
                record_paths.push(path);
            }
        }
        record_paths.sort();

        let mut records = Vec::with_capacity(record_paths.len());
        for path in record_paths {
            records.push(load_trigger_record_file(&path)?);
        }
        Ok(records)
    }

    pub fn recover_workflow_logs(&self) -> Result<FileArtifactRecoveryReport, FileStateError> {
        self.layout.ensure_state_tree()?;
        let mut report = FileArtifactRecoveryReport::default();
        for directory in self.workflow_log_roots()? {
            let recovered =
                recover_append_only_entries_if_exists(&directory, load_workflow_log_entry_file)?;
            report.promoted_staged_files += recovered.promoted_staged_files;
            report.removed_staged_files += recovered.removed_staged_files;
        }
        Ok(report)
    }

    pub fn recover_trigger_records(&self) -> Result<FileArtifactRecoveryReport, FileStateError> {
        self.layout.ensure_state_tree()?;
        let mut report = FileArtifactRecoveryReport::default();
        for directory in self.trigger_record_roots()? {
            let recovered =
                recover_append_only_entries_if_exists(&directory, load_trigger_record_file)?;
            report.promoted_staged_files += recovered.promoted_staged_files;
            report.removed_staged_files += recovered.removed_staged_files;
        }
        Ok(report)
    }

    pub fn recover_trigger_snapshots(&self) -> Result<FileArtifactRecoveryReport, FileStateError> {
        self.layout.ensure_state_tree()?;
        let mut report = FileArtifactRecoveryReport::default();
        for trigger_dir in self.trigger_state_dirs()? {
            let snapshot_path = trigger_dir.join(CANONICAL_TRIGGER_SNAPSHOT_FILE_NAME);
            let staged_snapshot_path = staged_path(&snapshot_path);
            if !staged_snapshot_path.exists() {
                continue;
            }
            if snapshot_path.exists() {
                fs::remove_file(&staged_snapshot_path).map_err(|source| FileStateError::Io {
                    path: staged_snapshot_path.clone(),
                    operation: "remove staged trigger snapshot",
                    source,
                })?;
                report.removed_staged_files += 1;
                continue;
            }
            let _ = load_trigger_snapshot_file(&staged_snapshot_path)?;
            fs::rename(&staged_snapshot_path, &snapshot_path).map_err(|source| {
                FileStateError::Io {
                    path: staged_snapshot_path.clone(),
                    operation: "promote staged trigger snapshot",
                    source,
                }
            })?;
            if let Some(parent) = snapshot_path.parent() {
                sync_directory(parent)?;
            }
            report.promoted_staged_files += 1;
        }
        Ok(report)
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
        for directory in self.trigger_record_roots()? {
            collect_existing_json_files(&directory, &mut record_paths)?;
        }
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

    pub fn inspect_legacy_usage(&self) -> Result<LegacyStateInspection, FileStateError> {
        let legacy_workflow_log_paths = self.collect_legacy_workflow_log_paths()?;
        let legacy_trigger_record_paths = self.collect_legacy_trigger_record_paths()?;
        let legacy_trigger_checkpoint_paths = self.collect_legacy_trigger_checkpoint_paths()?;

        Ok(LegacyStateInspection {
            dual_checkpoint_conflicts: self
                .detect_dual_checkpoints(&legacy_trigger_checkpoint_paths),
            duplicate_trigger_history_conflicts: self
                .detect_duplicate_trigger_history(&legacy_trigger_record_paths)?,
            legacy_workflow_log_paths,
            legacy_trigger_record_paths,
            legacy_trigger_checkpoint_paths,
        })
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
        let summaries = self.list_committed_run_summaries()?;
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
        if let Some(sequence) = self.read_workflow_log_sequence_cursor(run_id)? {
            return Ok(sequence);
        }

        let canonical_dir = self
            .layout
            .workflow_log_entry_path(run_id, 1)
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| self.layout.run_dir(run_id).join(WORKFLOW_LOGS_DIR_NAME));

        create_dir_all_file_state(&canonical_dir)?;
        let entries = match fs::read_dir(&canonical_dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(1),
            Err(source) => {
                return Err(FileStateError::Io {
                    path: canonical_dir,
                    operation: "read workflow log directory",
                    source,
                });
            }
        };

        let mut max_sequence = 0_u64;
        for entry in entries {
            let entry = entry.map_err(|source| FileStateError::Io {
                path: canonical_dir.clone(),
                operation: "read workflow log directory entry",
                source,
            })?;
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            if let Some(sequence) = parse_numeric_file_stem_prefix(&path) {
                max_sequence = max_sequence.max(sequence);
            }
        }

        Ok(max_sequence.saturating_add(1))
    }

    fn read_workflow_log_sequence_cursor(
        &self,
        run_id: &str,
    ) -> Result<Option<u64>, FileStateError> {
        let cursor_path = self.layout.workflow_log_sequence_cursor_path(run_id);
        match fs::read_to_string(&cursor_path) {
            Ok(value) => Ok(value.trim().parse::<u64>().ok()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(source) => Err(FileStateError::Io {
                path: cursor_path,
                operation: "read workflow log sequence cursor",
                source,
            }),
        }
    }

    fn write_workflow_log_sequence_cursor(
        &self,
        run_id: &str,
        next_sequence: u64,
    ) -> Result<(), FileStateError> {
        let cursor_path = self.layout.workflow_log_sequence_cursor_path(run_id);
        let staged = staged_path(&cursor_path);
        atomic_write_text_file(
            &cursor_path,
            &staged,
            &next_sequence.to_string(),
            "write workflow log sequence cursor",
        )
    }

    fn workflow_log_roots(&self) -> Result<Vec<PathBuf>, FileStateError> {
        let mut roots = Vec::new();

        let run_dirs = match fs::read_dir(&self.layout.runs_dir) {
            Ok(run_dirs) => run_dirs,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                roots.sort();
                roots.dedup();
                return Ok(roots);
            }
            Err(source) => {
                return Err(FileStateError::Io {
                    path: self.layout.runs_dir.clone(),
                    operation: "read runs directory",
                    source,
                });
            }
        };
        for entry in run_dirs {
            let entry = entry.map_err(|source| FileStateError::Io {
                path: self.layout.runs_dir.clone(),
                operation: "read runs directory entry",
                source,
            })?;
            let run_dir = entry.path();
            if run_dir.is_dir() {
                roots.push(run_dir.join(WORKFLOW_LOGS_DIR_NAME));
            }
        }

        roots.sort();
        roots.dedup();
        Ok(roots)
    }

    fn trigger_record_roots(&self) -> Result<Vec<PathBuf>, FileStateError> {
        let mut roots = Vec::new();

        let trigger_dirs = match fs::read_dir(&self.layout.trigger_state_dir) {
            Ok(trigger_dirs) => trigger_dirs,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                roots.sort();
                roots.dedup();
                return Ok(roots);
            }
            Err(source) => {
                return Err(FileStateError::Io {
                    path: self.layout.trigger_state_dir.clone(),
                    operation: "read trigger state directory",
                    source,
                });
            }
        };
        for entry in trigger_dirs {
            let entry = entry.map_err(|source| FileStateError::Io {
                path: self.layout.trigger_state_dir.clone(),
                operation: "read trigger state directory entry",
                source,
            })?;
            let trigger_dir = entry.path();
            if trigger_dir.is_dir() {
                roots.push(trigger_dir.join(CANONICAL_TRIGGER_RECORDS_DIR_NAME));
            }
        }

        roots.sort();
        roots.dedup();
        Ok(roots)
    }

    fn trigger_state_dirs(&self) -> Result<Vec<PathBuf>, FileStateError> {
        let mut directories = Vec::new();

        let trigger_dirs = match fs::read_dir(&self.layout.trigger_state_dir) {
            Ok(trigger_dirs) => trigger_dirs,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(directories),
            Err(source) => {
                return Err(FileStateError::Io {
                    path: self.layout.trigger_state_dir.clone(),
                    operation: "read trigger state directory",
                    source,
                });
            }
        };

        for entry in trigger_dirs {
            let entry = entry.map_err(|source| FileStateError::Io {
                path: self.layout.trigger_state_dir.clone(),
                operation: "read trigger state directory entry",
                source,
            })?;
            let trigger_dir = entry.path();
            if trigger_dir.is_dir() {
                directories.push(trigger_dir);
            }
        }

        directories.sort();
        directories.dedup();
        Ok(directories)
    }

    fn collect_legacy_workflow_log_paths(&self) -> Result<Vec<PathBuf>, FileStateError> {
        let mut paths = Vec::new();
        collect_existing_json_files(&self.layout.workflow_logs_dir, &mut paths)?;
        paths.sort();
        Ok(paths)
    }

    fn collect_legacy_trigger_record_paths(&self) -> Result<Vec<PathBuf>, FileStateError> {
        let mut paths = Vec::new();
        collect_existing_json_files(&self.layout.trigger_records_dir, &mut paths)?;
        paths.sort();
        Ok(paths)
    }

    fn collect_legacy_trigger_checkpoint_paths(&self) -> Result<Vec<PathBuf>, FileStateError> {
        let mut paths = Vec::new();
        collect_existing_json_files(&self.layout.trigger_checkpoints_dir, &mut paths)?;
        paths.sort();
        Ok(paths)
    }

    fn detect_dual_checkpoints(
        &self,
        legacy_checkpoint_paths: &[PathBuf],
    ) -> Vec<MixedCheckpointConflict> {
        let mut conflicts = Vec::new();
        for legacy_path in legacy_checkpoint_paths {
            let Some(trigger_id) = legacy_path
                .file_stem()
                .and_then(|value| value.to_str())
                .map(str::to_owned)
            else {
                continue;
            };
            let canonical_path = self.layout.trigger_checkpoint_path(&trigger_id);
            if canonical_path.exists() {
                conflicts.push(MixedCheckpointConflict {
                    trigger_id,
                    canonical_path,
                    legacy_path: legacy_path.clone(),
                });
            }
        }
        conflicts.sort_by(|left, right| left.trigger_id.cmp(&right.trigger_id));
        conflicts
    }

    fn detect_duplicate_trigger_history(
        &self,
        legacy_record_paths: &[PathBuf],
    ) -> Result<Vec<DuplicateTriggerHistoryConflict>, FileStateError> {
        let mut canonical_records = Vec::new();
        for directory in self.trigger_record_roots()? {
            if directory == self.layout.trigger_records_dir {
                continue;
            }
            collect_existing_json_files(&directory, &mut canonical_records)?;
        }

        let mut canonical_by_identity = BTreeMap::<(String, String), PathBuf>::new();
        for path in canonical_records {
            let record = load_trigger_record_file(&path)?;
            canonical_by_identity
                .entry((record.trigger_id, record.event_id))
                .or_insert(path);
        }

        let mut conflicts = Vec::new();
        for legacy_path in legacy_record_paths {
            let legacy_record = load_trigger_record_file(legacy_path)?;
            let identity = (
                legacy_record.trigger_id.clone(),
                legacy_record.event_id.clone(),
            );
            if let Some(canonical_path) = canonical_by_identity.get(&identity) {
                conflicts.push(DuplicateTriggerHistoryConflict {
                    trigger_id: legacy_record.trigger_id,
                    event_id: legacy_record.event_id,
                    canonical_path: canonical_path.clone(),
                    legacy_path: legacy_path.clone(),
                });
            }
        }

        conflicts.sort_by(|left, right| {
            left.trigger_id
                .cmp(&right.trigger_id)
                .then(left.event_id.cmp(&right.event_id))
        });
        Ok(conflicts)
    }
}

pub fn inspect_legacy_state_usage(
    layout: &StateLayout,
) -> Result<LegacyStateInspection, FileStateError> {
    FileBackedStateStore::new(layout.clone()).inspect_legacy_usage()
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

    pub fn rebuild_trigger_snapshot_coordination(
        &mut self,
        snapshots: &[TriggerSnapshotRecord],
        now_ms: i64,
    ) -> Result<(), CoordinationError> {
        let transaction =
            self.connection
                .transaction()
                .map_err(|source| CoordinationError::Sqlite {
                    path: self.db_path.clone(),
                    operation: "start trigger snapshot coordination rebuild transaction",
                    source,
                })?;

        transaction
            .execute("DELETE FROM coordination_tokens", [])
            .map_err(|source| CoordinationError::Sqlite {
                path: self.db_path.clone(),
                operation: "clear coordination tokens before snapshot rebuild",
                source,
            })?;

        for snapshot in snapshots {
            for token in &snapshot.dedup_tokens {
                upsert_coordination_token_if_unexpired(
                    &transaction,
                    &self.db_path,
                    CoordinationTokenKind::Dedup,
                    Some(token.key.as_str()),
                    Some(token.expires_at_ms),
                    now_ms,
                )?;
            }
            for token in &snapshot.cooldown_tokens {
                upsert_coordination_token_if_unexpired(
                    &transaction,
                    &self.db_path,
                    CoordinationTokenKind::Cooldown,
                    Some(token.key.as_str()),
                    Some(token.expires_at_ms),
                    now_ms,
                )?;
            }
        }

        transaction
            .commit()
            .map_err(|source| CoordinationError::Sqlite {
                path: self.db_path.clone(),
                operation: "commit trigger snapshot coordination rebuild transaction",
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

fn load_trigger_snapshot_file(path: &Path) -> Result<TriggerSnapshotRecord, FileStateError> {
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

fn atomic_write_text_file(
    path: &Path,
    staged_path: &Path,
    value: &str,
    _operation: &'static str,
) -> Result<(), FileStateError> {
    if let Some(parent) = path.parent() {
        create_dir_all_file_state(parent)?;
    }

    let mut file = File::create(staged_path).map_err(|source| FileStateError::Io {
        path: staged_path.to_path_buf(),
        operation: "create staged file",
        source,
    })?;
    file.write_all(value.as_bytes())
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

fn recover_append_only_entries_if_exists<T>(
    root: &Path,
    loader: fn(&Path) -> Result<T, FileStateError>,
) -> Result<FileArtifactRecoveryReport, FileStateError> {
    match fs::metadata(root) {
        Ok(metadata) if metadata.is_dir() => recover_append_only_entries(root, loader),
        Ok(_) => Ok(FileArtifactRecoveryReport::default()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(FileArtifactRecoveryReport::default())
        }
        Err(source) => Err(FileStateError::Io {
            path: root.to_path_buf(),
            operation: "inspect metadata",
            source,
        }),
    }
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

fn parse_numeric_file_stem_prefix(path: &Path) -> Option<u64> {
    let file_name = path.file_name()?.to_str()?;
    let prefix = file_name.split_once('.')?.0.split_once('-').map_or_else(
        || file_name.split_once('.').map(|(value, _)| value),
        |(value, _)| Some(value),
    )?;
    prefix.parse::<u64>().ok()
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

fn retain_unexpired_tokens(tokens: &mut Vec<TriggerTokenSnapshot>, now_ms: i64) {
    tokens.retain(|token| token.expires_at_ms > now_ms);
}

fn upsert_token_snapshot(
    tokens: &mut Vec<TriggerTokenSnapshot>,
    key: Option<&str>,
    expires_at_ms: Option<i64>,
    now_ms: i64,
) {
    let (Some(key), Some(expires_at_ms)) = (key, expires_at_ms) else {
        return;
    };
    if expires_at_ms <= now_ms {
        return;
    }

    if let Some(existing) = tokens.iter_mut().find(|token| token.key == key) {
        existing.expires_at_ms = existing.expires_at_ms.max(expires_at_ms);
        return;
    }

    tokens.push(TriggerTokenSnapshot {
        key: key.to_owned(),
        expires_at_ms,
    });
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

fn canonical_trigger_record_file_name(sequence: u64, event_id: &str) -> String {
    format!("{sequence:020}-{}.json", sanitize_path_component(event_id))
}
