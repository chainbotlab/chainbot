//! [INPUT]
//! External-node plugin manifests, protocol payloads, filesystem roots, and node-invoker dispatch policy.
//!
//! [OUTPUT]
//! Executes external node plugins via entrypoint-aware invokers with executable-path, host-environment, and MCP transport guards.
//!
//! [ROLE]
//! Owns the external node-plugin host runtime boundary and internal invoker seam.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;
use std::{collections::BTreeMap, collections::BTreeSet, collections::HashMap};

use crate::errors::ContractError;
use crate::secrets::{
    GpgSecretDecryptor, PlaintextSecretDecryptor, SecretProvider, SecretReference,
};
use rmcp::model::CallToolRequestParams;
use rmcp::service::{ClientInitializeError, RoleClient, RunningService, ServiceError};
use rmcp::transport::child_process::TokioChildProcess;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::StreamableHttpClientTransport;
use reqwest::header::{HeaderName, HeaderValue};
use tokio::process::Command as TokioCommand;
use tokio::runtime::{Builder as TokioRuntimeBuilder, Runtime as TokioRuntime};

use super::{configure_plugin_subprocess_environment, plugin_host_allowlisted_environment};
use super::contract::{
    validate_output_schema, ExternalNodePluginRequest, ExternalNodePluginResponse,
    McpTransportKind, NodePluginResultState, PluginKind, PluginManifest,
    PluginOperationDescriptor, EXTERNAL_NODE_ENTRYPOINT_EXEC_V1,
    EXTERNAL_NODE_ENTRYPOINT_MCP_TOOL_V1,
};

pub(crate) const MCP_NODE_INVOCATION_LIFECYCLE_POLICY: &str = "per_invocation_session";
const MCP_STDIO_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(3);
const MCP_STDIO_TIMEOUT: Duration = Duration::from_secs(10);
const MCP_STREAMABLE_HTTP_TIMEOUT: Duration = Duration::from_secs(10);
const MCP_STREAMABLE_HTTP_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(3);

