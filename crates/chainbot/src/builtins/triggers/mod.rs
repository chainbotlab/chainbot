//! [INPUT]
//! Trigger definitions, builtin trigger aliases, and accepted-at timestamps from the serve path.
//!
//! [OUTPUT]
//! Exposes the builtin-trigger contract, dispatch helpers, registry assembly, and per-emitter modules.
//!
//! [ROLE]
//! Owns the trigger builtin subsystem beneath the unified builtin namespace.

pub mod context;
pub mod contract;
pub mod dispatch;
pub mod emitters;
pub mod registry;

pub use registry::build_builtin_trigger_emissions;
