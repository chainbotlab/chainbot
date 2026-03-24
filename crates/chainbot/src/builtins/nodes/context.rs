use crate::builtins::nodes::script_worker::WorkerHost;
use crate::config::RootLayout;

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
