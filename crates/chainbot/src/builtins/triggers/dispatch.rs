//! [INPUT]
//! Trigger definitions from the trigger plane plus contract error semantics.
//!
//! [OUTPUT]
//! Resolves builtin trigger subtype names or returns definition errors when the trigger is not a builtin source.
//!
//! [ROLE]
//! Bridges trigger contracts into builtin trigger registry keys.

use crate::domain::trigger::TriggerDefinition;
use crate::errors::ContractError;

pub fn builtin_trigger_kind(definition: &TriggerDefinition) -> Result<&str, ContractError> {
    definition
        .builtin_subtype()?
        .ok_or_else(|| ContractError::InvalidTriggerDefinitionField {
            trigger_id: definition.trigger_id.clone(),
            field: "trigger.kind",
            detail: "expected builtin trigger subtype".to_owned(),
        })
}
