//! [INPUT]
//! Stable builtin trigger kinds, ingress trigger kinds, and CLI-facing trigger catalog metadata.
//!
//! [OUTPUT]
//! Defines builtin trigger specification records and catalog mode metadata for discovery surfaces.
//!
//! [ROLE]
//! Serves as the canonical builtin trigger descriptor catalog.

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
pub struct BuiltinTriggerSpec {
    pub source: &'static str,
    pub summary: &'static str,
    pub mode: BuiltinTriggerCatalogMode,
    pub params: &'static [&'static str],
    pub payload: &'static [&'static str],
}

const BUILTIN_TRIGGER_SPECS: &[BuiltinTriggerSpec] = &[
    BuiltinTriggerSpec {
        source: BUILTIN_TRIGGER_MANUAL_KIND,
        summary: "Emit one manual event with a static builtin payload.",
        mode: BuiltinTriggerCatalogMode::Poll,
        params: &[],
        payload: &["kind", "source"],
    },
    BuiltinTriggerSpec {
        source: BUILTIN_TRIGGER_MARKET_TICK_KIND,
        summary: "Emit periodic market tick events with a configurable symbol.",
        mode: BuiltinTriggerCatalogMode::Poll,
        params: &["symbol"],
        payload: &["kind", "source", "symbol"],
    },
    BuiltinTriggerSpec {
        source: BUILTIN_TRIGGER_CRON_KIND,
        summary: "Emit minute-slot cron events for matching UTC schedules.",
        mode: BuiltinTriggerCatalogMode::Poll,
        params: &["schedule", "timezone"],
        payload: &["kind", "source", "schedule", "slot_start_ms", "timezone"],
    },
    BuiltinTriggerSpec {
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
    BuiltinTriggerSpec {
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

pub fn builtin_trigger_specs() -> &'static [BuiltinTriggerSpec] {
    BUILTIN_TRIGGER_SPECS
}
