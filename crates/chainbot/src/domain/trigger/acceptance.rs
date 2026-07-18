//! [INPUT]
//! Trigger definitions, persisted trigger-state records, staged events, and emission mapping helpers.
//!
//! [OUTPUT]
//! Normalizes trigger emissions into accepted run requests while updating domain checkpoint and snapshot records.
//!
//! [ROLE]
//! Owns pure domain acceptance logic for trigger deduplication, cooldowns, and staged-event consumption.

// External trigger hosts durably stage events before this module evaluates them.
// Process lifecycle, transport, and acknowledgement remain in app::runtime::external_triggers.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt::{Display, Formatter};

use crate::domain::state::{
    sanitize_path_component, StagedTriggerEventRecord, TriggerEventRecord, TriggerSnapshotRecord,
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
    state_store: Box<dyn TriggerAcceptanceStore>,
    accepted_sequence: u64,
}

impl std::fmt::Debug for TriggerPlane {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TriggerPlane")
            .field("definitions", &self.definitions)
            .field("builtin_events", &self.builtin_events)
            .field("state_store", &self.state_store)
            .field("accepted_sequence", &self.accepted_sequence)
            .finish_non_exhaustive()
    }
}

pub trait TriggerAcceptanceStore: std::fmt::Debug {
    fn load_trigger_acceptance_state(
        &mut self,
        trigger_id: &str,
    ) -> Result<TriggerSnapshotRecord, TriggerPlaneError>;
    fn list_pending_trigger_events(
        &mut self,
        trigger_id: &str,
        limit: i64,
    ) -> Result<Vec<StagedTriggerEventRecord>, TriggerPlaneError>;
    fn accept_trigger_event(
        &mut self,
        command: TriggerAcceptanceCommand,
    ) -> Result<TriggerAcceptanceOutcome, TriggerPlaneError>;
}

#[derive(Debug)]
pub enum TriggerPlaneError {
    Contract(ContractError),
    RuntimeState(String),
}

impl TriggerPlane {
    pub(crate) fn open_domain_with_store(
        mut state_store: impl TriggerAcceptanceStore + 'static,
        definitions: Vec<TriggerDefinition>,
        builtin_events: BTreeMap<String, Vec<TriggerEmission>>,
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

        let mut accepted_sequence = 0_u64;
        for definition in &validated_definitions {
            let snapshot = state_store.load_trigger_acceptance_state(&definition.trigger_id)?;
            accepted_sequence = accepted_sequence.max(snapshot.last_sequence);
        }

        Ok(Self {
            definitions: validated_definitions,
            builtin_events,
            state_store: Box::new(state_store),
            accepted_sequence,
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
            let emissions = self
                .builtin_events
                .remove(&definition.trigger_id)
                .unwrap_or_default();
            let mut staged_requests =
                self.drain_staged_records_for_definition(&definition, accepted_at_ms, on_progress)?;
            run_requests.append(&mut staged_requests);

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
        let staged_records = self.state_store.list_pending_trigger_events(
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
            .load_trigger_acceptance_state(&definition.trigger_id)?
            .last_sequence;
        let outcome = self.state_store.accept_trigger_event(TriggerAcceptanceCommand {
            candidate_record: trigger_record,
            expected_snapshot_sequence,
            staged_id,
        })?;
        match outcome {
            TriggerAcceptanceOutcome::Accepted { request, .. } => Ok(Some(request)),
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
