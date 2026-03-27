//! [INPUT]
//! Builtin trigger specs, ingress params schemas, and payload semantics from builtin trigger handlers.
//!
//! [OUTPUT]
//! Exposes static builtin-trigger descriptors derived from the canonical trigger spec source.
//!
//! [ROLE]
//! Maps canonical trigger specs into discoverability metadata for the CLI layer.

use super::spec::{builtin_trigger_specs, BuiltinTriggerSpec};

#[cfg(test)]
use crate::ingress::contract::{BUILTIN_TRIGGER_WEBHOOK_KIND, BUILTIN_TRIGGER_WEBSOCKET_KIND};

#[cfg(test)]
use super::emitters::{
    cron::BUILTIN_TRIGGER_CRON_KIND, manual::BUILTIN_TRIGGER_MANUAL_KIND,
    market_tick::BUILTIN_TRIGGER_MARKET_TICK_KIND,
};

pub type BuiltinTriggerCatalogDescriptor = BuiltinTriggerSpec;

pub fn builtin_trigger_descriptors() -> &'static [BuiltinTriggerCatalogDescriptor] {
    builtin_trigger_specs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn builtin_trigger_descriptors_cover_registered_sources() {
        let expected = BTreeSet::from([
            BUILTIN_TRIGGER_MANUAL_KIND,
            BUILTIN_TRIGGER_MARKET_TICK_KIND,
            BUILTIN_TRIGGER_CRON_KIND,
            BUILTIN_TRIGGER_WEBHOOK_KIND,
            BUILTIN_TRIGGER_WEBSOCKET_KIND,
        ]);
        let actual = builtin_trigger_descriptors()
            .iter()
            .map(|descriptor| descriptor.source)
            .collect::<BTreeSet<_>>();
        assert_eq!(actual, expected);
    }
}
