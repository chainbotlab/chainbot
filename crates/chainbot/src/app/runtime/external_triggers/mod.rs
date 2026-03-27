//! [INPUT]
//! External trigger runtime modules for process listeners, supervision, and Wasmtime-backed plugin hosting.
//!
//! [OUTPUT]
//! Exposes the application runtime modules that manage external trigger plugin lifecycles.
//!
//! [ROLE]
//! Defines the application-layer subtree for external trigger runtime supervision.

pub(crate) mod process_listener;
pub(crate) mod supervisor;
pub(crate) mod wasmtime;
