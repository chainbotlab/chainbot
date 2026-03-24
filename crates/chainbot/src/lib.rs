//! [INPUT]
//! Module declarations for the ChainBot runtime, contract, and CLI boundaries.
//!
//! [OUTPUT]
//! Exposes the crate's stable module map for config, workflow, trigger, ingress, executor, state, plugin facades, secret, error, CLI, script protocol contracts, and builtin-owned execution helpers.
//!
//! [ROLE]
//! Freezes the public Rust module boundary for the `chainbot` crate.


pub mod builtins;
pub mod cli;
pub mod config;
pub mod errors;
pub mod executor;
pub mod ingress;
pub mod plugin;
pub mod script_protocol;
pub mod secrets;
pub mod state;
pub mod state_db;
pub mod trigger;
pub mod workflow;
