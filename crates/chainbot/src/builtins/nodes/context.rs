//! [INPUT]
//! Builtin node worker-host capabilities and resolved root-layout state.
//!
//! [OUTPUT]
//! Defines the shared runtime context and secret-decrypt mode passed into builtin node handlers.
//!
//! [ROLE]
//! Centralizes execution context shared across builtin node handler implementations.

use crate::builtins::nodes::script_worker::WorkerHost;
use crate::infrastructure::config::RootLayout;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretDecryptMode {
    Gpg,
    Plaintext,
}

#[derive(Debug, Clone)]
pub struct BuiltinRuntimeContext {
    pub root_layout: RootLayout,
    pub secret_mode: SecretDecryptMode,
    pub worker_host: WorkerHost,
}
