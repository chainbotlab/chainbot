//! [INPUT]
//! Builtin trigger subtype contracts, ingress params schemas, and payload semantics from builtin trigger handlers.
//!
//! [OUTPUT]
//! Defines static builtin-trigger descriptors for CLI catalog discovery and completeness tests.
//!
//! [ROLE]
//! Keeps builtin trigger discoverability metadata co-located with builtin trigger ownership rather than the CLI layer.

use crate::ingress::contract::{BUILTIN_TRIGGER_WEBHOOK_KIND, BUILTIN_TRIGGER_WEBSOCKET_KIND};

use super::emitters::{
    cron::BUILTIN_TRIGGER_CRON_KIND, manual::BUILTIN_TRIGGER_MANUAL_KIND,
    market_tick::BUILTIN_TRIGGER_MARKET_TICK_KIND,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltinTriggerCatalogMode {
    Poll,
    Ingress,
}

impl BuiltinTriggerCatalogMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Poll => "poll",
            Self::Ingress => "ingress",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuiltinTriggerCatalogDescriptor {
    pub source: &'static str,
    pub summary: &'static str,
    pub mode: BuiltinTriggerCatalogMode,
    pub params: &'static [&'static str],
    pub payload: &'static [&'static str],
}

const BUILTIN_TRIGGER_DESCRIPTORS: &[BuiltinTriggerCatalogDescriptor] = &[
    BuiltinTriggerCatalogDescriptor {
        source: BUILTIN_TRIGGER_MANUAL_KIND,
        summary: "Emit one manual event with a static builtin payload.",
        mode: BuiltinTriggerCatalogMode::Poll,
        params: &[],
        payload: &["kind", "source"],
    },
    BuiltinTriggerCatalogDescriptor {
        source: BUILTIN_TRIGGER_MARKET_TICK_KIND,
        summary: "Emit periodic market tick events with a configurable symbol.",
        mode: BuiltinTriggerCatalogMode::Poll,
        params: &["symbol"],
        payload: &["kind", "source", "symbol"],
    },
    BuiltinTriggerCatalogDescriptor {
        source: BUILTIN_TRIGGER_CRON_KIND,
        summary: "Emit minute-slot cron events for matching UTC schedules.",
        mode: BuiltinTriggerCatalogMode::Poll,
        params: &["schedule", "timezone"],
        payload: &["kind", "source", "schedule", "slot_start_ms", "timezone"],
    },
    BuiltinTriggerCatalogDescriptor {
        source: BUILTIN_TRIGGER_WEBHOOK_KIND,
        summary: "Receive JSON webhook payloads over HTTP while serve holds the lease.",
        mode: BuiltinTriggerCatalogMode::Ingress,
        params: &[
            "bind",
            "path",
            "method",
            "auth.kind",
            "auth.header_name",
            "auth.token",
            "max_body_bytes",
            "content_type",
            "idempotency_header",
        ],
        payload: &["payload"],
    },
    BuiltinTriggerCatalogDescriptor {
        source: BUILTIN_TRIGGER_WEBSOCKET_KIND,
        summary: "Receive JSON websocket messages while serve holds the lease.",
        mode: BuiltinTriggerCatalogMode::Ingress,
        params: &[
            "bind",
            "path",
            "auth.kind",
            "auth.header_name",
            "auth.token",
            "max_connections",
            "max_message_bytes",
            "idle_timeout_ms",
        ],
        payload: &["payload"],
    },
];

pub fn builtin_trigger_descriptors() -> &'static [BuiltinTriggerCatalogDescriptor] {
    BUILTIN_TRIGGER_DESCRIPTORS
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
