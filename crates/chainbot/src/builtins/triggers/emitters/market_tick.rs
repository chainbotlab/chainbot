use serde::Deserialize;

use crate::builtins::triggers::context::BuiltinTriggerContext;
use crate::builtins::triggers::contract::{decode_builtin_trigger_params, BuiltinTriggerHandler};
use crate::errors::ContractError;
use crate::trigger::{TriggerDefinition, TriggerEmission};

pub(crate) const BUILTIN_TRIGGER_MARKET_TICK_KIND: &str = "market_tick";

#[derive(Debug, Clone, Copy)]
pub struct MarketTickTriggerHandler;

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct MarketTickTriggerParams {
    symbol: String,
}

impl Default for MarketTickTriggerParams {
    fn default() -> Self {
        Self {
            symbol: String::from("BTCUSDT"),
        }
    }
}

impl BuiltinTriggerHandler for MarketTickTriggerHandler {
    fn kind(&self) -> &str {
        BUILTIN_TRIGGER_MARKET_TICK_KIND
    }

    fn validate(&self, definition: &TriggerDefinition) -> Result<(), ContractError> {
        let _: MarketTickTriggerParams = decode_builtin_trigger_params(definition)?;
        Ok(())
    }

    fn emit(
        &self,
        context: &BuiltinTriggerContext,
        definition: &TriggerDefinition,
    ) -> Result<Vec<TriggerEmission>, ContractError> {
        let params: MarketTickTriggerParams = decode_builtin_trigger_params(definition)?;
        Ok(vec![TriggerEmission {
            event_id: format!("builtin-event-{}", definition.trigger_id),
            occurred_at_ms: context.now_ms,
            checkpoint: None,
            source: Some(definition.source.clone()),
            payload: serde_json::json!({
                "kind": BUILTIN_TRIGGER_MARKET_TICK_KIND,
                "source": definition.source,
                "symbol": params.symbol,
            }),
            dedup_key: None,
            dedup_window_ms: None,
            cooldown_key: None,
            cooldown_ms: None,
        }])
    }
}
