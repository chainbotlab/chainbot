use std::collections::BTreeMap;

use crate::builtins::nodes::script_worker::WorkerHost;
use crate::config::RootLayout;
use crate::plugin::PluginManifest;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretDecryptMode {
    Gpg,
    Plaintext,
}

#[derive(Debug, Clone)]
pub struct BuiltinRuntimeContext {
    pub root_layout: RootLayout,
    pub manifests: BTreeMap<String, PluginManifest>,
    pub secret_mode: SecretDecryptMode,
    pub worker_host: WorkerHost,
}
