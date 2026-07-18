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

impl crate::domain::trigger::acceptance::TriggerAcceptanceStore for RuntimeStateStore {
    fn load_trigger_acceptance_state(
        &mut self,
        trigger_id: &str,
    ) -> Result<
        crate::domain::state::TriggerSnapshotRecord,
        crate::domain::trigger::TriggerPlaneError,
    > {
        RuntimeStateStore::load_trigger_acceptance_state(self, trigger_id).map_err(Into::into)
    }

    fn list_pending_trigger_events(
        &mut self,
        trigger_id: &str,
        limit: i64,
    ) -> Result<
        Vec<crate::domain::state::StagedTriggerEventRecord>,
        crate::domain::trigger::TriggerPlaneError,
    > {
        self.list_pending_staged_trigger_event_records(trigger_id, limit)
            .map_err(Into::into)
    }

    fn accept_trigger_event(
        &mut self,
        command: crate::domain::trigger::TriggerAcceptanceCommand,
    ) -> Result<
        crate::domain::trigger::TriggerAcceptanceOutcome,
        crate::domain::trigger::TriggerPlaneError,
    > {
        RuntimeStateStore::accept_trigger_event(self, command).map_err(Into::into)
    }
}
