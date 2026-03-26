//! [INPUT]
//! Trigger definitions, runtime storage config, daemon owner identity, and durable ingress inbox rows.
//!
//! [OUTPUT]
//! Exposes ingress listener contracts, desired-state reconciliation, durable inbox draining, and the supervisor that hosts webhook and websocket servers.
//!
//! [ROLE]
//! Owns long-lived listener-backed trigger ingress without changing the trigger plane's accepted-event semantics.

pub mod contract;
pub mod inbox;
pub mod reconcile;
pub mod supervisor;
pub mod webhook;
pub mod websocket;

pub use contract::{
    DesiredIngressState, IngressAuthConfig, IngressListenerSpec, IngressRuntimeError,
    IngressTransportKind, WebSocketTriggerParams, WebhookTriggerParams,
};
pub use inbox::drain_ingress_emissions;
pub use reconcile::build_desired_ingress_state;
pub use supervisor::TriggerIngressSupervisor;
