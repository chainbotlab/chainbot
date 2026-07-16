//! [INPUT]
//! Trigger definitions, persisted trigger-state records, staged events, and emission mapping helpers.
//!
//! [OUTPUT]
//! Normalizes trigger emissions into accepted run requests while updating domain checkpoint and snapshot records.
//!
//! [ROLE]
//! Owns pure domain acceptance logic for trigger deduplication, cooldowns, and staged-event consumption.

// Trigger Domain Acceptance vs App Runtime Supervision Boundary
//
// domain::trigger::acceptance
//   TriggerPlane::open_domain_with_store   — opens domain with a RuntimeStateStore
//   TriggerPlane::collect_run_requests     — collects from staged records + builtin emissions
//   TriggerPlane::normalize_emission        — dedup, cooldown, record write, snapshot update
//   Output: Vec<TriggerRunRequest>
//
// app::runtime::external_triggers
//   ProcessListener       — spawns plugin process, runs stdin/stdout protocol loop
//   WasmtimeRuntime      — WASM plugin host lifecycle
//   ExternalEmissionCollector — collects external plugin emissions into TriggerPlane
//
// The boundary: acceptance runs pure domain logic (dedup, cooldown, state persistence).
// It does NOT spawn processes, manage threads, or handle I/O. Those concerns live in
// app::runtime::external_triggers.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt::{Display, Formatter};

use crate::domain::state::{
    accepted_trigger_key, sanitize_path_component, StagedTriggerEventRecord,
    TriggerCheckpointRecord, TriggerEventRecord, TriggerSnapshotRecord,
};
use crate::errors::ContractError;

use super::contract::TriggerDefinition;
use super::emission::{map_trigger_payload, trigger_emission_from_staged_record, TriggerEmission};

const STAGED_TRIGGER_EVENT_BATCH_LIMIT: i64 = 256;

