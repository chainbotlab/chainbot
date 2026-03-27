//! [INPUT]
//! Infrastructure-layer adapters and runtime integration seams.
//!
//! [OUTPUT]
//! Exposes the infrastructure namespace for config and runtime-state adapters.
//!
//! [ROLE]
//! Defines the concrete adapter boundary in the frozen public module tree.

pub mod config;
pub mod state;
