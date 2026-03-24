//! [INPUT]
//! Durable ingress inbox rows, runtime storage access, and trigger definitions from the active serve loop.
//!
//! [OUTPUT]
//! Converts pending ingress rows into `TriggerEmission` values while preserving staging/accepted-event separation.
//!
//! [ROLE]
//! Bridges listener-backed ingress events into the existing trigger plane without changing trigger normalization semantics.

use std::collections::BTreeMap;

use crate::state::IngressInboxRecord;
use crate::state_db::RuntimeStateStore;
use crate::trigger::{TriggerDefinition, TriggerEmission, TriggerPlaneError};

#[derive(Debug, Default)]
pub struct DrainedIngressBatch {
    pub emissions: BTreeMap<String, Vec<TriggerEmission>>,
    pub inbox_ids: Vec<String>,
}

pub fn drain_ingress_emissions(
    state_store: &mut RuntimeStateStore,
    definitions: &[TriggerDefinition],
    batch_limit: usize,
) -> Result<DrainedIngressBatch, TriggerPlaneError> {
    let mut batch = DrainedIngressBatch::default();
    if batch_limit == 0 {
        return Ok(batch);
    }

    for definition in definitions.iter().filter(|definition| definition.enabled) {
        let pending = state_store.list_pending_ingress_inbox_records(
            &definition.trigger_id,
            i64::try_from(batch_limit).unwrap_or(i64::MAX),
        )?;
        if pending.is_empty() {
            continue;
        }
        let mut trigger_emissions = Vec::with_capacity(pending.len());
        for record in pending {
            trigger_emissions.push(ingress_record_to_emission(&record));
            batch.inbox_ids.push(record.inbox_id);
        }
        batch
            .emissions
            .insert(definition.trigger_id.clone(), trigger_emissions);
    }

    Ok(batch)
}

fn ingress_record_to_emission(record: &IngressInboxRecord) -> TriggerEmission {
    TriggerEmission {
        event_id: format!("ingress:{}:{}", record.trigger_id, record.ingress_event_id),
        occurred_at_ms: record.received_at_ms,
        checkpoint: None,
        source: Some(record.source.clone()),
        payload: serde_json::json!({
            "kind": record.transport_kind,
            "source": record.source,
            "transport": {
                "kind": record.transport_kind,
                "path": record.route_path,
                "method": record.http_method,
                "remote_addr": record.remote_addr,
            },
            "headers": record.headers,
            "body": record.payload,
        }),
        dedup_key: Some(format!(
            "ingress:{}:{}",
            record.trigger_id, record.ingress_event_id
        )),
        dedup_window_ms: None,
        cooldown_key: None,
        cooldown_ms: None,
    }
}
