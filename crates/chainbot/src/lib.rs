//! [INPUT]
//! Module declarations for the ChainBot runtime, contract, and CLI boundaries.
//!
//! [OUTPUT]
//! Exposes the crate's stable module map for config, workflow, trigger, executor, worker, state, plugin, secret, error, and CLI surfaces.
//!
//! [ROLE]
//! Freezes the public Rust module boundary for the `chainbot` crate.


pub mod cli;
pub mod config;
pub mod errors;
pub mod executor;
pub mod plugin;
pub mod secrets;
pub mod state;
pub mod trigger;
pub mod worker;
pub mod workflow;
