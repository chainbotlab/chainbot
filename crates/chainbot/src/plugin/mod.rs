//! [INPUT]
//! Shared plugin contract types, external node-plugin host runtime, and public crate module consumers.
//!
//! [OUTPUT]
//! Re-exports the canonical plugin contract and host surfaces behind a stable `chainbot::plugin` module path.
//!
//! [ROLE]
//! Preserves the public plugin API while delegating implementation to contract and host submodules.

mod contract;
mod host;

pub use contract::{
    ExternalNodePluginRequest, ExternalNodePluginResponse, PluginKind, PluginManifest,
    CURRENT_API_MAJOR, NODE_PLUGIN_CONTRACT_MAX_MAJOR, NODE_PLUGIN_CONTRACT_VERSION,
    NODE_PLUGIN_EXECUTE_CAPABILITY, PLUGIN_KIND_BUILTIN, PLUGIN_KIND_EXTERNAL_NODE,
    PLUGIN_KIND_EXTERNAL_TRIGGER,
};
pub use host::{ExternalNodePluginHost, NodePluginExecutionResult};

pub(crate) use host::configure_plugin_host_environment;
