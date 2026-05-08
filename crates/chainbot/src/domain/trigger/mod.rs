//! [INPUT]
//! Trigger definitions, trigger payload emissions, and trigger acceptance state models.
//!
//! [OUTPUT]
//! Domain-owned trigger contracts, emission normalization structures, and acceptance-plane APIs.
//!
//! [ROLE]
//! Freezes trigger-domain ownership boundaries independent from app runtime supervision.

pub mod acceptance;
pub mod contract;
pub mod emission;

pub use acceptance::{TriggerPlane, TriggerPlaneError, TriggerRunRequest, TriggerStateStore};
pub use contract::{
    TriggerAck, TriggerDefinition, TriggerEventFrame, TriggerFatal, TriggerHeartbeat,
    TriggerHostMessage, TriggerKind, TriggerPluginActivationBindings, TriggerPluginHostPolicy,
    TriggerPluginMessage, TriggerReady, TriggerStartCommand, TriggerStop, CURRENT_API_MAJOR,
    REQUIRED_TRIGGER_PLUGIN_CAPABILITY, TRIGGER_KIND_BUILTIN, TRIGGER_KIND_EXTERNAL_PLUGIN,
};
pub use emission::TriggerEmission;
