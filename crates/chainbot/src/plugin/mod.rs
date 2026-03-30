//! [INPUT]
//! Shared plugin contract types, external node-plugin host/runtime helpers, source-install helpers, and public crate module consumers.
//!
//! [OUTPUT]
//! Re-exports the canonical plugin contract, host runtime, and source-install helpers behind a stable `chainbot::plugin` module path.
//!
//! [ROLE]
//! Preserves the public plugin API while delegating implementation to contract, host, and source submodules.

use std::ffi::OsString;
use std::process::Command;

mod contract;
mod host;
pub(crate) mod source;

pub(crate) const PLUGIN_HOST_ENV_ALLOWLIST: &[&str] =
    &["PATH", "SYSTEMROOT", "WINDIR", "COMSPEC", "PATHEXT"];

pub use contract::{
    ExternalNodePluginRequest, ExternalNodePluginResponse, ExternalTriggerRuntimeContract,
    McpAuthConfig, McpPluginContract, McpStdioTransportConfig, McpStreamableHttpTransportConfig,
    McpTransportKind, PluginEventSchemaDescriptor, PluginKind, PluginManifest,
    PluginOperationDescriptor, TriggerDurableAckSemantics, TriggerHostErrorCategory,
    TriggerPushCallbackSemantics, TriggerRuntimeLifecycle, CURRENT_API_MAJOR,
    EXTERNAL_NODE_ENTRYPOINT_EXEC_V1, EXTERNAL_NODE_ENTRYPOINT_MCP_TOOL_V1,
    NODE_PLUGIN_CONTRACT_MAX_MAJOR, NODE_PLUGIN_CONTRACT_VERSION, NODE_PLUGIN_EXECUTE_CAPABILITY,
    PLUGIN_KIND_BUILTIN, PLUGIN_KIND_EXTERNAL_NODE, PLUGIN_KIND_EXTERNAL_TRIGGER,
};
pub use host::{
    ExternalNodePluginHost, NodePluginExecutionResult, PluginHostSecretMode,
};

pub(crate) fn plugin_host_allowlisted_environment() -> Vec<(&'static str, OsString)> {
    PLUGIN_HOST_ENV_ALLOWLIST
        .iter()
        .filter_map(|key| std::env::var_os(key).map(|value| (*key, value)))
        .collect()
}

pub(crate) fn configure_plugin_subprocess_environment(command: &mut Command) {
    command.env_clear();
    for (key, value) in plugin_host_allowlisted_environment() {
        command.env(key, value);
    }
}
