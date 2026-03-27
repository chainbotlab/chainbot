//! [INPUT]
//! Domain-layer abstractions and semantic contracts.
//!
//! [OUTPUT]
//! Exposes the stable domain namespace for workflow, trigger, runtime, and state contracts.
//!
//! [ROLE]
//! Defines the backend-agnostic domain boundary in the frozen public module tree.

pub mod state;
pub mod runtime;
pub mod trigger;
pub mod workflow;
