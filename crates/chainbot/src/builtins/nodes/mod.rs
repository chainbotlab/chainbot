//! [INPUT]
//! Workflow node definitions, plugin manifests, secret services, script worker hosting, and builtin dispatch requests.
//!
//! [OUTPUT]
//! Exposes the builtin-node contract, runtime context, dispatch helpers, registry assembly, and per-handler modules.
//!
//! [ROLE]
//! Owns the workflow builtin-node subsystem beneath the unified builtin namespace.

pub mod catalog;
pub mod context;
pub mod contract;
pub mod dispatch;
pub mod handlers;
pub mod input_resolver;
pub mod registry;
pub mod registry_store;
pub mod script_worker;
pub mod spec;

pub use context::{BuiltinRuntimeContext, SecretDecryptMode};
pub use registry::build_builtin_registry;