#[derive(Debug, Clone, PartialEq)]
pub struct TriggerAcceptanceCommand {
    pub candidate_record: TriggerEventRecord,
    pub expected_snapshot_sequence: u64,
    pub staged_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TriggerAcceptanceOutcome {
    Accepted {
        request: TriggerRunRequest,
        record_ref: String,
    },
    Duplicate,
    DedupSuppressed,
    CooldownSuppressed,
    Conflict,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TriggerRunRequest {
    pub run_id: String,
    pub workflow_id: String,
    pub trigger_id: String,
    pub event_id: String,
    pub source: String,
    pub accepted_at_ms: i64,
    pub payload: serde_json::Value,
    pub trigger_record_ref: String,
}

pub struct TriggerPlane {
    definitions: Vec<TriggerDefinition>,
    builtin_events: BTreeMap<String, Vec<TriggerEmission>>,
    state_store: Box<dyn TriggerStateStore>,
    accepted_sequence: u64,
    accepted_event_keys: BTreeSet<String>,
    external_emission_collector: Option<Box<ExternalEmissionCollector>>,
}

impl std::fmt::Debug for TriggerPlane {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TriggerPlane")
            .field("definitions", &self.definitions)
            .field("builtin_events", &self.builtin_events)
            .field("state_store", &self.state_store)
            .field("accepted_sequence", &self.accepted_sequence)
            .field("accepted_event_keys", &self.accepted_event_keys)
            .finish_non_exhaustive()
    }
}

type ExternalEmissionCollector = dyn FnMut(
    &mut dyn TriggerStateStore,
    &TriggerDefinition,
    &mut dyn FnMut() -> Result<(), TriggerPlaneError>,
) -> Result<Vec<TriggerEmission>, TriggerPlaneError>;

pub trait TriggerStateStore: std::fmt::Debug {
    fn list_trigger_snapshots_for_acceptance(
        &mut self,
    ) -> Result<Vec<TriggerSnapshotRecord>, TriggerPlaneError>;
    fn load_trigger_records_after_sequence_for_acceptance(
        &mut self,
        trigger_id: &str,
        last_sequence: u64,
    ) -> Result<Vec<TriggerEventRecord>, TriggerPlaneError>;
    fn write_trigger_snapshot_for_acceptance(
        &mut self,
        snapshot: &TriggerSnapshotRecord,
    ) -> Result<(), TriggerPlaneError>;
    fn list_pending_staged_trigger_event_records_for_acceptance(
        &mut self,
        trigger_id: &str,
        limit: i64,
    ) -> Result<Vec<StagedTriggerEventRecord>, TriggerPlaneError>;
    fn mark_staged_trigger_event_accepted_for_acceptance(
        &mut self,
        staging_id: &str,
        accepted_at_ms: i64,
    ) -> Result<(), TriggerPlaneError>;
    fn append_staged_trigger_event_record_for_acceptance(
        &mut self,
        record: &StagedTriggerEventRecord,
    ) -> Result<(), TriggerPlaneError>;
    fn dedup_is_ready_for_acceptance(
        &mut self,
        key: &str,
        now_ms: i64,
    ) -> Result<bool, TriggerPlaneError>;
    fn cooldown_is_ready_for_acceptance(
        &mut self,
        key: &str,
        now_ms: i64,
    ) -> Result<bool, TriggerPlaneError>;
    fn write_trigger_record_for_acceptance(
        &mut self,
        record: &TriggerEventRecord,
    ) -> Result<String, TriggerPlaneError>;
    fn read_trigger_snapshot_for_acceptance(
        &mut self,
        trigger_id: &str,
    ) -> Result<Option<TriggerSnapshotRecord>, TriggerPlaneError>;
    fn read_trigger_checkpoint_for_acceptance(
        &mut self,
        trigger_id: &str,
    ) -> Result<Option<TriggerCheckpointRecord>, TriggerPlaneError>;
    fn write_trigger_checkpoint_for_acceptance(
        &mut self,
        checkpoint: &TriggerCheckpointRecord,
    ) -> Result<(), TriggerPlaneError>;
    fn accept_trigger_event(
        &mut self,
        command: TriggerAcceptanceCommand,
    ) -> Result<TriggerAcceptanceOutcome, TriggerPlaneError> {
        let record = command.candidate_record;
        let record_ref = self.write_trigger_record_for_acceptance(&record)?;
        let mut snapshot = self
            .read_trigger_snapshot_for_acceptance(&record.trigger_id)?
            .unwrap_or_else(|| TriggerSnapshotRecord::new(record.trigger_id.clone()));
        if snapshot.last_sequence != command.expected_snapshot_sequence {
            return Ok(TriggerAcceptanceOutcome::Conflict);
        }
        snapshot.apply_record(&record);
        self.write_trigger_snapshot_for_acceptance(&snapshot)?;
        if let Some(checkpoint) = record.checkpoint.clone() {
            self.write_trigger_checkpoint_for_acceptance(&TriggerCheckpointRecord {
                schema_version: String::from("1.0.0"),
                trigger_id: record.trigger_id.clone(),
                checkpoint,
                acked_at_ms: record.accepted_at_ms,
            })?;
        }
        if let Some(staged_id) = command.staged_id {
            self.mark_staged_trigger_event_accepted_for_acceptance(&staged_id, record.accepted_at_ms)?;
        }
        Ok(TriggerAcceptanceOutcome::Accepted {
            request: TriggerRunRequest {
                run_id: record.run_id,
                workflow_id: record.workflow_id,
                trigger_id: record.trigger_id,
                event_id: record.event_id,
                source: record.source,
                accepted_at_ms: record.accepted_at_ms,
                payload: record.payload,
                trigger_record_ref: record_ref.clone(),
            },
            record_ref,
        })
    }
}

#[derive(Debug)]
pub enum TriggerPlaneError {
    Contract(ContractError),
    RuntimeState(String),
}

impl TriggerPlane {
    pub(crate) fn open_domain_with_store(
        mut state_store: impl TriggerStateStore + 'static,
        definitions: Vec<TriggerDefinition>,
        builtin_events: BTreeMap<String, Vec<TriggerEmission>>,
        external_emission_collector: Option<Box<ExternalEmissionCollector>>,
    ) -> Result<Self, TriggerPlaneError> {
        let mut validated_definitions = Vec::with_capacity(definitions.len());
        let mut definition_ids = BTreeSet::new();
        for definition in definitions {
            definition.validate()?;
            if !definition_ids.insert(definition.trigger_id.clone()) {
                return Err(TriggerPlaneError::Contract(
                    ContractError::DuplicateTriggerId {
                        trigger_id: definition.trigger_id,
                    },
                ));
            }
            validated_definitions.push(definition);
        }

        let mut snapshots_by_trigger = state_store
            .list_trigger_snapshots_for_acceptance()?
            .into_iter()
            .map(|snapshot| (snapshot.trigger_id.clone(), snapshot))
            .collect::<BTreeMap<_, _>>();

        let mut accepted_event_keys = BTreeSet::new();
        let mut accepted_sequence = 0_u64;

        for definition in &validated_definitions {
            let mut snapshot = snapshots_by_trigger
                .remove(&definition.trigger_id)
                .unwrap_or_else(|| TriggerSnapshotRecord::new(definition.trigger_id.clone()));
            let delta_records = state_store.load_trigger_records_after_sequence_for_acceptance(
                &definition.trigger_id,
                snapshot.last_sequence,
            )?;
            if !delta_records.is_empty() {
                for record in &delta_records {
                    snapshot.apply_record(record);
                }
                state_store.write_trigger_snapshot_for_acceptance(&snapshot)?;
            }

            accepted_sequence = accepted_sequence.max(snapshot.last_sequence);
            accepted_event_keys.extend(
                snapshot
                    .accepted_event_ids
                    .iter()
                    .map(|event_id| accepted_trigger_key(&definition.trigger_id, event_id)),
            );
        }

        Ok(Self {
            definitions: validated_definitions,
            builtin_events,
            state_store: Box::new(state_store),
            accepted_sequence,
            accepted_event_keys,
            external_emission_collector,
        })
    }

