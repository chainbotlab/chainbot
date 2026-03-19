use crate::errors::ContractError;
use crate::trigger::TriggerDefinition;

pub fn builtin_trigger_kind(definition: &TriggerDefinition) -> Result<&str, ContractError> {
    definition
        .builtin_subtype()?
        .ok_or_else(|| ContractError::InvalidTriggerDefinitionField {
            trigger_id: definition.trigger_id.clone(),
            field: "trigger.kind",
            detail: "expected builtin trigger subtype".to_owned(),
        })
}
