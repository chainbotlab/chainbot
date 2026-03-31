//! [INPUT]
//! Runtime-state persistence backends, coordination adapters, and backend dispatch implementations.
//!
//! [OUTPUT]
//! Exposes infrastructure-owned runtime-state store implementations and file/SQLite persistence adapters.
//!
//! [ROLE]
//! Owns storage-specific runtime-state implementations behind domain model contracts.

pub mod db_store;
pub mod file_store;
pub mod sqlite_coordination;

pub use db_store::{
    RuntimeDaemonStatus, RuntimeHistoryArchiveCounts, RuntimeHistoryArchiveStats,
    RuntimeStateError, RuntimeStateStore,
};
pub use file_store::{
    inspect_legacy_state_usage, DuplicateTriggerHistoryConflict, FileArtifactRecoveryReport,
    FileBackedStateStore, FileStateError, LegacyStateInspection, MixedCheckpointConflict,
    RunSummaryRecoveryReport, RuntimeStateRecoveryReport, StateLayout,
};
pub use sqlite_coordination::{CoordinationError, CoordinationStore};

pub(crate) use file_store::sanitize_path_component;

impl From<RuntimeStateError> for crate::domain::trigger::TriggerPlaneError {
    fn from(value: RuntimeStateError) -> Self {
        Self::runtime_state(value.to_string())
    }
}

impl crate::domain::trigger::acceptance::TriggerStateStore for RuntimeStateStore {
    fn list_trigger_snapshots_for_acceptance(
        &mut self,
    ) -> Result<
        Vec<crate::domain::state::TriggerSnapshotRecord>,
        crate::domain::trigger::TriggerPlaneError,
    > {
        self.list_trigger_snapshots().map_err(|error| {
            crate::domain::trigger::TriggerPlaneError::runtime_state(error.to_string())
        })
    }

    fn load_trigger_records_after_sequence_for_acceptance(
        &mut self,
        trigger_id: &str,
        last_sequence: u64,
    ) -> Result<
        Vec<crate::domain::state::TriggerEventRecord>,
        crate::domain::trigger::TriggerPlaneError,
    > {
        self.load_trigger_records_after_sequence(trigger_id, last_sequence)
            .map_err(|error| {
                crate::domain::trigger::TriggerPlaneError::runtime_state(error.to_string())
            })
    }

    fn write_trigger_snapshot_for_acceptance(
        &mut self,
        snapshot: &crate::domain::state::TriggerSnapshotRecord,
    ) -> Result<(), crate::domain::trigger::TriggerPlaneError> {
        self.write_trigger_snapshot(snapshot).map_err(|error| {
            crate::domain::trigger::TriggerPlaneError::runtime_state(error.to_string())
        })
    }

    fn list_pending_staged_trigger_event_records_for_acceptance(
        &mut self,
        trigger_id: &str,
        limit: i64,
    ) -> Result<
        Vec<crate::domain::state::StagedTriggerEventRecord>,
        crate::domain::trigger::TriggerPlaneError,
    > {
        self.list_pending_staged_trigger_event_records(trigger_id, limit)
            .map_err(|error| {
                crate::domain::trigger::TriggerPlaneError::runtime_state(error.to_string())
            })
    }

    fn mark_staged_trigger_event_accepted_for_acceptance(
        &mut self,
        staging_id: &str,
        accepted_at_ms: i64,
    ) -> Result<(), crate::domain::trigger::TriggerPlaneError> {
        self.mark_staged_trigger_event_accepted(staging_id, accepted_at_ms)
            .map_err(|error| {
                crate::domain::trigger::TriggerPlaneError::runtime_state(error.to_string())
            })
    }

    fn append_staged_trigger_event_record_for_acceptance(
        &mut self,
        record: &crate::domain::state::StagedTriggerEventRecord,
    ) -> Result<(), crate::domain::trigger::TriggerPlaneError> {
        self.append_staged_trigger_event_record(record)
            .map(|_| ())
            .map_err(|error| {
                crate::domain::trigger::TriggerPlaneError::runtime_state(error.to_string())
            })
    }

    fn dedup_is_ready_for_acceptance(
        &mut self,
        key: &str,
        now_ms: i64,
    ) -> Result<bool, crate::domain::trigger::TriggerPlaneError> {
        self.dedup_is_ready(key, now_ms).map_err(|error| {
            crate::domain::trigger::TriggerPlaneError::runtime_state(error.to_string())
        })
    }

    fn cooldown_is_ready_for_acceptance(
        &mut self,
        key: &str,
        now_ms: i64,
    ) -> Result<bool, crate::domain::trigger::TriggerPlaneError> {
        self.cooldown_is_ready(key, now_ms).map_err(|error| {
            crate::domain::trigger::TriggerPlaneError::runtime_state(error.to_string())
        })
    }

    fn write_trigger_record_for_acceptance(
        &mut self,
        record: &crate::domain::state::TriggerEventRecord,
    ) -> Result<String, crate::domain::trigger::TriggerPlaneError> {
        self.write_trigger_record(record).map_err(|error| {
            crate::domain::trigger::TriggerPlaneError::runtime_state(error.to_string())
        })
    }

    fn read_trigger_snapshot_for_acceptance(
        &mut self,
        trigger_id: &str,
    ) -> Result<
        Option<crate::domain::state::TriggerSnapshotRecord>,
        crate::domain::trigger::TriggerPlaneError,
    > {
        self.read_trigger_snapshot(trigger_id).map_err(|error| {
            crate::domain::trigger::TriggerPlaneError::runtime_state(error.to_string())
        })
    }

    fn read_trigger_checkpoint_for_acceptance(
        &mut self,
        trigger_id: &str,
    ) -> Result<
        Option<crate::domain::state::TriggerCheckpointRecord>,
        crate::domain::trigger::TriggerPlaneError,
    > {
        self.read_trigger_checkpoint(trigger_id).map_err(|error| {
            crate::domain::trigger::TriggerPlaneError::runtime_state(error.to_string())
        })
    }

    fn write_trigger_checkpoint_for_acceptance(
        &mut self,
        checkpoint: &crate::domain::state::TriggerCheckpointRecord,
    ) -> Result<(), crate::domain::trigger::TriggerPlaneError> {
        self.write_trigger_checkpoint(checkpoint).map_err(|error| {
            crate::domain::trigger::TriggerPlaneError::runtime_state(error.to_string())
        })
    }
}
