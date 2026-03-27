//! [INPUT]
//! Module declarations for the ChainBot runtime, contract, and CLI boundaries.
//!
//! [OUTPUT]
//! Exposes the crate's frozen public module map for app/domain/infrastructure boundaries plus stable facade modules.
//!
//! [ROLE]
//! Freezes the public Rust module boundary for the `chainbot` crate.


pub mod app;
pub mod domain;
pub mod infrastructure;
pub mod builtins;
pub mod errors;
pub mod ingress;
pub mod plugin;
pub mod script_protocol;
pub mod secrets;