    pub fn collect_run_requests(
        &mut self,
        accepted_at_ms: i64,
    ) -> Result<Vec<TriggerRunRequest>, TriggerPlaneError> {
        self.collect_run_requests_with_progress(accepted_at_ms, &mut || Ok(()))
    }

    pub fn collect_run_requests_with_progress<F>(
        &mut self,
        accepted_at_ms: i64,
        on_progress: &mut F,
    ) -> Result<Vec<TriggerRunRequest>, TriggerPlaneError>
    where
        F: FnMut() -> Result<(), TriggerPlaneError>,
    {
        let mut run_requests = Vec::new();

        let definitions = self.definitions.clone();
        for definition in definitions {
            on_progress()?;
            if !definition.enabled {
                continue;
            }
            let mut emissions = self
                .builtin_events
                .remove(&definition.trigger_id)
                .unwrap_or_default();
            let mut staged_requests =
                self.drain_staged_records_for_definition(&definition, accepted_at_ms, on_progress)?;
            run_requests.append(&mut staged_requests);
            if definition.kind()? == crate::domain::trigger::TriggerKind::ExternalPlugin {
                if let Some(collector) = self.external_emission_collector.as_mut() {
                    let mut progress = || on_progress();
                    let mut external =
                        collector(self.state_store.as_mut(), &definition, &mut progress)?;
                    emissions.append(&mut external);
                }
            }

            let mut post_external_staged_requests =
                self.drain_staged_records_for_definition(&definition, accepted_at_ms, on_progress)?;
            run_requests.append(&mut post_external_staged_requests);

            for emission in emissions {
                let request = self.normalize_emission(&definition, emission, accepted_at_ms, None)?;
                if let Some(request) = request {
                    run_requests.push(request);
                }
            }
        }

        Ok(run_requests)
    }

    fn drain_staged_records_for_definition<F>(
        &mut self,
        definition: &TriggerDefinition,
        accepted_at_ms: i64,
        on_progress: &mut F,
    ) -> Result<Vec<TriggerRunRequest>, TriggerPlaneError>
    where
        F: FnMut() -> Result<(), TriggerPlaneError>,
    {
        let staged_records = self
            .state_store
            .list_pending_staged_trigger_event_records_for_acceptance(
                &definition.trigger_id,
                STAGED_TRIGGER_EVENT_BATCH_LIMIT,
            )?;
        let mut requests = Vec::new();
        for staged_record in staged_records {
            on_progress()?;
            let request = self.normalize_emission(
                definition,
                trigger_emission_from_staged_record(&staged_record),
                accepted_at_ms,
                Some(staged_record.staging_id),
            )?;
            if let Some(request) = request {
                requests.push(request);
            }
        }
        Ok(requests)
    }

