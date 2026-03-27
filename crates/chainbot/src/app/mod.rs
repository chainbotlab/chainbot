//! [INPUT]
//! Application-layer entrypoints that bridge process-facing boundaries into crate runtime modules.
//!
//! [OUTPUT]
//! Exposes app-level adapters such as CLI execution entrypoints.
//!
//! [ROLE]
//! Defines the application boundary for the frozen crate module tree.

pub mod cli;
pub mod definitions;
pub(crate) mod runtime;

pub use runtime::execution::ExecutionPlane;