fn mcp_stdio_timeout() -> Duration {
    std::env::var("CHAINBOT_MCP_STDIO_TIMEOUT_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_millis)
        .unwrap_or(MCP_STDIO_TIMEOUT)
}

#[derive(Debug, Clone, PartialEq)]
pub struct NodePluginExecutionResult {
    pub output: std::collections::BTreeMap<String, serde_json::Value>,
    pub result_state: Option<NodePluginResultState>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalNodePluginHost {
    plugins_root: PathBuf,
    secret_runtime: Option<PluginHostSecretRuntime>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginHostSecretMode {
    Gpg,
    Plaintext,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PluginHostSecretRuntime {
    secrets_root: PathBuf,
    secret_mode: PluginHostSecretMode,
}

impl ExternalNodePluginHost {
    pub fn new(plugins_root: PathBuf) -> Self {
        Self {
            plugins_root,
            secret_runtime: None,
        }
    }

    pub fn with_secret_runtime(
        plugins_root: PathBuf,
        secrets_root: PathBuf,
        secret_mode: PluginHostSecretMode,
    ) -> Self {
        Self {
            plugins_root,
            secret_runtime: Some(PluginHostSecretRuntime {
                secrets_root,
                secret_mode,
            }),
        }
    }

    pub fn plugins_root(&self) -> &Path {
        &self.plugins_root
    }

    pub fn execute(
        &self,
        manifest: &PluginManifest,
        request: &ExternalNodePluginRequest,
    ) -> Result<NodePluginExecutionResult, ContractError> {
        self.execute_node_invocation(manifest, request)
    }

    pub fn execute_node_invocation(
        &self,
        manifest: &PluginManifest,
        request: &ExternalNodePluginRequest,
    ) -> Result<NodePluginExecutionResult, ContractError> {
        match NodeInvokerKind::for_manifest(manifest)? {
            NodeInvokerKind::LegacySubprocess => self.execute_legacy_subprocess(manifest, request),
            NodeInvokerKind::McpPerInvocationSession => {
                self.execute_mcp_per_invocation_session(manifest, request)
            }
        }
    }

    fn execute_legacy_subprocess(
        &self,
        manifest: &PluginManifest,
        request: &ExternalNodePluginRequest,
    ) -> Result<NodePluginExecutionResult, ContractError> {
        manifest.validate()?;
        request.validate(manifest)?;

        let executable = self.resolve_executable_path(manifest)?;
        let request_json = serde_json::to_vec(request).map_err(|source| {
            ContractError::NodePluginProtocolEncode {
                plugin_id: manifest.plugin_id.clone(),
                source,
            }
        })?;

        let mut command = Command::new(&executable);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        configure_plugin_subprocess_environment(&mut command);

        let mut child = command
            .spawn()
            .map_err(|source| ContractError::NodePluginSpawnFailed {
                plugin_id: manifest.plugin_id.clone(),
                executable: executable.clone(),
                source,
            })?;

        if let Some(stdin) = child.stdin.as_mut() {
            use std::io::Write;
            stdin.write_all(&request_json).map_err(|source| {
                ContractError::NodePluginProcessIo {
                    plugin_id: manifest.plugin_id.clone(),
                    operation: "write request to plugin stdin",
                    source,
                }
            })?;
        }

        let output =
            child
                .wait_with_output()
                .map_err(|source| ContractError::NodePluginProcessIo {
                    plugin_id: manifest.plugin_id.clone(),
                    operation: "wait for plugin process output",
                    source,
                })?;

        if !output.status.success() {
            return Err(ContractError::NodePluginProcessFailed {
                plugin_id: manifest.plugin_id.clone(),
                exit_code: output.status.code(),
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            });
        }

        let response: ExternalNodePluginResponse =
            serde_json::from_slice(&output.stdout).map_err(|source| {
                ContractError::NodePluginProtocolDecode {
                    plugin_id: manifest.plugin_id.clone(),
                    source,
                }
            })?;

        response.validate(manifest)?;
        if !response.success {
            return Err(ContractError::NodePluginReturnedFailure {
                plugin_id: manifest.plugin_id.clone(),
                message: response.error.unwrap_or_else(|| {
                    "plugin returned success=false without error detail".to_owned()
                }),
            });
        }

        let operation = manifest.node_operation(&request.operation)?;

        validate_output_schema(
            &manifest.plugin_id,
            &operation.output_schema,
            &response.output,
        )?;

        Ok(NodePluginExecutionResult {
            output: response.output,
            result_state: response.result_state,
        })
    }

    fn execute_mcp_per_invocation_session(
        &self,
        manifest: &PluginManifest,
        request: &ExternalNodePluginRequest,
    ) -> Result<NodePluginExecutionResult, ContractError> {
        manifest.validate()?;
        request.validate(manifest)?;

        let _session = McpInvocationSession::start(&manifest.plugin_id);
        let operation = manifest.node_operation(&request.operation)?;

        let mut adapter = create_mcp_session_adapter(self, manifest)?;

        adapter
            .initialize()
            .map_err(|failure| map_mcp_invocation_failure(&manifest.plugin_id, failure))?;

        let discovered_tools = adapter
            .list_tools()
            .map_err(|failure| map_mcp_invocation_failure(&manifest.plugin_id, failure))?;
        validate_manifest_operation_against_discovered_tools(
            &manifest.plugin_id,
            operation,
            &request.operation,
            &discovered_tools,
        )?;

        let call_result = adapter
            .call_tool(&request.operation, &request.input)
            .map_err(|failure| map_mcp_invocation_failure(&manifest.plugin_id, failure))?;

        let output = normalize_mcp_tool_result(&manifest.plugin_id, call_result)?;
        validate_output_schema(&manifest.plugin_id, &operation.output_schema, &output)?;

        Ok(NodePluginExecutionResult {
            output,
            result_state: None,
        })
    }

    fn resolve_executable_path(&self, manifest: &PluginManifest) -> Result<PathBuf, ContractError> {
        if manifest.kind()? != PluginKind::ExternalNode {
            return Err(ContractError::NodePluginInvalidKind {
                plugin_id: manifest.plugin_id.clone(),
                kind: manifest.kind.clone(),
            });
        }

        let executable = manifest.executable.as_deref().ok_or_else(|| {
            ContractError::NodePluginMissingExecutable {
                plugin_id: manifest.plugin_id.clone(),
            }
        })?;

        self.resolve_relative_executable_path(manifest, executable)
    }

    fn resolve_stdio_command_path(
        &self,
        manifest: &PluginManifest,
        command: &str,
    ) -> Result<PathBuf, ContractError> {
        if command.trim().is_empty() {
            return Err(ContractError::NodePluginProtocolContractViolation {
                plugin_id: manifest.plugin_id.clone(),
                detail: "plugin.mcp.stdio.command must not be empty".to_owned(),
            });
        }

        self.resolve_relative_executable_path(manifest, command)
    }

    fn resolve_relative_executable_path(
        &self,
        manifest: &PluginManifest,
        executable: &str,
    ) -> Result<PathBuf, ContractError> {
        let executable_path = Path::new(executable);

        if executable_path.is_absolute() {
            return Err(ContractError::NodePluginInvalidExecutablePath {
                plugin_id: manifest.plugin_id.clone(),
                executable: executable.to_owned(),
                detail: "absolute executable paths are not allowed".to_owned(),
            });
        }

        let manifest_root = self
            .manifest_root(manifest)
            .unwrap_or_else(|| self.plugins_root.clone());
        let candidate = manifest_root.join(executable_path);
        let canonical_root =
            fs::canonicalize(&self.plugins_root).map_err(|source| ContractError::Io {
                path: self.plugins_root.clone(),
                operation: "canonicalize plugins root",
                source,
            })?;
        let canonical_candidate = fs::canonicalize(&candidate).map_err(|source| {
            ContractError::NodePluginInvalidExecutablePath {
                plugin_id: manifest.plugin_id.clone(),
                executable: executable.to_owned(),
                detail: source.to_string(),
            }
        })?;

        if !canonical_candidate.starts_with(&canonical_root) {
            return Err(ContractError::NodePluginInvalidExecutablePath {
                plugin_id: manifest.plugin_id.clone(),
                executable: executable.to_owned(),
                detail: "executable path escapes plugins root".to_owned(),
            });
        }

        let metadata = fs::metadata(&canonical_candidate).map_err(|source| ContractError::Io {
            path: canonical_candidate.clone(),
            operation: "inspect plugin executable metadata",
            source,
        })?;
        if !metadata.is_file() {
            return Err(ContractError::NodePluginInvalidExecutablePath {
                plugin_id: manifest.plugin_id.clone(),
                executable: executable.to_owned(),
                detail: "resolved executable path is not a file".to_owned(),
            });
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o111 == 0 {
                return Err(ContractError::NodePluginInvalidExecutablePath {
                    plugin_id: manifest.plugin_id.clone(),
                    executable: executable.to_owned(),
                    detail: "resolved executable path is not executable".to_owned(),
                });
            }
        }

        Ok(canonical_candidate)
    }

    fn manifest_root(&self, manifest: &PluginManifest) -> Option<PathBuf> {
        manifest
            .manifest_path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .map(Path::to_path_buf)
    }

    fn resolve_streamable_http_auth_headers(
        &self,
        manifest: &PluginManifest,
    ) -> Result<HashMap<HeaderName, HeaderValue>, ContractError> {
        let Some(auth) = manifest.mcp.as_ref().and_then(|mcp| mcp.auth.as_ref()) else {
            return Ok(HashMap::new());
        };

        let secret_runtime = self.secret_runtime.as_ref().ok_or_else(|| {
            ContractError::NodePluginProtocolContractViolation {
                plugin_id: manifest.plugin_id.clone(),
                detail: format!(
                    "streamable HTTP MCP auth requires execution-time secret runtime wiring for lifecycle={MCP_NODE_INVOCATION_LIFECYCLE_POLICY}"
                ),
            }
        })?;

        let secret_reference = SecretReference::parse(
            auth.token_secret_ref
                .as_deref()
                .unwrap_or_default(),
        )?;
        let secret_value = match secret_runtime.secret_mode {
            PluginHostSecretMode::Gpg => SecretProvider::new(
                secret_runtime.secrets_root.clone(),
                GpgSecretDecryptor::new(),
            )
            .resolve_reference(&secret_reference)?,
            PluginHostSecretMode::Plaintext => SecretProvider::new(
                secret_runtime.secrets_root.clone(),
                PlaintextSecretDecryptor,
            )
            .resolve_reference(&secret_reference)?,
        };

        let header_name = HeaderName::from_bytes(
            auth.header_name
                .as_deref()
                .unwrap_or_default()
                .as_bytes(),
        )
        .map_err(|_| ContractError::NodePluginProtocolContractViolation {
            plugin_id: manifest.plugin_id.clone(),
            detail: "plugin.mcp.auth.header_name is not a valid HTTP header name".to_owned(),
        })?;
        let mut header_value = HeaderValue::from_str(secret_value.expose()).map_err(|_| {
            ContractError::NodePluginProtocolContractViolation {
                plugin_id: manifest.plugin_id.clone(),
                detail: "resolved plugin.mcp auth secret is not a valid HTTP header value"
                    .to_owned(),
            }
        })?;
        header_value.set_sensitive(true);

        Ok(HashMap::from([(header_name, header_value)]))
    }
}

trait McpSessionAdapter: Send {
    fn initialize(&mut self) -> Result<(), McpInvocationFailure>;

    fn list_tools(&mut self) -> Result<Vec<McpDiscoveredTool>, McpInvocationFailure>;

    fn call_tool(
        &mut self,
        tool_name: &str,
        arguments: &BTreeMap<String, serde_json::Value>,
    ) -> Result<McpToolCallResult, McpInvocationFailure>;
}

#[derive(Debug, Clone)]
struct McpDiscoveredTool {
    name: String,
    input_schema: serde_json::Value,
}

#[derive(Debug, Clone)]
struct McpToolCallResult {
    is_error: bool,
    structured_content: Option<serde_json::Value>,
    content: Vec<serde_json::Value>,
}

#[derive(Debug, Clone)]
struct McpNormalizedToolInputSchema {
    properties: BTreeSet<String>,
    required: BTreeSet<String>,
}

fn create_mcp_session_adapter(
    host: &ExternalNodePluginHost,
    manifest: &PluginManifest,
) -> Result<Box<dyn McpSessionAdapter>, ContractError> {
    #[cfg(test)]
    if let Some(adapter) = take_test_mcp_adapter(&manifest.plugin_id) {
        return Ok(adapter);
    }

    let Some(mcp) = manifest.mcp.as_ref() else {
        return Err(ContractError::NodePluginProtocolContractViolation {
            plugin_id: manifest.plugin_id.clone(),
            detail: format!(
                "entrypoint={} requires plugin.mcp transport contract",
                EXTERNAL_NODE_ENTRYPOINT_MCP_TOOL_V1
            ),
        });
    };

    match mcp.transport {
        McpTransportKind::Stdio => {
            let stdio = mcp.stdio.as_ref().ok_or_else(|| {
                ContractError::NodePluginProtocolContractViolation {
                    plugin_id: manifest.plugin_id.clone(),
                    detail: "mcp.transport=stdio requires plugin.mcp.stdio configuration"
                        .to_owned(),
                }
            })?;
            let command = host.resolve_stdio_command_path(manifest, &stdio.command)?;

            Ok(Box::new(RmcpStdioSessionAdapter::new(
                &manifest.plugin_id,
                command,
                stdio.args.clone(),
            )?))
        }
        McpTransportKind::StreamableHttp => {
            let streamable_http = mcp.streamable_http.as_ref().ok_or_else(|| {
                ContractError::NodePluginProtocolContractViolation {
                    plugin_id: manifest.plugin_id.clone(),
                    detail: "mcp.transport=streamable_http requires plugin.mcp.streamable_http configuration"
                        .to_owned(),
                }
            })?;
            let headers = host.resolve_streamable_http_auth_headers(manifest)?;

            Ok(Box::new(RmcpStreamableHttpSessionAdapter::new(
                &manifest.plugin_id,
                streamable_http.url.clone(),
                headers,
            )?))
        }
    }
}

struct RmcpStdioSessionAdapter {
    command: PathBuf,
    args: Vec<String>,
    runtime: TokioRuntime,
    service: Option<RunningService<RoleClient, ()>>,
}

impl RmcpStdioSessionAdapter {
    fn new(plugin_id: &str, command: PathBuf, args: Vec<String>) -> Result<Self, ContractError> {
        let runtime = TokioRuntimeBuilder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|source| ContractError::NodePluginProtocolContractViolation {
                plugin_id: plugin_id.to_owned(),
                detail: format!("failed to construct stdio MCP runtime: {source}"),
            })?;

        Ok(Self {
            command,
            args,
            runtime,
            service: None,
        })
    }
}

impl McpSessionAdapter for RmcpStdioSessionAdapter {
    fn initialize(&mut self) -> Result<(), McpInvocationFailure> {
        if self.service.is_some() {
            return Ok(());
        }

        let command_path = self.command.clone();
        let args = self.args.clone();
        let service = self.runtime.block_on(async {
            let mut command = TokioCommand::new(&command_path);
            command.args(&args);
            configure_plugin_host_environment_tokio(&mut command);

            let transport = TokioChildProcess::builder(command)
                .stderr(Stdio::null())
                .spawn()
                .map(|(transport, _stderr)| transport)
                .map_err(|source| McpInvocationFailure::Protocol {
                    detail: format!(
                        "failed to spawn stdio MCP child process `{}`: {source}",
                        command_path.display()
                    ),
                })?;

            tokio::time::timeout(mcp_stdio_timeout(), rmcp::service::serve_client((), transport))
                .await
                .map_err(|_| McpInvocationFailure::Timeout {
                    detail: format!(
                        "stdio MCP initialize request timed out after {:?}",
                        mcp_stdio_timeout()
                    ),
                })?
                .map_err(|error| map_rmcp_client_initialize_error("stdio", error))
        })?;
        self.service = Some(service);
        Ok(())
    }

    fn list_tools(&mut self) -> Result<Vec<McpDiscoveredTool>, McpInvocationFailure> {
        let service = self
            .service
            .as_mut()
            .ok_or_else(|| McpInvocationFailure::Protocol {
                detail: "stdio MCP session must be initialized before listing tools".to_owned(),
            })?;

        let tools = self
            .runtime
            .block_on(async { tokio::time::timeout(mcp_stdio_timeout(), service.list_all_tools()).await })
            .map_err(|_| McpInvocationFailure::Timeout {
                detail: format!(
                    "stdio MCP request to list tools timed out after {:?}",
                    mcp_stdio_timeout()
                ),
            })?
            .map_err(|error| map_rmcp_service_error("stdio", "list tools", error))?;

        Ok(tools
            .into_iter()
            .map(|tool| McpDiscoveredTool {
                name: tool.name.into_owned(),
                input_schema: serde_json::Value::Object(tool.input_schema.as_ref().clone()),
            })
            .collect())
    }

    fn call_tool(
        &mut self,
        tool_name: &str,
        arguments: &BTreeMap<String, serde_json::Value>,
    ) -> Result<McpToolCallResult, McpInvocationFailure> {
        let service = self
            .service
            .as_mut()
            .ok_or_else(|| McpInvocationFailure::Protocol {
                detail: "stdio MCP session must be initialized before calling tools".to_owned(),
            })?;

        let arguments = serde_json::Map::from_iter(
            arguments
                .iter()
                .map(|(key, value)| (key.clone(), value.clone())),
        );
        let params = CallToolRequestParams::new(tool_name.to_owned()).with_arguments(arguments);
        let result = self
            .runtime
            .block_on(async { tokio::time::timeout(mcp_stdio_timeout(), service.call_tool(params)).await })
            .map_err(|_| McpInvocationFailure::Timeout {
                detail: format!(
                    "stdio MCP request to call tool timed out after {:?}",
                    mcp_stdio_timeout()
                ),
            })?
            .map_err(|error| map_rmcp_service_error("stdio", "call tool", error))?;

        let content = result
            .content
            .into_iter()
            .map(|item| {
                serde_json::to_value(item).map_err(|source| McpInvocationFailure::Protocol {
                    detail: format!(
                        "failed to serialize MCP tool content into JSON for normalization: {source}"
                    ),
                })
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(McpToolCallResult {
            is_error: result.is_error.unwrap_or(false),
            structured_content: result.structured_content,
            content,
        })
    }
}

impl Drop for RmcpStdioSessionAdapter {
    fn drop(&mut self) {
        if let Some(mut service) = self.service.take() {
            let _ = self
                .runtime
                .block_on(async { service.close_with_timeout(MCP_STDIO_SHUTDOWN_TIMEOUT).await });
        }
    }
}

struct RmcpStreamableHttpSessionAdapter {
    url: String,
    headers: HashMap<HeaderName, HeaderValue>,
    runtime: TokioRuntime,
    service: Option<RunningService<RoleClient, ()>>,
}

impl RmcpStreamableHttpSessionAdapter {
    fn new(
        plugin_id: &str,
        url: String,
        headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<Self, ContractError> {
        let runtime = TokioRuntimeBuilder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|source| ContractError::NodePluginProtocolContractViolation {
                plugin_id: plugin_id.to_owned(),
                detail: format!("failed to construct streamable HTTP MCP runtime: {source}"),
            })?;

        Ok(Self {
            url,
            headers,
            runtime,
            service: None,
        })
    }
}

impl McpSessionAdapter for RmcpStreamableHttpSessionAdapter {
    fn initialize(&mut self) -> Result<(), McpInvocationFailure> {
        if self.service.is_some() {
            return Ok(());
        }

        let url = self.url.clone();
        let headers = self.headers.clone();
        let service = self.runtime.block_on(async move {
            let mut config = StreamableHttpClientTransportConfig::with_uri(url);
            if !headers.is_empty() {
                config = config.custom_headers(headers);
            }
            let transport = StreamableHttpClientTransport::from_config(config);

            tokio::time::timeout(
                MCP_STREAMABLE_HTTP_TIMEOUT,
                rmcp::service::serve_client((), transport),
            )
            .await
            .map_err(|_| McpInvocationFailure::Timeout {
                detail: format!(
                    "streamable HTTP MCP initialize request timed out after {MCP_STREAMABLE_HTTP_TIMEOUT:?}"
                ),
            })?
            .map_err(|error| map_rmcp_client_initialize_error("streamable HTTP", error))
        })?;

        self.service = Some(service);
        Ok(())
    }

    fn list_tools(&mut self) -> Result<Vec<McpDiscoveredTool>, McpInvocationFailure> {
        let service = self
            .service
            .as_mut()
            .ok_or_else(|| McpInvocationFailure::Protocol {
                detail: "streamable HTTP MCP session must be initialized before listing tools"
                    .to_owned(),
            })?;

        let tools = self
            .runtime
            .block_on(async {
                tokio::time::timeout(MCP_STREAMABLE_HTTP_TIMEOUT, service.list_all_tools()).await
            })
            .map_err(|_| McpInvocationFailure::Timeout {
                detail: format!(
                    "streamable HTTP MCP request to list tools timed out after {MCP_STREAMABLE_HTTP_TIMEOUT:?}"
                ),
            })?
            .map_err(|error| map_rmcp_service_error("streamable HTTP", "list tools", error))?;

        Ok(tools
            .into_iter()
            .map(|tool| McpDiscoveredTool {
                name: tool.name.into_owned(),
                input_schema: serde_json::Value::Object(tool.input_schema.as_ref().clone()),
            })
            .collect())
    }

    fn call_tool(
        &mut self,
        tool_name: &str,
        arguments: &BTreeMap<String, serde_json::Value>,
    ) -> Result<McpToolCallResult, McpInvocationFailure> {
        let service = self
            .service
            .as_mut()
            .ok_or_else(|| McpInvocationFailure::Protocol {
                detail: "streamable HTTP MCP session must be initialized before calling tools"
                    .to_owned(),
            })?;

        let arguments = serde_json::Map::from_iter(
            arguments
                .iter()
                .map(|(key, value)| (key.clone(), value.clone())),
        );
        let params = CallToolRequestParams::new(tool_name.to_owned()).with_arguments(arguments);
        let result = self
            .runtime
            .block_on(async {
                tokio::time::timeout(MCP_STREAMABLE_HTTP_TIMEOUT, service.call_tool(params)).await
            })
            .map_err(|_| McpInvocationFailure::Timeout {
                detail: format!(
                    "streamable HTTP MCP request to call tool timed out after {MCP_STREAMABLE_HTTP_TIMEOUT:?}"
                ),
            })?
            .map_err(|error| map_rmcp_service_error("streamable HTTP", "call tool", error))?;

        let content = result
            .content
            .into_iter()
            .map(|item| {
                serde_json::to_value(item).map_err(|source| McpInvocationFailure::Protocol {
                    detail: format!(
                        "failed to serialize MCP tool content into JSON for normalization: {source}"
                    ),
                })
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(McpToolCallResult {
            is_error: result.is_error.unwrap_or(false),
            structured_content: result.structured_content,
            content,
        })
    }
}

impl Drop for RmcpStreamableHttpSessionAdapter {
    fn drop(&mut self) {
        if let Some(mut service) = self.service.take() {
            let _ = self.runtime.block_on(async {
                service
                    .close_with_timeout(MCP_STREAMABLE_HTTP_SHUTDOWN_TIMEOUT)
                    .await
            });
        }
    }
}

fn map_rmcp_client_initialize_error(
    transport: &str,
    error: ClientInitializeError,
) -> McpInvocationFailure {
    if let Some(mapped) = normalize_streamable_http_failure_message(transport, &error.to_string()) {
        return mapped;
    }

    match error {
        ClientInitializeError::ExpectedInitResponse(response) => McpInvocationFailure::Protocol {
            detail: format!(
                "{transport} MCP initialize handshake returned an unexpected response envelope: {response:?}"
            ),
        },
        ClientInitializeError::ExpectedInitResult(result) => McpInvocationFailure::Protocol {
            detail: format!(
                "{transport} MCP initialize handshake returned an unexpected result payload: {result:?}"
            ),
        },
        ClientInitializeError::ConflictInitResponseId(expected, actual) => {
            McpInvocationFailure::Protocol {
                detail: format!(
                    "{transport} MCP initialize handshake returned mismatched response id: expected {expected:?}, got {actual:?}"
                ),
            }
        }
        ClientInitializeError::ConnectionClosed(context) => McpInvocationFailure::Protocol {
            detail: format!(
                "{transport} MCP server closed the transport before initialization completed: {context}"
            ),
        },
        ClientInitializeError::TransportError { error, context } => McpInvocationFailure::Protocol {
            detail: format!(
                "{transport} MCP transport failed during initialization ({context}): {error}"
            ),
        },
        ClientInitializeError::JsonRpcError(error) => McpInvocationFailure::Protocol {
            detail: format!("{transport} MCP initialize request failed: {error}"),
        },
        ClientInitializeError::Cancelled => McpInvocationFailure::Cancelled {
            detail: format!("{transport} MCP initialize request was cancelled"),
        },
        other => McpInvocationFailure::Protocol {
            detail: format!("{transport} MCP initialize request failed: {other}"),
        },
    }
}

fn map_rmcp_service_error(
    transport: &str,
    action: &str,
    error: ServiceError,
) -> McpInvocationFailure {
    if let Some(mapped) = normalize_streamable_http_failure_message(transport, &error.to_string()) {
        return mapped;
    }

    match error {
        ServiceError::McpError(error) => McpInvocationFailure::Protocol {
            detail: format!("{transport} MCP failed to {action}: {error}"),
        },
        ServiceError::TransportSend(error) => McpInvocationFailure::Protocol {
            detail: format!(
                "{transport} MCP transport send failed while attempting to {action}: {error}"
            ),
        },
        ServiceError::TransportClosed => McpInvocationFailure::Protocol {
            detail: format!(
                "{transport} MCP transport closed before a valid response payload was received while attempting to {action}"
            ),
        },
        ServiceError::UnexpectedResponse => McpInvocationFailure::Protocol {
            detail: format!(
                "{transport} MCP returned an unexpected response while attempting to {action}"
            ),
        },
        ServiceError::Cancelled { reason } => McpInvocationFailure::Cancelled {
            detail: reason
                .unwrap_or_else(|| format!("{transport} MCP request to {action} was cancelled")),
        },
        ServiceError::Timeout { timeout } => McpInvocationFailure::Timeout {
            detail: format!("{transport} MCP request to {action} timed out after {timeout:?}"),
        },
        other => McpInvocationFailure::Protocol {
            detail: format!("{transport} MCP request to {action} failed: {other}"),
        },
    }
}

fn normalize_streamable_http_failure_message(
    transport: &str,
    detail: &str,
) -> Option<McpInvocationFailure> {
    if transport != "streamable HTTP" {
        return None;
    }

    let normalized = detail.to_ascii_lowercase();

    if normalized.contains("401") || normalized.contains("unauthorized") || normalized.contains("authrequired") {
        return Some(McpInvocationFailure::Protocol {
            detail: "streamable HTTP MCP request failed with 401 unauthorized".to_owned(),
        });
    }

    if normalized.contains("403") || normalized.contains("forbidden") || normalized.contains("insufficientscope") {
        return Some(McpInvocationFailure::Protocol {
            detail: "streamable HTTP MCP request failed with 403 forbidden".to_owned(),
        });
    }

    if normalized.contains("sessionexpired")
        || (normalized.contains("404") && normalized.contains("session"))
        || normalized.contains("stale session")
    {
        return Some(McpInvocationFailure::Protocol {
            detail: "streamable HTTP MCP session became stale and could not be re-established"
                .to_owned(),
        });
    }

    if normalized.contains("timeout") || normalized.contains("timed out") {
        return Some(McpInvocationFailure::Timeout {
            detail: "streamable HTTP MCP request timed out while waiting for a valid response"
                .to_owned(),
        });
    }

    if normalized.contains("unexpectedcontenttype")
        || normalized.contains("unexpectedserverresponse")
        || normalized.contains("serverdoesnotsupportsse")
        || normalized.contains("missingsessionidinresponse")
        || normalized.contains("content-type")
        || normalized.contains("text/html")
        || normalized.contains("not a valid mcp")
    {
        return Some(McpInvocationFailure::Protocol {
            detail: "streamable HTTP endpoint did not return a valid MCP session/response"
                .to_owned(),
        });
    }

    if normalized.contains("reservedheaderconflict") || normalized.contains("reserved header") {
        return Some(McpInvocationFailure::Protocol {
            detail: "streamable HTTP MCP auth/header configuration conflicts with a reserved header"
                .to_owned(),
        });
    }

    None
}

fn validate_manifest_operation_against_discovered_tools(
    plugin_id: &str,
    operation: &PluginOperationDescriptor,
    operation_name: &str,
    discovered_tools: &[McpDiscoveredTool],
) -> Result<(), ContractError> {
    let mut matches = discovered_tools
        .iter()
        .filter(|tool| tool.name == operation_name)
        .collect::<Vec<_>>();

    if matches.is_empty() {
        return Err(ContractError::NodePluginProtocolContractViolation {
            plugin_id: plugin_id.to_owned(),
            detail: format!(
                "manifest operation `{operation_name}` is not present in runtime MCP tool discovery"
            ),
        });
    }

    if matches.len() > 1 {
        return Err(ContractError::NodePluginProtocolContractViolation {
            plugin_id: plugin_id.to_owned(),
            detail: format!(
                "runtime MCP tool discovery returned duplicated tool entries for `{operation_name}`"
            ),
        });
    }

    let discovered = matches
        .pop()
        .expect("matches should contain one discovered tool when not empty");
    let normalized =
        normalize_mcp_tool_input_schema(plugin_id, operation_name, &discovered.input_schema)?;

    let manifest_input_schema = operation
        .input_schema
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();

    for declared in &manifest_input_schema {
        if !normalized.properties.contains(declared) {
            return Err(ContractError::NodePluginProtocolContractViolation {
                plugin_id: plugin_id.to_owned(),
                detail: format!(
                    "manifest operation `{operation_name}` declares input key `{declared}` that is absent from MCP tool schema"
                ),
            });
        }
    }

    if normalized.required != manifest_input_schema {
        return Err(ContractError::NodePluginProtocolContractViolation {
            plugin_id: plugin_id.to_owned(),
            detail: format!(
                "manifest input schema for `{operation_name}` does not match MCP required arguments; manifest={:?}, mcp_required={:?}",
                manifest_input_schema,
                normalized.required,
            ),
        });
    }

    Ok(())
}

fn normalize_mcp_tool_input_schema(
    plugin_id: &str,
    operation_name: &str,
    input_schema: &serde_json::Value,
) -> Result<McpNormalizedToolInputSchema, ContractError> {
    let schema_object = input_schema.as_object().ok_or_else(|| {
        ContractError::NodePluginProtocolContractViolation {
            plugin_id: plugin_id.to_owned(),
            detail: format!(
                "unsupported MCP input schema for `{operation_name}`: schema must be a JSON object"
            ),
        }
    })?;

    if let Some(schema_type) = schema_object.get("type") {
        if schema_type != "object" {
            return Err(ContractError::NodePluginProtocolContractViolation {
                plugin_id: plugin_id.to_owned(),
                detail: format!(
                    "unsupported MCP input schema for `{operation_name}`: schema.type must be `object`"
                ),
            });
        }
    }

    let properties = schema_object
        .get("properties")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| ContractError::NodePluginProtocolContractViolation {
            plugin_id: plugin_id.to_owned(),
            detail: format!(
                "unsupported MCP input schema for `{operation_name}`: schema.properties must be an object"
            ),
        })?
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();

    let required = match schema_object.get("required") {
        None => BTreeSet::new(),
        Some(required_value) => required_value
            .as_array()
            .ok_or_else(|| ContractError::NodePluginProtocolContractViolation {
                plugin_id: plugin_id.to_owned(),
                detail: format!(
                    "unsupported MCP input schema for `{operation_name}`: schema.required must be an array"
                ),
            })?
            .iter()
            .map(|item| {
                item.as_str().map(str::to_owned).ok_or_else(|| {
                    ContractError::NodePluginProtocolContractViolation {
                        plugin_id: plugin_id.to_owned(),
                        detail: format!(
                            "unsupported MCP input schema for `{operation_name}`: schema.required values must be strings"
                        ),
                    }
                })
            })
            .collect::<Result<BTreeSet<_>, _>>()?,
    };

    for required_key in &required {
        if !properties.contains(required_key) {
            return Err(ContractError::NodePluginProtocolContractViolation {
                plugin_id: plugin_id.to_owned(),
                detail: format!(
                    "unsupported MCP input schema for `{operation_name}`: required key `{required_key}` is missing in schema.properties"
                ),
            });
        }
    }

    Ok(McpNormalizedToolInputSchema {
        properties,
        required,
    })
}

fn normalize_mcp_tool_result(
    plugin_id: &str,
    call_result: McpToolCallResult,
) -> Result<BTreeMap<String, serde_json::Value>, ContractError> {
    if call_result.is_error {
        return Err(ContractError::NodePluginReturnedFailure {
            plugin_id: plugin_id.to_owned(),
            message: extract_mcp_error_message(&call_result),
        });
    }

    let structured_object = call_result
        .structured_content
        .as_ref()
        .map(|value| mcp_value_as_object(plugin_id, value))
        .transpose()?
        .map(|map| map.clone());
    let content_object = normalize_mcp_content_object(plugin_id, &call_result.content)?;

    let output_object = match (structured_object, content_object) {
        (Some(structured), Some(content)) => {
            if structured != content {
                return Err(ContractError::NodePluginProtocolContractViolation {
                    plugin_id: plugin_id.to_owned(),
                    detail: "MCP tool call returned ambiguous result: structured_content and content JSON object disagree".to_owned(),
                });
            }
            structured
        }
        (Some(structured), None) => structured,
        (None, Some(content)) => content,
        (None, None) => {
            return Err(ContractError::NodePluginProtocolContractViolation {
                plugin_id: plugin_id.to_owned(),
                detail: "MCP tool call returned no structured JSON object output".to_owned(),
            });
        }
    };

    Ok(BTreeMap::from_iter(output_object.into_iter()))
}

fn normalize_mcp_content_object(
    plugin_id: &str,
    content: &[serde_json::Value],
) -> Result<Option<serde_json::Map<String, serde_json::Value>>, ContractError> {
    if content.is_empty() {
        return Ok(None);
    }

    if content.len() != 1 {
        return Err(ContractError::NodePluginProtocolContractViolation {
            plugin_id: plugin_id.to_owned(),
            detail: "unsupported MCP tool content shape: expected a single JSON content item"
                .to_owned(),
        });
    }

    let only = &content[0];
    let object =
        only.as_object()
            .ok_or_else(|| ContractError::NodePluginProtocolContractViolation {
                plugin_id: plugin_id.to_owned(),
                detail: "unsupported MCP tool content item: expected object payload".to_owned(),
            })?;

    if object
        .get("type")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|value| value == "text")
    {
        return Err(ContractError::NodePluginProtocolContractViolation {
            plugin_id: plugin_id.to_owned(),
            detail: "unsupported MCP tool content item: text-only payload cannot be mapped to node output schema".to_owned(),
        });
    }

    let payload = object.get("json").unwrap_or(only);
    let payload_object = mcp_value_as_object(plugin_id, payload)?.clone();
    Ok(Some(payload_object))
}

fn mcp_value_as_object<'a>(
    plugin_id: &str,
    value: &'a serde_json::Value,
) -> Result<&'a serde_json::Map<String, serde_json::Value>, ContractError> {
    value
        .as_object()
        .ok_or_else(|| ContractError::NodePluginProtocolContractViolation {
            plugin_id: plugin_id.to_owned(),
            detail: "unsupported MCP result shape: expected a JSON object payload".to_owned(),
        })
}

fn extract_mcp_error_message(call_result: &McpToolCallResult) -> String {
    if let Some(message) = call_result
        .structured_content
        .as_ref()
        .and_then(extract_error_detail_from_value)
    {
        return message;
    }

    if let Some(message) = call_result
        .content
        .iter()
        .find_map(extract_error_detail_from_value)
    {
        return message;
    }

    "MCP tool call returned is_error=true without error detail".to_owned()
}

fn extract_error_detail_from_value(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(message) if !message.trim().is_empty() => Some(message.clone()),
        serde_json::Value::Object(object) => {
            for key in ["error", "message", "detail", "text"] {
                if let Some(message) = object
                    .get(key)
                    .and_then(serde_json::Value::as_str)
                    .filter(|message| !message.trim().is_empty())
                {
                    return Some(message.to_owned());
                }
            }

            if let Some(nested) = object.get("json") {
                return extract_error_detail_from_value(nested);
            }

            None
        }
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NodeInvokerKind {
    LegacySubprocess,
    McpPerInvocationSession,
}

impl NodeInvokerKind {
    fn for_manifest(manifest: &PluginManifest) -> Result<Self, ContractError> {
        manifest.validate()?;
        match manifest.entrypoint.as_str() {
            EXTERNAL_NODE_ENTRYPOINT_EXEC_V1 => Ok(Self::LegacySubprocess),
            EXTERNAL_NODE_ENTRYPOINT_MCP_TOOL_V1 => Ok(Self::McpPerInvocationSession),
            _ => Err(ContractError::NodePluginInvalidField {
                plugin_id: manifest.plugin_id.clone(),
                field: "plugin.entrypoint",
                detail: "unsupported external_node entrypoint for node invoker dispatch".to_owned(),
            }),
        }
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
enum McpInvocationFailure {
    Timeout { detail: String },
    Cancelled { detail: String },
    Protocol { detail: String },
}

#[cfg(test)]
thread_local! {
    static TEST_MCP_ADAPTER_REGISTRY: std::cell::RefCell<BTreeMap<String, Box<dyn McpSessionAdapter>>> =
        std::cell::RefCell::new(BTreeMap::new());
}

#[cfg(test)]
fn install_test_mcp_adapter(plugin_id: &str, adapter: Box<dyn McpSessionAdapter>) {
    TEST_MCP_ADAPTER_REGISTRY.with(|registry| {
        registry.borrow_mut().insert(plugin_id.to_owned(), adapter);
    });
}

#[cfg(test)]
fn take_test_mcp_adapter(plugin_id: &str) -> Option<Box<dyn McpSessionAdapter>> {
    TEST_MCP_ADAPTER_REGISTRY.with(|registry| registry.borrow_mut().remove(plugin_id))
}

fn map_mcp_invocation_failure(plugin_id: &str, failure: McpInvocationFailure) -> ContractError {
    match failure {
        McpInvocationFailure::Timeout { detail } => ContractError::NodePluginProcessIo {
            plugin_id: plugin_id.to_owned(),
            operation: "execute mcp node invocation with timeout guard",
            source: std::io::Error::new(std::io::ErrorKind::TimedOut, detail),
        },
        McpInvocationFailure::Cancelled { detail } => ContractError::NodePluginProcessIo {
            plugin_id: plugin_id.to_owned(),
            operation: "execute mcp node invocation with cancellation guard",
            source: std::io::Error::new(std::io::ErrorKind::Interrupted, detail),
        },
        McpInvocationFailure::Protocol { detail } => {
            ContractError::NodePluginProtocolContractViolation {
                plugin_id: plugin_id.to_owned(),
                detail,
            }
        }
    }
}

struct McpInvocationSession {
    plugin_id: String,
}

impl McpInvocationSession {
    fn start(plugin_id: &str) -> Self {
        record_mcp_lifecycle_event(format!("start:{plugin_id}"));
        Self {
            plugin_id: plugin_id.to_owned(),
        }
    }
}

impl Drop for McpInvocationSession {
    fn drop(&mut self) {
        record_mcp_lifecycle_event(format!("stop:{}", self.plugin_id));
    }
}

#[cfg(not(test))]
fn record_mcp_lifecycle_event(_event: String) {}

#[cfg(test)]
thread_local! {
    static MCP_LIFECYCLE_EVENTS: std::cell::RefCell<Vec<String>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

#[cfg(test)]
fn record_mcp_lifecycle_event(event: String) {
    MCP_LIFECYCLE_EVENTS.with(|events| {
        events.borrow_mut().push(event);
    });
}

#[cfg(test)]
fn take_mcp_lifecycle_events() -> Vec<String> {
    MCP_LIFECYCLE_EVENTS.with(|events| {
        let mut borrowed = events.borrow_mut();
        let snapshot = borrowed.clone();
        borrowed.clear();
        snapshot
    })
}

fn configure_plugin_host_environment_tokio(command: &mut TokioCommand) {
    command.env_clear();
    for (key, value) in plugin_host_allowlisted_environment() {
        command.env(key, value);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::json;

    use super::*;
    use crate::plugin::{
        ExternalNodePluginRequest, McpPluginContract, McpStdioTransportConfig,
        McpStreamableHttpTransportConfig, McpTransportKind,
        PluginOperationDescriptor, EXTERNAL_NODE_ENTRYPOINT_MCP_TOOL_V1,
        NODE_PLUGIN_EXECUTE_CAPABILITY, PLUGIN_KIND_EXTERNAL_NODE,
    };

    fn mcp_manifest() -> PluginManifest {
        PluginManifest {
            api_version: "2.0.0".to_owned(),
            plugin_id: "mcp-quote".to_owned(),
            kind: PLUGIN_KIND_EXTERNAL_NODE.to_owned(),
            entrypoint: EXTERNAL_NODE_ENTRYPOINT_MCP_TOOL_V1.to_owned(),
            capabilities: vec![NODE_PLUGIN_EXECUTE_CAPABILITY.to_owned()],
            executable: None,
            trigger_runtime: None,
            input_schema: Vec::new(),
            output_schema: Vec::new(),
            operations: vec![PluginOperationDescriptor {
                name: "normalize".to_owned(),
                summary: Some("Normalize quote payload".to_owned()),
                input_schema: vec!["symbol".to_owned()],
                output_schema: vec!["decision".to_owned()],
                ..PluginOperationDescriptor::default()
            }],
            event_schema: None,
            mcp: Some(McpPluginContract {
                transport: McpTransportKind::Stdio,
                stdio: Some(McpStdioTransportConfig {
                    command: "node".to_owned(),
                    args: vec!["server.js".to_owned()],
                }),
                streamable_http: None,
                auth: None,
            }),
            manifest_path: PathBuf::new(),
        }
    }

    fn mcp_request(plugin_id: &str) -> ExternalNodePluginRequest {
        ExternalNodePluginRequest {
            contract_version: "1.0.0".to_owned(),
            plugin_id: plugin_id.to_owned(),
            node_id: "node-1".to_owned(),
            operation: "normalize".to_owned(),
            requested_capabilities: vec![NODE_PLUGIN_EXECUTE_CAPABILITY.to_owned()],
            input: BTreeMap::from_iter([("symbol".to_owned(), json!("BTCUSDT"))]),
            activation: None,
        }
    }

    #[derive(Debug, Clone)]
    struct StubMcpAdapter {
        initialize_result: Result<(), McpInvocationFailure>,
        list_tools_result: Result<Vec<McpDiscoveredTool>, McpInvocationFailure>,
        call_result: Result<McpToolCallResult, McpInvocationFailure>,
        expected_tool_name: String,
        expected_arguments: BTreeMap<String, serde_json::Value>,
    }

    impl StubMcpAdapter {
        fn success() -> Self {
            Self {
                initialize_result: Ok(()),
                list_tools_result: Ok(vec![McpDiscoveredTool {
                    name: "normalize".to_owned(),
                    input_schema: json!({
                        "type": "object",
                        "properties": {
                            "symbol": { "type": "string" }
                        },
                        "required": ["symbol"]
                    }),
                }]),
                call_result: Ok(McpToolCallResult {
                    is_error: false,
                    structured_content: Some(json!({ "decision": "buy" })),
                    content: Vec::new(),
                }),
                expected_tool_name: "normalize".to_owned(),
                expected_arguments: BTreeMap::from_iter([("symbol".to_owned(), json!("BTCUSDT"))]),
            }
        }
    }

    impl McpSessionAdapter for StubMcpAdapter {
        fn initialize(&mut self) -> Result<(), McpInvocationFailure> {
            self.initialize_result.clone()
        }

        fn list_tools(&mut self) -> Result<Vec<McpDiscoveredTool>, McpInvocationFailure> {
            self.list_tools_result.clone()
        }

        fn call_tool(
            &mut self,
            tool_name: &str,
            arguments: &BTreeMap<String, serde_json::Value>,
        ) -> Result<McpToolCallResult, McpInvocationFailure> {
            assert_eq!(tool_name, self.expected_tool_name);
            assert_eq!(arguments, &self.expected_arguments);
            self.call_result.clone()
        }
    }

    #[test]
    fn mcp_invoker_lifecycle_unit() {
        take_mcp_lifecycle_events();

        let host = ExternalNodePluginHost::new(PathBuf::new());
        let mut manifest = mcp_manifest();
        manifest.mcp = Some(McpPluginContract {
            transport: McpTransportKind::StreamableHttp,
            stdio: None,
            streamable_http: Some(McpStreamableHttpTransportConfig {
                url: "http://127.0.0.1:4000/mcp".to_owned(),
            }),
            auth: None,
        });
        let request = mcp_request(&manifest.plugin_id);
        install_test_mcp_adapter(&manifest.plugin_id, Box::new(StubMcpAdapter::success()));

        let result = host
            .execute_node_invocation(&manifest, &request)
            .expect("mcp invocation should keep per-invocation lifecycle around adapter calls");
        assert_eq!(result.output.get("decision"), Some(&json!("buy")));

        let events = take_mcp_lifecycle_events();
        assert_eq!(events, vec!["start:mcp-quote", "stop:mcp-quote"]);
    }

    #[test]
    fn mcp_tool_discovery_validation() {
        take_mcp_lifecycle_events();

        let host = ExternalNodePluginHost::new(PathBuf::new());
        let manifest = mcp_manifest();
        let request = mcp_request(&manifest.plugin_id);

        let mut adapter = StubMcpAdapter::success();
        adapter.list_tools_result = Ok(vec![McpDiscoveredTool {
            name: "other_tool".to_owned(),
            input_schema: json!({
                "type": "object",
                "properties": { "symbol": { "type": "string" } },
                "required": ["symbol"]
            }),
        }]);
        install_test_mcp_adapter(&manifest.plugin_id, Box::new(adapter));

        let error = host
            .execute_node_invocation(&manifest, &request)
            .expect_err("manifest operation must be present in runtime MCP discovery");
        assert!(matches!(
            error,
            ContractError::NodePluginProtocolContractViolation { plugin_id, detail }
                if plugin_id == "mcp-quote"
                    && detail.contains("not present in runtime MCP tool discovery")
        ));

        let events = take_mcp_lifecycle_events();
        assert_eq!(events, vec!["start:mcp-quote", "stop:mcp-quote"]);
    }

    #[test]
    fn mcp_schema_translation() {
        take_mcp_lifecycle_events();

        let host = ExternalNodePluginHost::new(PathBuf::new());
        let manifest = mcp_manifest();
        let request = mcp_request(&manifest.plugin_id);

        let mut adapter = StubMcpAdapter::success();
        adapter.call_result = Ok(McpToolCallResult {
            is_error: false,
            structured_content: None,
            content: vec![json!({
                "type": "json",
                "json": {
                    "decision": "sell"
                }
            })],
        });
        install_test_mcp_adapter(&manifest.plugin_id, Box::new(adapter));

        let result = host
            .execute_node_invocation(&manifest, &request)
            .expect("mcp output should be normalized into node plugin output map");
        assert_eq!(result.output.get("decision"), Some(&json!("sell")));

        let events = take_mcp_lifecycle_events();
        assert_eq!(events, vec!["start:mcp-quote", "stop:mcp-quote"]);
    }

    #[test]
    fn mcp_invalid_tool_schema_rejected() {
        take_mcp_lifecycle_events();

        let host = ExternalNodePluginHost::new(PathBuf::new());
        let manifest = mcp_manifest();
        let request = mcp_request(&manifest.plugin_id);

        let mut adapter = StubMcpAdapter::success();
        adapter.list_tools_result = Ok(vec![McpDiscoveredTool {
            name: "normalize".to_owned(),
            input_schema: json!({
                "type": "array",
                "items": { "type": "string" }
            }),
        }]);
        install_test_mcp_adapter(&manifest.plugin_id, Box::new(adapter));

        let error = host
            .execute_node_invocation(&manifest, &request)
            .expect_err("unsupported MCP tool schema must fail closed before tool call");
        assert!(matches!(
            error,
            ContractError::NodePluginProtocolContractViolation { plugin_id, detail }
                if plugin_id == "mcp-quote"
                    && detail.contains("schema.type must be `object`")
        ));

        let events = take_mcp_lifecycle_events();
        assert_eq!(events, vec!["start:mcp-quote", "stop:mcp-quote"]);
    }

    #[test]
    fn mcp_error_mapping_unit() {
        let timeout = map_mcp_invocation_failure(
            "mcp-quote",
            McpInvocationFailure::Timeout {
                detail: "request deadline exceeded".to_owned(),
            },
        );
        assert!(matches!(
            timeout,
            ContractError::NodePluginProcessIo {
                operation: "execute mcp node invocation with timeout guard",
                ..
            }
        ));

        let cancelled = map_mcp_invocation_failure(
            "mcp-quote",
            McpInvocationFailure::Cancelled {
                detail: "runtime cancelled invocation".to_owned(),
            },
        );
        assert!(matches!(
            cancelled,
            ContractError::NodePluginProcessIo {
                operation: "execute mcp node invocation with cancellation guard",
                ..
            }
        ));

        let protocol = map_mcp_invocation_failure(
            "mcp-quote",
            McpInvocationFailure::Protocol {
                detail: "invalid mcp payload".to_owned(),
            },
        );
        assert!(matches!(
            protocol,
            ContractError::NodePluginProtocolContractViolation { .. }
        ));
    }
}
