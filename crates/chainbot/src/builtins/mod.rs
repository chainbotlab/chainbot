//! [INPUT]
//! Root layout paths, plugin manifests, trigger definitions, secret resolution services, script worker hosting, and builtin dispatch requests.
//!
//! [OUTPUT]
//! Exposes builtin node and builtin trigger module trees, plus crate-level convenience re-exports for runtime assembly.
//!
//! [ROLE]
//! Defines the unified builtin namespace outside the workflow execution and trigger planes.

pub mod nodes;
pub mod triggers;

pub use nodes::{build_builtin_registry, BuiltinRuntimeContext, SecretDecryptMode};
pub use triggers::build_builtin_trigger_emissions;
