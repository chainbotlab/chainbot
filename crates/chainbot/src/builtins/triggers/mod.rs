//! [INPUT]
//! Trigger definitions, builtin trigger aliases, params payloads, and accepted-at timestamps from the serve path.
//!
//! [OUTPUT]
//! Exposes builtin-trigger validation, dispatch helpers, registry assembly, and per-emitter modules.
//!
//! [ROLE]
//! Owns the trigger builtin subsystem beneath the unified builtin namespace.

pub mod catalog;
pub mod context;
pub mod contract;
pub mod dispatch;
pub mod emitters;
pub mod registry;

pub use registry::build_builtin_trigger_emissions;
pub use registry::validate_builtin_trigger_definition;
