//! [INPUT]
//! Builtin trigger evaluation context, trigger definitions, emission contracts, and param decoding helpers.
//!
//! [OUTPUT]
//! Defines the builtin trigger handler trait and shared helpers for validating or emitting builtin trigger events.
//!
//! [ROLE]
//! Provides the canonical contract surface for builtin trigger emitters.

use serde::de::DeserializeOwned;

use crate::builtins::triggers::context::BuiltinTriggerContext;
use crate::domain::trigger::{TriggerDefinition, TriggerEmission};
use crate::errors::ContractError;

pub trait BuiltinTriggerHandler: Send + Sync {
    fn kind(&self) -> &str;

    fn validate(&self, _definition: &TriggerDefinition) -> Result<(), ContractError> {
        Ok(())
    }

    fn emit(
        &self,
        context: &BuiltinTriggerContext,
        definition: &TriggerDefinition,
    ) -> Result<Vec<TriggerEmission>, ContractError>;
}

pub(crate) fn decode_builtin_trigger_params<T>(
    definition: &TriggerDefinition,
) -> Result<T, ContractError>
where
    T: DeserializeOwned,
{
    serde_json::from_value(serde_json::to_value(&definition.params).map_err(|source| {
        ContractError::InvalidTriggerDefinitionField {
            trigger_id: definition.trigger_id.clone(),
            field: "trigger.params",
            detail: format!("failed to serialize trigger params: {source}"),
        }
    })?)
    .map_err(|source| ContractError::InvalidTriggerDefinitionField {
        trigger_id: definition.trigger_id.clone(),
        field: "trigger.params",
        detail: source.to_string(),
    })
}
