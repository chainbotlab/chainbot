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

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::domain::state::{
    accepted_trigger_key, RunRecordSummary, RunStatus, TriggerCheckpointRecord, TriggerEventRecord,
    TriggerSnapshotRecord, WorkflowRuntimeLogEntry,
};
use crate::errors::ContractError;
use crate::infrastructure::config::RootLayout;

pub const COORDINATION_DB_FILE_NAME: &str = "coordination.sqlite3";
pub const RUNS_DIR_NAME: &str = "runs";
pub const WORKFLOW_LOGS_DIR_NAME: &str = "workflow-logs";
pub const TRIGGER_RECORDS_DIR_NAME: &str = "trigger-records";
pub const TRIGGER_CHECKPOINTS_DIR_NAME: &str = "trigger-checkpoints";
pub const CANONICAL_TRIGGERS_DIR_NAME: &str = "triggers";
pub const CANONICAL_TRIGGER_RECORDS_DIR_NAME: &str = "records";
pub const CANONICAL_TRIGGER_CHECKPOINT_FILE_NAME: &str = "checkpoint.json";
pub const CANONICAL_TRIGGER_SNAPSHOT_FILE_NAME: &str = "snapshot.json";

const RUN_SUMMARY_FILE_NAME: &str = "summary.json";
const STAGED_FILE_SUFFIX: &str = ".next";
const WORKFLOW_LOG_SEQUENCE_CURSOR_FILE_NAME: &str = "workflow-log-sequence.cursor";

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

pub(crate) fn sanitize_path_component(value: &str) -> String {
    crate::domain::state::sanitize_path_component(value)
}

fn canonical_trigger_record_file_name(sequence: u64, event_id: &str) -> String {
    format!("{sequence:020}-{}.json", sanitize_path_component(event_id))
}