    fn normalize_emission(
        &mut self,
        definition: &TriggerDefinition,
        emission: TriggerEmission,
        accepted_at_ms: i64,
        staged_id: Option<String>,
    ) -> Result<Option<TriggerRunRequest>, TriggerPlaneError> {
        let accepted_event_key = accepted_trigger_key(&definition.trigger_id, &emission.event_id);

        if emission.event_id.trim().is_empty() {
            return Err(TriggerPlaneError::Contract(
                ContractError::InvalidTriggerEmission {
                    trigger_id: definition.trigger_id.clone(),
                    detail: "event_id cannot be empty".to_owned(),
                },
            ));
        }

        let dedup_key = emission
            .dedup_window_ms
            .filter(|value| *value > 0)
            .map(|_| {
                emission
                    .dedup_key
                    .clone()
                    .unwrap_or_else(|| format!("{}:{}", definition.trigger_id, emission.event_id))
            });
        let dedup_expires_at_ms = emission
            .dedup_window_ms
            .filter(|value| *value > 0)
            .map(|window_ms| accepted_at_ms.saturating_add(window_ms));
        let cooldown_key = emission.cooldown_ms.filter(|value| *value > 0).map(|_| {
            emission
                .cooldown_key
                .clone()
                .unwrap_or_else(|| definition.trigger_id.clone())
        });
        let cooldown_expires_at_ms = emission
            .cooldown_ms
            .filter(|value| *value > 0)
            .map(|cooldown_ms| accepted_at_ms.saturating_add(cooldown_ms));
        self.accepted_sequence = self.accepted_sequence.saturating_add(1);
        let run_id = format!(
            "run-{}-{}-{}-{}-{}-{:020}",
            sanitize_path_component(&definition.workflow_id),
            sanitize_path_component(&definition.trigger_id),
            sanitize_path_component(&emission.event_id),
            emission.occurred_at_ms,
            accepted_at_ms,
            self.accepted_sequence
        );

        let source = emission
            .source
            .clone()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| definition.source.clone());
        if source.trim().is_empty() {
            return Err(TriggerPlaneError::Contract(
                ContractError::InvalidTriggerEmission {
                    trigger_id: definition.trigger_id.clone(),
                    detail: "source cannot be empty".to_owned(),
                },
            ));
        }

        let payload = map_trigger_payload(definition, &emission.payload);
        let checkpoint = emission.checkpoint.clone();
        let trigger_record = TriggerEventRecord {
            schema_version: String::from("1.0.0"),
            run_id: run_id.clone(),
            sequence: self.accepted_sequence,
            trigger_id: definition.trigger_id.clone(),
            workflow_id: definition.workflow_id.clone(),
            event_id: emission.event_id.clone(),
            checkpoint: checkpoint.clone(),
            source: source.clone(),
            accepted_at_ms,
            payload: payload.clone(),
            dedup_key,
            dedup_expires_at_ms,
            cooldown_key,
            cooldown_expires_at_ms,
        };

        let expected_snapshot_sequence = self
            .state_store
            .read_trigger_snapshot_for_acceptance(&definition.trigger_id)?
            .map_or(0, |snapshot| snapshot.last_sequence);
        let outcome = self.state_store.accept_trigger_event(TriggerAcceptanceCommand {
            candidate_record: trigger_record,
            expected_snapshot_sequence,
            staged_id,
        })?;
        match outcome {
            TriggerAcceptanceOutcome::Accepted { request, .. } => {
                self.accepted_event_keys.insert(accepted_event_key);
                Ok(Some(request))
            }
            TriggerAcceptanceOutcome::Duplicate
            | TriggerAcceptanceOutcome::DedupSuppressed
            | TriggerAcceptanceOutcome::CooldownSuppressed
            | TriggerAcceptanceOutcome::Conflict => Ok(None),
        }
    }
}

impl Display for TriggerPlaneError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Contract(source) => write!(f, "trigger contract error: {source}"),
            Self::RuntimeState(source) => write!(f, "trigger runtime-state error: {source}"),
        }
    }
}

impl Error for TriggerPlaneError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Contract(source) => Some(source),
            Self::RuntimeState(_) => None,
        }
    }
}

impl From<ContractError> for TriggerPlaneError {
    fn from(value: ContractError) -> Self {
        Self::Contract(value)
    }
}

impl TriggerPlaneError {
    pub fn runtime_state(detail: impl Into<String>) -> Self {
        Self::RuntimeState(detail.into())
    }
}
