//! [INPUT]
//! Shared plugin manifests, external-node protocol payloads, and contract error helpers.
//!
//! [OUTPUT]
//! Defines validated plugin manifest and external-node protocol contract types for ChainBot.
//!
//! [ROLE]
//! Owns the shared plugin contract surface used by config loading, trigger orchestration, builtin external-node dispatch, and tests.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::errors::{assert_required_major, assert_supported_major, ContractError};

pub const CURRENT_API_MAJOR: u64 = 2;
pub const NODE_PLUGIN_CONTRACT_VERSION: &str = "1.0.0";
pub const NODE_PLUGIN_CONTRACT_MAX_MAJOR: u64 = 1;

pub const PLUGIN_KIND_BUILTIN: &str = "builtin";
pub const PLUGIN_KIND_EXTERNAL_NODE: &str = "external_node";
pub const PLUGIN_KIND_EXTERNAL_TRIGGER: &str = "external_trigger";
pub const NODE_PLUGIN_EXECUTE_CAPABILITY: &str = "node:execute";
pub const EXTERNAL_NODE_ENTRYPOINT_EXEC_V1: &str = "node.exec.v1";
pub const EXTERNAL_NODE_ENTRYPOINT_EXEC_V2: &str = "node.exec.v2";
pub const EXTERNAL_NODE_ENTRYPOINT_MCP_TOOL_V1: &str = "mcp.tool.v1";
pub const NODE_EXEC_V2_JSONRPC_VERSION: &str = "2.0";
pub const NODE_EXEC_V2_METHOD_EXECUTE: &str = "node.execute";

const REQUIRED_TRIGGER_HOST_ERROR_CATEGORIES: &[TriggerHostErrorCategory] = &[
    TriggerHostErrorCategory::Transport,
    TriggerHostErrorCategory::ProtocolContract,
    TriggerHostErrorCategory::PluginFatal,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PluginOperationKind {
    #[default]
    Generic,
    Read,
    Write,
    Transfer,
    RawRead,
    RawWrite,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PluginOperationDescriptor {
    pub name: String,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub input_schema: Vec<String>,
    #[serde(default)]
    pub optional_input_schema: Vec<String>,
    #[serde(default)]
    pub output_schema: Vec<String>,
    #[serde(default)]
    pub kind: PluginOperationKind,
    #[serde(default)]
    pub requires_managed_signing: bool,
    #[serde(default)]
    pub default_confirmation: Option<String>,
}

impl PluginOperationDescriptor {
    pub fn is_write_like(&self) -> bool {
        matches!(
            self.kind,
            PluginOperationKind::Write
                | PluginOperationKind::Transfer
                | PluginOperationKind::RawWrite
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginTriggerListenerMode {
    EventLog,
    StateChange,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PluginEventSchemaDescriptor {
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub fields: Vec<String>,
    #[serde(default)]
    pub listener_modes: Vec<PluginTriggerListenerMode>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PluginActivationContract {
    #[serde(default)]
    pub required_secret_slots: Vec<String>,
    #[serde(default)]
    pub optional_secret_slots: Vec<String>,
    #[serde(default)]
    pub requires_allowed_origins: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PluginActivationEnvelope {
    #[serde(default)]
    pub secrets: BTreeMap<String, String>,
    #[serde(default)]
    pub allowed_origins: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PluginKind {
    Builtin,
    ExternalNode,
    ExternalTrigger,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerRuntimeLifecycle {
    ProcessShortLived,
    ProcessDaemonSession,
    WasmDaemonPersistentSession,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WasmTriggerAbi {
    ComponentV1,
    CoreV0,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerPushCallbackSemantics {
    InlineResponse,
    HostCallback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerDurableAckSemantics {
    CallerScope,
    AfterStorePersist,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerHostErrorCategory {
    Transport,
    ProtocolContract,
    PluginFatal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalTriggerRuntimeContract {
    #[serde(default)]
    pub lifecycle: Option<TriggerRuntimeLifecycle>,
    #[serde(default)]
    pub push_callback: Option<TriggerPushCallbackSemantics>,
    #[serde(default)]
    pub durable_ack: Option<TriggerDurableAckSemantics>,
    #[serde(default)]
    pub host_error_categories: Vec<TriggerHostErrorCategory>,
    #[serde(default)]
    pub module: Option<String>,
    #[serde(default)]
    pub abi: Option<WasmTriggerAbi>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpTransportKind {
    Stdio,
    StreamableHttp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpStdioTransportConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpStreamableHttpTransportConfig {
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpAuthConfig {
    #[serde(default)]
    pub header_name: Option<String>,
    #[serde(default)]
    pub token_secret_ref: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpPluginContract {
    pub transport: McpTransportKind,
    #[serde(default)]
    pub stdio: Option<McpStdioTransportConfig>,
    #[serde(default)]
    pub streamable_http: Option<McpStreamableHttpTransportConfig>,
    #[serde(default)]
    pub auth: Option<McpAuthConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginManifest {
    #[serde(rename = "manifest_version")]
    pub api_version: String,
    pub plugin_id: String,
    pub kind: String,
    pub entrypoint: String,
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub executable: Option<String>,
    #[serde(default)]
    pub input_schema: Vec<String>,
    #[serde(default)]
    pub output_schema: Vec<String>,
    #[serde(default)]
    pub trigger_runtime: Option<ExternalTriggerRuntimeContract>,
    #[serde(default)]
    pub operations: Vec<PluginOperationDescriptor>,
    #[serde(default)]
    pub event_schema: Option<PluginEventSchemaDescriptor>,
    #[serde(default)]
    pub activation: Option<PluginActivationContract>,
    #[serde(default)]
    pub mcp: Option<McpPluginContract>,
    #[serde(skip)]
    pub manifest_path: PathBuf,
}

impl PluginManifest {
    pub fn validate(&self) -> Result<(), ContractError> {
        assert_required_major(
            "plugin.manifest_version",
            &self.api_version,
            CURRENT_API_MAJOR,
        )?;

        validate_non_empty(&self.plugin_id, "plugin.plugin_id", &self.plugin_id)?;
        validate_non_empty(&self.entrypoint, "plugin.entrypoint", &self.plugin_id)?;

        validate_unique_non_empty_list(&self.capabilities, "plugin.capabilities", &self.plugin_id)?;

        match self.kind()? {
            PluginKind::Builtin => {
                ensure_list_empty(&self.input_schema, "plugin.input_schema", &self.plugin_id)?;
                ensure_list_empty(&self.output_schema, "plugin.output_schema", &self.plugin_id)?;
                ensure_trigger_runtime_absent(&self.plugin_id, self.trigger_runtime.as_ref())?;
                ensure_operations_absent(&self.plugin_id, &self.operations)?;
                ensure_event_schema_absent(&self.plugin_id, self.event_schema.as_ref())?;
                ensure_mcp_absent(&self.plugin_id, self.mcp.as_ref())?;
            }
            PluginKind::ExternalNode => {
                ensure_list_empty(&self.input_schema, "plugin.input_schema", &self.plugin_id)?;
                ensure_list_empty(&self.output_schema, "plugin.output_schema", &self.plugin_id)?;
                ensure_trigger_runtime_absent(&self.plugin_id, self.trigger_runtime.as_ref())?;
                ensure_event_schema_absent(&self.plugin_id, self.event_schema.as_ref())?;
                validate_activation_contract(&self.plugin_id, self.activation.as_ref())?;
                if self.entrypoint == EXTERNAL_NODE_ENTRYPOINT_MCP_TOOL_V1 {
                    validate_external_node_mcp_contract(self)?;
                } else {
                    validate_external_node_exec_contract(self)?;
                }
            }
            PluginKind::ExternalTrigger => {
                ensure_list_empty(&self.input_schema, "plugin.input_schema", &self.plugin_id)?;
                ensure_list_empty(&self.output_schema, "plugin.output_schema", &self.plugin_id)?;
                ensure_operations_absent(&self.plugin_id, &self.operations)?;
                ensure_mcp_absent(&self.plugin_id, self.mcp.as_ref())?;
                validate_activation_contract(&self.plugin_id, self.activation.as_ref())?;
                validate_external_trigger_runtime_contract(
                    &self.plugin_id,
                    self.executable.as_deref(),
                    self.trigger_runtime.as_ref(),
                )?;
                validate_event_schema(&self.plugin_id, self.event_schema.as_ref())?;
                if self.event_schema.is_none() {
                    return Err(ContractError::NodePluginInvalidField {
                        plugin_id: self.plugin_id.clone(),
                        field: "plugin.event_schema",
                        detail: "external_trigger plugins must declare event_schema".to_owned(),
                    });
                }
            }
        }

        Ok(())
    }

    pub fn kind(&self) -> Result<PluginKind, ContractError> {
        match self.kind.as_str() {
            PLUGIN_KIND_BUILTIN => Ok(PluginKind::Builtin),
            PLUGIN_KIND_EXTERNAL_NODE => Ok(PluginKind::ExternalNode),
            PLUGIN_KIND_EXTERNAL_TRIGGER => Ok(PluginKind::ExternalTrigger),
            _ => Err(ContractError::NodePluginInvalidKind {
                plugin_id: self.plugin_id.clone(),
                kind: self.kind.clone(),
            }),
        }
    }

    pub fn from_json_str(input: &str) -> Result<Self, ContractError> {
        let manifest: Self = serde_json::from_str(input)?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn node_operation(
        &self,
        operation_name: &str,
    ) -> Result<&PluginOperationDescriptor, ContractError> {
        if self.kind()? != PluginKind::ExternalNode {
            return Err(ContractError::NodePluginInvalidKind {
                plugin_id: self.plugin_id.clone(),
                kind: self.kind.clone(),
            });
        }
        let operation = self
            .operations
            .iter()
            .find(|operation| operation.name == operation_name)
            .ok_or_else(|| ContractError::NodePluginProtocolContractViolation {
                plugin_id: self.plugin_id.clone(),
                detail: format!("unknown operation `{operation_name}`"),
            })?;
        Ok(operation)
    }

    pub fn is_streamable_http_mcp_entrypoint(&self) -> bool {
        self.entrypoint == EXTERNAL_NODE_ENTRYPOINT_MCP_TOOL_V1
            && matches!(
                self.mcp.as_ref().map(|mcp| mcp.transport),
                Some(McpTransportKind::StreamableHttp)
            )
    }

    pub fn trigger_event_schema(&self) -> Result<&PluginEventSchemaDescriptor, ContractError> {
        if self.kind()? != PluginKind::ExternalTrigger {
            return Err(ContractError::NodePluginInvalidKind {
                plugin_id: self.plugin_id.clone(),
                kind: self.kind.clone(),
            });
        }
        self.event_schema
            .as_ref()
            .ok_or_else(|| ContractError::NodePluginInvalidField {
                plugin_id: self.plugin_id.clone(),
                field: "plugin.event_schema",
                detail: "external_trigger plugins must declare event_schema".to_owned(),
            })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExternalNodePluginRequest {
    pub contract_version: String,
    pub plugin_id: String,
    pub node_id: String,
    pub operation: String,
    #[serde(default)]
    pub requested_capabilities: Vec<String>,
    #[serde(default)]
    pub input: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub activation: Option<PluginActivationEnvelope>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExternalNodePluginResponse {
    pub contract_version: String,
    pub success: bool,
    #[serde(default)]
    pub result_state: Option<NodePluginResultState>,
    #[serde(default)]
    pub output: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum JsonRpcId {
    String(String),
    Number(i64),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExternalNodeJsonRpcRequest {
    pub jsonrpc: String,
    pub id: JsonRpcId,
    pub method: String,
    pub params: ExternalNodePluginRequest,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExternalNodeJsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(default)]
    pub data: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExternalNodeJsonRpcSuccessResult {
    pub contract_version: String,
    #[serde(default)]
    pub result_state: Option<NodePluginResultState>,
    #[serde(default)]
    pub output: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExternalNodeJsonRpcResponse {
    pub jsonrpc: String,
    pub id: JsonRpcId,
    #[serde(default)]
    pub result: Option<ExternalNodeJsonRpcSuccessResult>,
    #[serde(default)]
    pub error: Option<ExternalNodeJsonRpcError>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodePluginResultState {
    PreSubmitFailure,
    Submitted,
    Settled,
    Ambiguous,
}

impl ExternalNodePluginRequest {
    pub fn validate(&self, manifest: &PluginManifest) -> Result<(), ContractError> {
        assert_supported_major(
            "node_plugin_request.contract_version",
            &self.contract_version,
            NODE_PLUGIN_CONTRACT_MAX_MAJOR,
        )?;

        if self.plugin_id != manifest.plugin_id {
            return Err(ContractError::NodePluginProtocolContractViolation {
                plugin_id: manifest.plugin_id.clone(),
                detail: "request plugin_id does not match manifest plugin_id".to_owned(),
            });
        }

        validate_non_empty(
            &self.node_id,
            "node_plugin_request.node_id",
            &self.plugin_id,
        )?;
        validate_non_empty(
            &self.operation,
            "node_plugin_request.operation",
            &self.plugin_id,
        )?;

        for capability in &self.requested_capabilities {
            if !manifest
                .capabilities
                .iter()
                .any(|declared| declared == capability)
            {
                return Err(ContractError::NodePluginCapabilityNotDeclared {
                    plugin_id: manifest.plugin_id.clone(),
                    capability: capability.clone(),
                });
            }
        }

        let operation = manifest.node_operation(&self.operation)?;
        validate_input_schema(
            &manifest.plugin_id,
            &operation.input_schema,
            &operation.optional_input_schema,
            &self.input,
        )?;
        validate_activation_envelope(manifest, self.activation.as_ref())?;
        validate_operation_execution_requirements(&manifest.plugin_id, operation, &self.input)
    }
}

impl ExternalNodePluginResponse {
    pub fn validate(&self, manifest: &PluginManifest) -> Result<(), ContractError> {
        assert_supported_major(
            "node_plugin_response.contract_version",
            &self.contract_version,
            NODE_PLUGIN_CONTRACT_MAX_MAJOR,
        )?;
        if self.success {
            Ok(())
        } else {
            validate_non_empty(
                self.error.as_deref().unwrap_or_default(),
                "node_plugin_response.error",
                &manifest.plugin_id,
            )
        }
    }
}

impl ExternalNodeJsonRpcSuccessResult {
    pub fn validate(&self, manifest: &PluginManifest) -> Result<(), ContractError> {
        assert_supported_major(
            "node_exec_v2.result.contract_version",
            &self.contract_version,
            NODE_PLUGIN_CONTRACT_MAX_MAJOR,
        )
        .map_err(|error| match error {
            ContractError::UnsupportedFutureMajorVersion { .. }
            | ContractError::UnsupportedMajorVersion { .. } => error,
            _ => ContractError::NodePluginProtocolContractViolation {
                plugin_id: manifest.plugin_id.clone(),
                detail: "node.exec.v2 result contract_version is invalid".to_owned(),
            },
        })
    }
}

impl ExternalNodeJsonRpcResponse {
    pub fn validate(
        &self,
        manifest: &PluginManifest,
        request_id: &JsonRpcId,
    ) -> Result<(), ContractError> {
        if self.jsonrpc != NODE_EXEC_V2_JSONRPC_VERSION {
            return Err(ContractError::NodePluginProtocolContractViolation {
                plugin_id: manifest.plugin_id.clone(),
                detail: format!(
                    "node.exec.v2 response must set jsonrpc={NODE_EXEC_V2_JSONRPC_VERSION}"
                ),
            });
        }

        if &self.id != request_id {
            return Err(ContractError::NodePluginProtocolContractViolation {
                plugin_id: manifest.plugin_id.clone(),
                detail: "node.exec.v2 response id must match request id".to_owned(),
            });
        }

        match (&self.result, &self.error) {
            (Some(result), None) => result.validate(manifest),
            (None, Some(error)) => validate_non_empty(
                &error.message,
                "node_exec_v2.error.message",
                &manifest.plugin_id,
            ),
            (Some(_), Some(_)) => Err(ContractError::NodePluginProtocolContractViolation {
                plugin_id: manifest.plugin_id.clone(),
                detail: "node.exec.v2 response cannot include both result and error".to_owned(),
            }),
            (None, None) => Err(ContractError::NodePluginProtocolContractViolation {
                plugin_id: manifest.plugin_id.clone(),
                detail: "node.exec.v2 response must include either result or error".to_owned(),
            }),
        }
    }
}

pub(crate) fn validate_output_schema(
    plugin_id: &str,
    output_schema: &[String],
    output: &BTreeMap<String, serde_json::Value>,
) -> Result<(), ContractError> {
    let allowed: BTreeSet<&str> = output_schema.iter().map(String::as_str).collect();
    for key in output.keys() {
        if !allowed.contains(key.as_str()) {
            return Err(ContractError::NodePluginOutputSchemaMismatch {
                plugin_id: plugin_id.to_owned(),
                detail: format!("output key {key} is not declared in manifest output_schema"),
            });
        }
    }

    for required in output_schema {
        if !output.contains_key(required) {
            return Err(ContractError::NodePluginOutputSchemaMismatch {
                plugin_id: plugin_id.to_owned(),
                detail: format!("required output key {required} is missing"),
            });
        }
    }

    Ok(())
}

fn validate_non_empty(
    value: &str,
    field: &'static str,
    plugin_id: &str,
) -> Result<(), ContractError> {
    if value.trim().is_empty() {
        return Err(ContractError::NodePluginInvalidField {
            plugin_id: plugin_id.to_owned(),
            field,
            detail: "value cannot be empty".to_owned(),
        });
    }
    Ok(())
}

fn validate_unique_non_empty_list(
    values: &[String],
    field: &'static str,
    plugin_id: &str,
) -> Result<(), ContractError> {
    let mut seen = BTreeSet::new();
    for value in values {
        validate_non_empty(value, field, plugin_id)?;
        if !seen.insert(value.clone()) {
            return Err(ContractError::NodePluginInvalidField {
                plugin_id: plugin_id.to_owned(),
                field,
                detail: format!("duplicated value: {value}"),
            });
        }
    }
    Ok(())
}

fn ensure_list_empty(
    values: &[String],
    field: &'static str,
    plugin_id: &str,
) -> Result<(), ContractError> {
    if values.is_empty() {
        return Ok(());
    }
    Err(ContractError::NodePluginInvalidField {
        plugin_id: plugin_id.to_owned(),
        field,
        detail: "field is not allowed for this plugin kind".to_owned(),
    })
}

fn ensure_trigger_runtime_absent(
    plugin_id: &str,
    trigger_runtime: Option<&ExternalTriggerRuntimeContract>,
) -> Result<(), ContractError> {
    if trigger_runtime.is_none() {
        return Ok(());
    }
    Err(ContractError::NodePluginInvalidField {
        plugin_id: plugin_id.to_owned(),
        field: "plugin.trigger_runtime",
        detail: "field is not allowed for this plugin kind".to_owned(),
    })
}

fn validate_activation_contract(
    plugin_id: &str,
    activation: Option<&PluginActivationContract>,
) -> Result<(), ContractError> {
    let Some(activation) = activation else {
        return Ok(());
    };
    validate_unique_non_empty_list(
        &activation.required_secret_slots,
        "plugin.activation.required_secret_slots",
        plugin_id,
    )?;
    validate_unique_non_empty_list(
        &activation.optional_secret_slots,
        "plugin.activation.optional_secret_slots",
        plugin_id,
    )?;
    for slot in &activation.required_secret_slots {
        if activation
            .optional_secret_slots
            .iter()
            .any(|other| other == slot)
        {
            return Err(ContractError::NodePluginInvalidField {
                plugin_id: plugin_id.to_owned(),
                field: "plugin.activation.optional_secret_slots",
                detail: format!("slot {slot} cannot be both required and optional"),
            });
        }
    }
    if activation.requires_allowed_origins
        && activation.required_secret_slots.is_empty()
        && activation.optional_secret_slots.is_empty()
    {
        return Err(ContractError::NodePluginInvalidField {
            plugin_id: plugin_id.to_owned(),
            field: "plugin.activation.requires_allowed_origins",
            detail: "allowed origins require at least one declared secret slot".to_owned(),
        });
    }
    Ok(())
}

fn validate_external_trigger_runtime_contract(
    plugin_id: &str,
    executable: Option<&str>,
    trigger_runtime: Option<&ExternalTriggerRuntimeContract>,
) -> Result<(), ContractError> {
    let Some(trigger_runtime) = trigger_runtime else {
        return Err(ContractError::NodePluginInvalidField {
            plugin_id: plugin_id.to_owned(),
            field: "plugin.trigger_runtime.lifecycle",
            detail: "field is required".to_owned(),
        });
    };

    let lifecycle =
        trigger_runtime
            .lifecycle
            .ok_or_else(|| ContractError::NodePluginInvalidField {
                plugin_id: plugin_id.to_owned(),
                field: "plugin.trigger_runtime.lifecycle",
                detail: "field is required".to_owned(),
            })?;
    let push_callback =
        trigger_runtime
            .push_callback
            .ok_or_else(|| ContractError::NodePluginInvalidField {
                plugin_id: plugin_id.to_owned(),
                field: "plugin.trigger_runtime.push_callback",
                detail: "field is required".to_owned(),
            })?;
    let durable_ack =
        trigger_runtime
            .durable_ack
            .ok_or_else(|| ContractError::NodePluginInvalidField {
                plugin_id: plugin_id.to_owned(),
                field: "plugin.trigger_runtime.durable_ack",
                detail: "field is required".to_owned(),
            })?;

    validate_trigger_host_error_categories(plugin_id, &trigger_runtime.host_error_categories)?;

    match lifecycle {
        TriggerRuntimeLifecycle::ProcessShortLived => {
            if trigger_runtime.abi.is_some() {
                return Err(ContractError::NodePluginInvalidField {
                    plugin_id: plugin_id.to_owned(),
                    field: "plugin.trigger_runtime.abi",
                    detail: "process_short_lived lifecycle does not allow trigger_runtime.abi"
                        .to_owned(),
                });
            }
            validate_non_empty(
                executable.unwrap_or_default(),
                "plugin.executable",
                plugin_id,
            )?;
            if trigger_runtime.module.is_some() {
                return Err(ContractError::NodePluginInvalidField {
                    plugin_id: plugin_id.to_owned(),
                    field: "plugin.trigger_runtime.module",
                    detail: "process_short_lived lifecycle does not allow trigger_runtime.module"
                        .to_owned(),
                });
            }
            if push_callback != TriggerPushCallbackSemantics::InlineResponse {
                return Err(ContractError::NodePluginInvalidField {
                    plugin_id: plugin_id.to_owned(),
                    field: "plugin.trigger_runtime.push_callback",
                    detail: "process_short_lived lifecycle requires push_callback=inline_response"
                        .to_owned(),
                });
            }
            if durable_ack != TriggerDurableAckSemantics::CallerScope {
                return Err(ContractError::NodePluginInvalidField {
                    plugin_id: plugin_id.to_owned(),
                    field: "plugin.trigger_runtime.durable_ack",
                    detail: "process_short_lived lifecycle requires durable_ack=caller_scope"
                        .to_owned(),
                });
            }
        }
        TriggerRuntimeLifecycle::ProcessDaemonSession => {
            if trigger_runtime.abi.is_some() {
                return Err(ContractError::NodePluginInvalidField {
                    plugin_id: plugin_id.to_owned(),
                    field: "plugin.trigger_runtime.abi",
                    detail: "process_daemon_session lifecycle does not allow trigger_runtime.abi"
                        .to_owned(),
                });
            }
            validate_non_empty(
                executable.unwrap_or_default(),
                "plugin.executable",
                plugin_id,
            )?;
            if trigger_runtime.module.is_some() {
                return Err(ContractError::NodePluginInvalidField {
                    plugin_id: plugin_id.to_owned(),
                    field: "plugin.trigger_runtime.module",
                    detail: "process_daemon_session lifecycle does not allow trigger_runtime.module"
                        .to_owned(),
                });
            }
            if push_callback != TriggerPushCallbackSemantics::InlineResponse {
                return Err(ContractError::NodePluginInvalidField {
                    plugin_id: plugin_id.to_owned(),
                    field: "plugin.trigger_runtime.push_callback",
                    detail: "process_daemon_session lifecycle requires push_callback=inline_response"
                        .to_owned(),
                });
            }
            if durable_ack != TriggerDurableAckSemantics::AfterStorePersist {
                return Err(ContractError::NodePluginInvalidField {
                    plugin_id: plugin_id.to_owned(),
                    field: "plugin.trigger_runtime.durable_ack",
                    detail: "process_daemon_session lifecycle requires durable_ack=after_store_persist"
                        .to_owned(),
                });
            }
        }
        TriggerRuntimeLifecycle::WasmDaemonPersistentSession => {
            if let Some(executable) = executable.filter(|value| !value.trim().is_empty()) {
                return Err(ContractError::NodePluginInvalidField {
                    plugin_id: plugin_id.to_owned(),
                    field: "plugin.executable",
                    detail: format!(
                        "wasm_daemon_persistent_session lifecycle does not allow plugin.executable: {executable}"
                    ),
                });
            }
            validate_non_empty(
                trigger_runtime.module.as_deref().unwrap_or_default(),
                "plugin.trigger_runtime.module",
                plugin_id,
            )?;
            if push_callback != TriggerPushCallbackSemantics::HostCallback {
                return Err(ContractError::NodePluginInvalidField {
                    plugin_id: plugin_id.to_owned(),
                    field: "plugin.trigger_runtime.push_callback",
                    detail: "wasm_daemon_persistent_session lifecycle requires push_callback=host_callback"
                        .to_owned(),
                });
            }
            if durable_ack != TriggerDurableAckSemantics::AfterStorePersist {
                return Err(ContractError::NodePluginInvalidField {
                    plugin_id: plugin_id.to_owned(),
                    field: "plugin.trigger_runtime.durable_ack",
                    detail: "wasm_daemon_persistent_session lifecycle requires durable_ack=after_store_persist"
                        .to_owned(),
                });
            }
        }
    }

    Ok(())
}

fn validate_trigger_host_error_categories(
    plugin_id: &str,
    categories: &[TriggerHostErrorCategory],
) -> Result<(), ContractError> {
    if categories.is_empty() {
        return Err(ContractError::NodePluginInvalidField {
            plugin_id: plugin_id.to_owned(),
            field: "plugin.trigger_runtime.host_error_categories",
            detail: "field must declare at least one host error category".to_owned(),
        });
    }

    let mut seen = BTreeSet::new();
    for category in categories {
        if !seen.insert(*category) {
            return Err(ContractError::NodePluginInvalidField {
                plugin_id: plugin_id.to_owned(),
                field: "plugin.trigger_runtime.host_error_categories",
                detail: format!("duplicated value: {}", category.as_str()),
            });
        }
    }

    for required in REQUIRED_TRIGGER_HOST_ERROR_CATEGORIES {
        if !seen.contains(required) {
            return Err(ContractError::NodePluginInvalidField {
                plugin_id: plugin_id.to_owned(),
                field: "plugin.trigger_runtime.host_error_categories",
                detail: format!(
                    "required host error category is missing: {}",
                    required.as_str()
                ),
            });
        }
    }
    Ok(())
}

impl TriggerHostErrorCategory {
    fn as_str(self) -> &'static str {
        match self {
            Self::Transport => "transport",
            Self::ProtocolContract => "protocol_contract",
            Self::PluginFatal => "plugin_fatal",
        }
    }
}

fn ensure_operations_absent(
    plugin_id: &str,
    operations: &[PluginOperationDescriptor],
) -> Result<(), ContractError> {
    if operations.is_empty() {
        return Ok(());
    }
    Err(ContractError::NodePluginInvalidField {
        plugin_id: plugin_id.to_owned(),
        field: "plugin.operations",
        detail: "field is not allowed for this plugin kind".to_owned(),
    })
}

fn ensure_event_schema_absent(
    plugin_id: &str,
    event_schema: Option<&PluginEventSchemaDescriptor>,
) -> Result<(), ContractError> {
    if event_schema.is_none() {
        return Ok(());
    }
    Err(ContractError::NodePluginInvalidField {
        plugin_id: plugin_id.to_owned(),
        field: "plugin.event_schema",
        detail: "field is not allowed for this plugin kind".to_owned(),
    })
}

fn ensure_mcp_absent(
    plugin_id: &str,
    mcp: Option<&McpPluginContract>,
) -> Result<(), ContractError> {
    if mcp.is_none() {
        return Ok(());
    }
    Err(ContractError::NodePluginInvalidField {
        plugin_id: plugin_id.to_owned(),
        field: "plugin.mcp",
        detail: "field is not allowed for this plugin entrypoint or kind".to_owned(),
    })
}

fn validate_external_node_mcp_contract(manifest: &PluginManifest) -> Result<(), ContractError> {
    validate_operations(&manifest.plugin_id, &manifest.operations)?;
    if manifest.operations.is_empty() {
        return Err(ContractError::NodePluginInvalidField {
            plugin_id: manifest.plugin_id.clone(),
            field: "plugin.operations",
            detail: "external_node plugins must declare at least one operation".to_owned(),
        });
    }

    if !manifest
        .capabilities
        .iter()
        .any(|capability| capability == NODE_PLUGIN_EXECUTE_CAPABILITY)
    {
        return Err(ContractError::NodePluginCapabilityNotDeclared {
            plugin_id: manifest.plugin_id.clone(),
            capability: NODE_PLUGIN_EXECUTE_CAPABILITY.to_owned(),
        });
    }

    if let Some(executable) = manifest
        .executable
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        return Err(ContractError::NodePluginInvalidField {
            plugin_id: manifest.plugin_id.clone(),
            field: "plugin.executable",
            detail: format!(
                "entrypoint={} does not allow plugin.executable: {executable}",
                EXTERNAL_NODE_ENTRYPOINT_MCP_TOOL_V1
            ),
        });
    }

    let Some(mcp) = manifest.mcp.as_ref() else {
        return Err(ContractError::NodePluginInvalidField {
            plugin_id: manifest.plugin_id.clone(),
            field: "plugin.mcp.transport",
            detail: "field is required when plugin.entrypoint=mcp.tool.v1".to_owned(),
        });
    };

    validate_mcp_contract(&manifest.plugin_id, mcp)
}

fn validate_external_node_exec_contract(manifest: &PluginManifest) -> Result<(), ContractError> {
    validate_non_empty(
        manifest.executable.as_deref().unwrap_or_default(),
        "plugin.executable",
        &manifest.plugin_id,
    )?;
    validate_operations(&manifest.plugin_id, &manifest.operations)?;
    ensure_mcp_absent(&manifest.plugin_id, manifest.mcp.as_ref())?;
    if manifest.operations.is_empty() {
        return Err(ContractError::NodePluginInvalidField {
            plugin_id: manifest.plugin_id.clone(),
            field: "plugin.operations",
            detail: "external_node plugins must declare at least one operation".to_owned(),
        });
    }

    if !manifest
        .capabilities
        .iter()
        .any(|capability| capability == NODE_PLUGIN_EXECUTE_CAPABILITY)
    {
        return Err(ContractError::NodePluginCapabilityNotDeclared {
            plugin_id: manifest.plugin_id.clone(),
            capability: NODE_PLUGIN_EXECUTE_CAPABILITY.to_owned(),
        });
    }

    match manifest.entrypoint.as_str() {
        EXTERNAL_NODE_ENTRYPOINT_EXEC_V1 | EXTERNAL_NODE_ENTRYPOINT_EXEC_V2 => Ok(()),
        _ => Err(ContractError::NodePluginInvalidField {
            plugin_id: manifest.plugin_id.clone(),
            field: "plugin.entrypoint",
            detail: "unsupported external_node subprocess entrypoint".to_owned(),
        }),
    }
}

fn validate_mcp_contract(plugin_id: &str, mcp: &McpPluginContract) -> Result<(), ContractError> {
    if let Some(auth) = mcp.auth.as_ref() {
        validate_non_empty(
            auth.header_name.as_deref().unwrap_or_default(),
            "plugin.mcp.auth.header_name",
            plugin_id,
        )?;
        validate_non_empty(
            auth.token_secret_ref.as_deref().unwrap_or_default(),
            "plugin.mcp.auth.token_secret_ref",
            plugin_id,
        )?;
    }

    match mcp.transport {
        McpTransportKind::Stdio => {
            if mcp.streamable_http.is_some() {
                return Err(ContractError::NodePluginInvalidField {
                    plugin_id: plugin_id.to_owned(),
                    field: "plugin.mcp.streamable_http",
                    detail: "transport=stdio does not allow mcp.streamable_http".to_owned(),
                });
            }

            let Some(stdio) = mcp.stdio.as_ref() else {
                return Err(ContractError::NodePluginInvalidField {
                    plugin_id: plugin_id.to_owned(),
                    field: "plugin.mcp.stdio.command",
                    detail: "field is required when plugin.mcp.transport=stdio".to_owned(),
                });
            };

            validate_non_empty(&stdio.command, "plugin.mcp.stdio.command", plugin_id)?;
            for arg in &stdio.args {
                validate_non_empty(arg, "plugin.mcp.stdio.args", plugin_id)?;
            }
        }
        McpTransportKind::StreamableHttp => {
            if mcp.stdio.is_some() {
                return Err(ContractError::NodePluginInvalidField {
                    plugin_id: plugin_id.to_owned(),
                    field: "plugin.mcp.stdio",
                    detail: "transport=streamable_http does not allow mcp.stdio".to_owned(),
                });
            }

            let Some(streamable_http) = mcp.streamable_http.as_ref() else {
                return Err(ContractError::NodePluginInvalidField {
                    plugin_id: plugin_id.to_owned(),
                    field: "plugin.mcp.streamable_http.url",
                    detail: "field is required when plugin.mcp.transport=streamable_http"
                        .to_owned(),
                });
            };

            validate_non_empty(
                &streamable_http.url,
                "plugin.mcp.streamable_http.url",
                plugin_id,
            )?;
        }
    }

    Ok(())
}

fn validate_operations(
    plugin_id: &str,
    operations: &[PluginOperationDescriptor],
) -> Result<(), ContractError> {
    let mut seen = BTreeSet::new();
    for operation in operations {
        validate_non_empty(&operation.name, "plugin.operations.name", plugin_id)?;
        if let Some(summary) = operation.summary.as_deref() {
            validate_non_empty(summary, "plugin.operations.summary", plugin_id)?;
        }
        validate_unique_non_empty_list(
            &operation.input_schema,
            "plugin.operations.input_schema",
            plugin_id,
        )?;
        validate_unique_non_empty_list(
            &operation.optional_input_schema,
            "plugin.operations.optional_input_schema",
            plugin_id,
        )?;
        validate_unique_non_empty_list(
            &operation.output_schema,
            "plugin.operations.output_schema",
            plugin_id,
        )?;
        for optional in &operation.optional_input_schema {
            if operation
                .input_schema
                .iter()
                .any(|required| required == optional)
            {
                return Err(ContractError::NodePluginInvalidField {
                    plugin_id: plugin_id.to_owned(),
                    field: "plugin.operations.optional_input_schema",
                    detail: format!("input {optional} cannot be both required and optional"),
                });
            }
        }
        if operation.requires_managed_signing && !operation.is_write_like() {
            return Err(ContractError::NodePluginInvalidField {
                plugin_id: plugin_id.to_owned(),
                field: "plugin.operations.requires_managed_signing",
                detail: format!(
                    "operation {} can only require managed signing when kind is write, transfer, or raw_write",
                    operation.name
                ),
            });
        }
        if let Some(default_confirmation) = operation.default_confirmation.as_deref() {
            validate_non_empty(
                default_confirmation,
                "plugin.operations.default_confirmation",
                plugin_id,
            )?;
            if !operation.is_write_like() {
                return Err(ContractError::NodePluginInvalidField {
                    plugin_id: plugin_id.to_owned(),
                    field: "plugin.operations.default_confirmation",
                    detail: format!(
                        "operation {} can only declare default_confirmation when kind is write, transfer, or raw_write",
                        operation.name
                    ),
                });
            }
        }
        if !seen.insert(operation.name.clone()) {
            return Err(ContractError::NodePluginInvalidField {
                plugin_id: plugin_id.to_owned(),
                field: "plugin.operations.name",
                detail: format!("duplicated value: {}", operation.name),
            });
        }
    }
    Ok(())
}

fn validate_operation_execution_requirements(
    plugin_id: &str,
    operation: &PluginOperationDescriptor,
    input: &BTreeMap<String, serde_json::Value>,
) -> Result<(), ContractError> {
    if operation.default_confirmation.is_some() {
        validate_required_string_input(plugin_id, &operation.name, input, "confirmation_mode")?;
    }
    Ok(())
}

fn validate_required_string_input(
    plugin_id: &str,
    operation_name: &str,
    input: &BTreeMap<String, serde_json::Value>,
    key: &str,
) -> Result<(), ContractError> {
    let value =
        input
            .get(key)
            .ok_or_else(|| ContractError::NodePluginProtocolContractViolation {
                plugin_id: plugin_id.to_owned(),
                detail: format!(
                    "operation `{operation_name}` requires string input `{key}` at execution time"
                ),
            })?;
    match value {
        serde_json::Value::String(text) if !text.trim().is_empty() => Ok(()),
        _ => Err(ContractError::NodePluginProtocolContractViolation {
            plugin_id: plugin_id.to_owned(),
            detail: format!(
                "operation `{operation_name}` requires non-empty string input `{key}` at execution time"
            ),
        }),
    }
}

fn validate_allowed_origin(
    plugin_id: &str,
    origin: &str,
    field: &'static str,
) -> Result<(), ContractError> {
    let parsed =
        reqwest::Url::parse(origin).map_err(|source| ContractError::NodePluginInvalidField {
            plugin_id: plugin_id.to_owned(),
            field,
            detail: format!("invalid allowed origin {origin}: {source}"),
        })?;
    if !matches!(parsed.scheme(), "http" | "https" | "ws" | "wss") {
        return Err(ContractError::NodePluginInvalidField {
            plugin_id: plugin_id.to_owned(),
            field,
            detail: format!("allowed origin {origin} must use http, https, ws, or wss"),
        });
    }
    if parsed.host_str().is_none() {
        return Err(ContractError::NodePluginInvalidField {
            plugin_id: plugin_id.to_owned(),
            field,
            detail: format!("allowed origin {origin} must include a host"),
        });
    }
    if parsed.query().is_some() || parsed.fragment().is_some() {
        return Err(ContractError::NodePluginInvalidField {
            plugin_id: plugin_id.to_owned(),
            field,
            detail: format!("allowed origin {origin} must not include query or fragment"),
        });
    }
    if parsed.path() != "/" {
        return Err(ContractError::NodePluginInvalidField {
            plugin_id: plugin_id.to_owned(),
            field,
            detail: format!("allowed origin {origin} must not include a path"),
        });
    }
    Ok(())
}

fn validate_activation_envelope(
    manifest: &PluginManifest,
    activation: Option<&PluginActivationEnvelope>,
) -> Result<(), ContractError> {
    let plugin_id = manifest.plugin_id.as_str();
    let contract = manifest.activation.as_ref();
    let Some(activation) = activation else {
        if let Some(contract) = contract {
            let requires_secrets = !contract.required_secret_slots.is_empty();
            if requires_secrets || contract.requires_allowed_origins {
                let mut requirements = Vec::new();
                if requires_secrets {
                    requirements.push(format!(
                        "required activation secret slots: {}",
                        contract.required_secret_slots.join(", ")
                    ));
                }
                if contract.requires_allowed_origins {
                    requirements.push("allowed_origins".to_owned());
                }
                return Err(ContractError::NodePluginProtocolContractViolation {
                    plugin_id: manifest.plugin_id.clone(),
                    detail: format!(
                        "activation is required for this plugin ({})",
                        requirements.join("; ")
                    ),
                });
            }
        }
        return Ok(());
    };
    for (slot, value) in &activation.secrets {
        validate_non_empty(slot, "node_plugin_request.activation.secrets", plugin_id)?;
        validate_non_empty(value, "node_plugin_request.activation.secrets", plugin_id)?;
    }
    validate_unique_non_empty_list(
        &activation.allowed_origins,
        "node_plugin_request.activation.allowed_origins",
        plugin_id,
    )?;
    for origin in &activation.allowed_origins {
        validate_allowed_origin(
            plugin_id,
            origin,
            "node_plugin_request.activation.allowed_origins",
        )?;
    }

    if let Some(contract) = contract {
        for slot in activation.secrets.keys() {
            let mut declared = contract
                .required_secret_slots
                .iter()
                .chain(contract.optional_secret_slots.iter());
            if !declared.any(|candidate| candidate == slot) {
                return Err(ContractError::NodePluginProtocolContractViolation {
                    plugin_id: manifest.plugin_id.clone(),
                    detail: format!(
                        "activation secret slot `{slot}` is not declared in plugin manifest"
                    ),
                });
            }
        }
        for slot in &contract.required_secret_slots {
            if !activation.secrets.contains_key(slot) {
                return Err(ContractError::NodePluginProtocolContractViolation {
                    plugin_id: manifest.plugin_id.clone(),
                    detail: format!("required activation secret slot `{slot}` is missing"),
                });
            }
        }
        if contract.requires_allowed_origins && activation.allowed_origins.is_empty() {
            return Err(ContractError::NodePluginProtocolContractViolation {
                plugin_id: manifest.plugin_id.clone(),
                detail: "activation allowed_origins are required for this plugin".to_owned(),
            });
        }
    }
    Ok(())
}

fn validate_event_schema(
    plugin_id: &str,
    event_schema: Option<&PluginEventSchemaDescriptor>,
) -> Result<(), ContractError> {
    let Some(event_schema) = event_schema else {
        return Ok(());
    };
    if let Some(summary) = event_schema.summary.as_deref() {
        validate_non_empty(summary, "plugin.event_schema.summary", plugin_id)?;
    }
    validate_unique_non_empty_list(
        &event_schema.fields,
        "plugin.event_schema.fields",
        plugin_id,
    )?;
    let mut seen_modes = BTreeSet::new();
    for mode in &event_schema.listener_modes {
        if !seen_modes.insert(*mode) {
            return Err(ContractError::NodePluginInvalidField {
                plugin_id: plugin_id.to_owned(),
                field: "plugin.event_schema.listener_modes",
                detail: "duplicated listener mode".to_owned(),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_manifest() -> PluginManifest {
        PluginManifest {
            api_version: "2.0.0".to_owned(),
            plugin_id: "quote-node-plugin".to_owned(),
            kind: PLUGIN_KIND_EXTERNAL_NODE.to_owned(),
            entrypoint: EXTERNAL_NODE_ENTRYPOINT_EXEC_V1.to_owned(),
            capabilities: vec![NODE_PLUGIN_EXECUTE_CAPABILITY.to_owned()],
            executable: Some("bin/external_node.sh".to_owned()),
            input_schema: Vec::new(),
            output_schema: Vec::new(),
            trigger_runtime: None,
            operations: Vec::new(),
            event_schema: None,
            activation: None,
            mcp: None,
            manifest_path: PathBuf::new(),
        }
    }

    fn mcp_stdio_contract() -> McpPluginContract {
        McpPluginContract {
            transport: McpTransportKind::Stdio,
            stdio: Some(McpStdioTransportConfig {
                command: "node".to_owned(),
                args: vec!["server.js".to_owned()],
            }),
            streamable_http: None,
            auth: None,
        }
    }

    fn process_short_lived_contract() -> ExternalTriggerRuntimeContract {
        ExternalTriggerRuntimeContract {
            lifecycle: Some(TriggerRuntimeLifecycle::ProcessShortLived),
            push_callback: Some(TriggerPushCallbackSemantics::InlineResponse),
            durable_ack: Some(TriggerDurableAckSemantics::CallerScope),
            host_error_categories: vec![
                TriggerHostErrorCategory::Transport,
                TriggerHostErrorCategory::ProtocolContract,
                TriggerHostErrorCategory::PluginFatal,
            ],
            module: None,
            abi: None,
        }
    }

    fn wasm_persistent_contract() -> ExternalTriggerRuntimeContract {
        ExternalTriggerRuntimeContract {
            lifecycle: Some(TriggerRuntimeLifecycle::WasmDaemonPersistentSession),
            push_callback: Some(TriggerPushCallbackSemantics::HostCallback),
            durable_ack: Some(TriggerDurableAckSemantics::AfterStorePersist),
            host_error_categories: vec![
                TriggerHostErrorCategory::Transport,
                TriggerHostErrorCategory::ProtocolContract,
                TriggerHostErrorCategory::PluginFatal,
            ],
            module: Some("bin/external_trigger.wasm".to_owned()),
            abi: Some(WasmTriggerAbi::ComponentV1),
        }
    }

    #[test]
    fn plugin_manifest_accepts_kind_scoped_metadata() {
        let mut external_node = base_manifest();
        external_node.operations = vec![PluginOperationDescriptor {
            name: "normalize".to_owned(),
            summary: Some("Normalize quote payload".to_owned()),
            input_schema: vec!["symbol".to_owned()],
            output_schema: vec!["decision".to_owned()],
            ..PluginOperationDescriptor::default()
        }];
        external_node.activation = Some(PluginActivationContract {
            required_secret_slots: vec!["api_token".to_owned()],
            optional_secret_slots: vec!["secondary_token".to_owned()],
            requires_allowed_origins: true,
        });
        external_node
            .validate()
            .expect("operations metadata should validate");

        let mut external_trigger = base_manifest();
        external_trigger.kind = PLUGIN_KIND_EXTERNAL_TRIGGER.to_owned();
        external_trigger.capabilities = vec!["trigger.listen.event".to_owned()];
        external_trigger.trigger_runtime = Some(process_short_lived_contract());
        external_trigger.event_schema = Some(PluginEventSchemaDescriptor {
            summary: Some("Market tick payload".to_owned()),
            fields: vec!["symbol".to_owned(), "price".to_owned()],
            ..PluginEventSchemaDescriptor::default()
        });
        external_trigger
            .validate()
            .expect("event schema metadata should validate");
    }

    #[test]
    fn plugin_manifest_rejects_missing_trigger_runtime_contract_for_external_trigger() {
        let mut manifest = base_manifest();
        manifest.kind = PLUGIN_KIND_EXTERNAL_TRIGGER.to_owned();
        manifest.capabilities = vec!["trigger.listen.event".to_owned()];
        manifest.trigger_runtime = None;
        manifest.event_schema = Some(PluginEventSchemaDescriptor {
            summary: Some("tick".to_owned()),
            fields: vec!["symbol".to_owned(), "price".to_owned()],
            ..PluginEventSchemaDescriptor::default()
        });

        assert!(matches!(
            manifest.validate(),
            Err(ContractError::NodePluginInvalidField {
                field: "plugin.trigger_runtime.lifecycle",
                ..
            })
        ));
    }

    #[test]
    fn plugin_manifest_rejects_duplicate_operation_names() {
        let mut manifest = base_manifest();
        manifest.operations = vec![
            PluginOperationDescriptor {
                name: "normalize".to_owned(),
                summary: None,
                input_schema: Vec::new(),
                output_schema: Vec::new(),
                ..PluginOperationDescriptor::default()
            },
            PluginOperationDescriptor {
                name: "normalize".to_owned(),
                summary: None,
                input_schema: Vec::new(),
                output_schema: Vec::new(),
                ..PluginOperationDescriptor::default()
            },
        ];
        assert!(matches!(
            manifest.validate(),
            Err(ContractError::NodePluginInvalidField {
                field: "plugin.operations.name",
                ..
            })
        ));
    }

    #[test]
    fn plugin_manifest_rejects_activation_slot_overlap() {
        let mut manifest = base_manifest();
        manifest.operations = vec![PluginOperationDescriptor {
            name: "normalize".to_owned(),
            summary: None,
            input_schema: vec!["symbol".to_owned()],
            output_schema: vec!["decision".to_owned()],
            ..PluginOperationDescriptor::default()
        }];
        manifest.activation = Some(PluginActivationContract {
            required_secret_slots: vec!["token".to_owned()],
            optional_secret_slots: vec!["token".to_owned()],
            requires_allowed_origins: false,
        });
        assert!(matches!(
            manifest.validate(),
            Err(ContractError::NodePluginInvalidField {
                field: "plugin.activation.optional_secret_slots",
                ..
            })
        ));
    }

    #[test]
    fn node_plugin_request_validates_activation_against_manifest_contract() {
        let mut manifest = base_manifest();
        manifest.operations = vec![PluginOperationDescriptor {
            name: "normalize".to_owned(),
            summary: None,
            input_schema: vec!["symbol".to_owned()],
            output_schema: vec!["decision".to_owned()],
            ..PluginOperationDescriptor::default()
        }];
        manifest.activation = Some(PluginActivationContract {
            required_secret_slots: vec!["api_token".to_owned()],
            optional_secret_slots: vec![],
            requires_allowed_origins: true,
        });

        let valid = ExternalNodePluginRequest {
            contract_version: "1.0.0".to_owned(),
            plugin_id: manifest.plugin_id.clone(),
            node_id: "node-1".to_owned(),
            operation: "normalize".to_owned(),
            requested_capabilities: vec![NODE_PLUGIN_EXECUTE_CAPABILITY.to_owned()],
            input: BTreeMap::from([(String::from("symbol"), serde_json::json!("BTCUSDT"))]),
            activation: Some(PluginActivationEnvelope {
                secrets: BTreeMap::from([(String::from("api_token"), String::from("secret"))]),
                allowed_origins: vec!["https://api.example.test".to_owned()],
            }),
        };
        valid
            .validate(&manifest)
            .expect("activation should satisfy manifest contract");

        let missing_origin = ExternalNodePluginRequest {
            activation: Some(PluginActivationEnvelope {
                secrets: BTreeMap::from([(String::from("api_token"), String::from("secret"))]),
                allowed_origins: Vec::new(),
            }),
            ..valid.clone()
        };
        assert!(matches!(
            missing_origin.validate(&manifest),
            Err(ContractError::NodePluginProtocolContractViolation { detail, .. })
                if detail.contains("allowed_origins")
        ));

        let unknown_slot = ExternalNodePluginRequest {
            activation: Some(PluginActivationEnvelope {
                secrets: BTreeMap::from([(String::from("other"), String::from("secret"))]),
                allowed_origins: vec!["https://api.example.test".to_owned()],
            }),
            ..valid
        };
        assert!(matches!(
            unknown_slot.validate(&manifest),
            Err(ContractError::NodePluginProtocolContractViolation { detail, .. })
                if detail.contains("not declared")
        ));
    }

    #[test]
    fn plugin_manifest_rejects_wrong_metadata_for_kind() {
        let mut external_node = base_manifest();
        external_node.event_schema = Some(PluginEventSchemaDescriptor {
            summary: Some("wrong".to_owned()),
            fields: vec!["symbol".to_owned()],
            ..PluginEventSchemaDescriptor::default()
        });
        assert!(matches!(
            external_node.validate(),
            Err(ContractError::NodePluginInvalidField {
                field: "plugin.event_schema",
                ..
            })
        ));

        let mut external_trigger = base_manifest();
        external_trigger.kind = PLUGIN_KIND_EXTERNAL_TRIGGER.to_owned();
        external_trigger.capabilities = vec!["trigger.listen.event".to_owned()];
        external_trigger.operations = vec![PluginOperationDescriptor {
            name: "normalize".to_owned(),
            summary: None,
            input_schema: vec!["symbol".to_owned()],
            output_schema: vec!["decision".to_owned()],
            ..PluginOperationDescriptor::default()
        }];
        external_trigger.event_schema = Some(PluginEventSchemaDescriptor {
            summary: Some("tick".to_owned()),
            fields: vec!["symbol".to_owned()],
            ..PluginEventSchemaDescriptor::default()
        });
        assert!(matches!(
            external_trigger.validate(),
            Err(ContractError::NodePluginInvalidField {
                field: "plugin.operations",
                ..
            })
        ));
    }

    #[test]
    fn plugin_manifest_accepts_process_short_lived_trigger_runtime_contract() {
        let mut manifest = base_manifest();
        manifest.kind = PLUGIN_KIND_EXTERNAL_TRIGGER.to_owned();
        manifest.capabilities = vec!["trigger.listen.event".to_owned()];
        manifest.trigger_runtime = Some(process_short_lived_contract());
        manifest.event_schema = Some(PluginEventSchemaDescriptor {
            summary: Some("tick".to_owned()),
            fields: vec!["symbol".to_owned(), "price".to_owned()],
            ..PluginEventSchemaDescriptor::default()
        });

        manifest
            .validate()
            .expect("process short-lived trigger runtime contract should validate");
    }

    #[test]
    fn plugin_manifest_accepts_release_n_core_v0_compatibility() {
        let mut manifest = base_manifest();
        manifest.kind = PLUGIN_KIND_EXTERNAL_TRIGGER.to_owned();
        manifest.capabilities = vec!["trigger.listen.event".to_owned()];
        manifest.executable = None;
        let mut runtime = wasm_persistent_contract();
        runtime.abi = None;
        manifest.trigger_runtime = Some(runtime);
        manifest.event_schema = Some(PluginEventSchemaDescriptor {
            summary: Some("tick".to_owned()),
            fields: vec!["symbol".to_owned(), "price".to_owned()],
            ..PluginEventSchemaDescriptor::default()
        });

        manifest
            .validate()
            .expect("missing wasm ABI should remain valid during Release N");
    }

    #[test]
    fn plugin_manifest_accepts_wasm_persistent_trigger_runtime_contract() {
        let mut manifest = base_manifest();
        manifest.kind = PLUGIN_KIND_EXTERNAL_TRIGGER.to_owned();
        manifest.capabilities = vec!["trigger.listen.event".to_owned()];
        manifest.executable = None;
        manifest.trigger_runtime = Some(wasm_persistent_contract());
        manifest.event_schema = Some(PluginEventSchemaDescriptor {
            summary: Some("tick".to_owned()),
            fields: vec!["symbol".to_owned(), "price".to_owned()],
            ..PluginEventSchemaDescriptor::default()
        });

        manifest
            .validate()
            .expect("wasm persistent trigger runtime contract should validate");
    }

    #[test]
    fn plugin_manifest_rejects_mixed_trigger_runtime_declaration() {
        let mut manifest = base_manifest();
        manifest.kind = PLUGIN_KIND_EXTERNAL_TRIGGER.to_owned();
        manifest.capabilities = vec!["trigger.listen.event".to_owned()];
        manifest.executable = Some("bin/external_trigger.sh".to_owned());
        manifest.trigger_runtime = Some(wasm_persistent_contract());
        manifest.event_schema = Some(PluginEventSchemaDescriptor {
            summary: Some("tick".to_owned()),
            fields: vec!["symbol".to_owned(), "price".to_owned()],
            ..PluginEventSchemaDescriptor::default()
        });

        assert!(matches!(
            manifest.validate(),
            Err(ContractError::NodePluginInvalidField {
                field: "plugin.executable",
                ..
            })
        ));
    }

    #[test]
    fn plugin_manifest_rejects_missing_trigger_runtime_lifecycle() {
        let mut manifest = base_manifest();
        manifest.kind = PLUGIN_KIND_EXTERNAL_TRIGGER.to_owned();
        manifest.capabilities = vec!["trigger.listen.event".to_owned()];
        manifest.trigger_runtime = Some(ExternalTriggerRuntimeContract {
            lifecycle: None,
            push_callback: Some(TriggerPushCallbackSemantics::InlineResponse),
            durable_ack: Some(TriggerDurableAckSemantics::CallerScope),
            host_error_categories: vec![
                TriggerHostErrorCategory::Transport,
                TriggerHostErrorCategory::ProtocolContract,
                TriggerHostErrorCategory::PluginFatal,
            ],
            module: None,
            abi: None,
        });
        manifest.event_schema = Some(PluginEventSchemaDescriptor {
            summary: Some("tick".to_owned()),
            fields: vec!["symbol".to_owned(), "price".to_owned()],
            ..PluginEventSchemaDescriptor::default()
        });

        assert!(matches!(
            manifest.validate(),
            Err(ContractError::NodePluginInvalidField {
                field: "plugin.trigger_runtime.lifecycle",
                ..
            })
        ));
    }

    #[test]
    fn mcp_manifest_validation() {
        let mut manifest = base_manifest();
        manifest.entrypoint = EXTERNAL_NODE_ENTRYPOINT_MCP_TOOL_V1.to_owned();
        manifest.executable = None;
        manifest.operations = vec![PluginOperationDescriptor {
            name: "echo".to_owned(),
            summary: Some("Echo tool".to_owned()),
            input_schema: vec!["message".to_owned()],
            output_schema: vec!["message".to_owned()],
            ..PluginOperationDescriptor::default()
        }];
        manifest.mcp = Some(mcp_stdio_contract());

        manifest
            .validate()
            .expect("mcp.tool.v1 manifest should validate");
        assert_eq!(manifest.operations[0].name, "echo");
    }

    #[test]
    fn mcp_manifest_rejects_mixed_transport_fields() {
        let mut manifest = base_manifest();
        manifest.entrypoint = EXTERNAL_NODE_ENTRYPOINT_MCP_TOOL_V1.to_owned();
        manifest.executable = None;
        manifest.operations = vec![PluginOperationDescriptor {
            name: "echo".to_owned(),
            summary: None,
            input_schema: vec!["message".to_owned()],
            output_schema: vec!["message".to_owned()],
            ..PluginOperationDescriptor::default()
        }];
        manifest.mcp = Some(McpPluginContract {
            transport: McpTransportKind::Stdio,
            stdio: Some(McpStdioTransportConfig {
                command: "node".to_owned(),
                args: vec!["server.js".to_owned()],
            }),
            streamable_http: Some(McpStreamableHttpTransportConfig {
                url: "https://example.test/mcp".to_owned(),
            }),
            auth: Some(McpAuthConfig {
                header_name: Some("Authorization".to_owned()),
                token_secret_ref: Some("secret://mcp/http#token".to_owned()),
            }),
        });

        assert!(matches!(
            manifest.validate(),
            Err(ContractError::NodePluginInvalidField {
                field: "plugin.mcp.streamable_http",
                ..
            })
        ));
    }
}

fn validate_input_schema(
    plugin_id: &str,
    input_schema: &[String],
    optional_input_schema: &[String],
    input: &BTreeMap<String, serde_json::Value>,
) -> Result<(), ContractError> {
    let allowed: BTreeSet<&str> = input_schema
        .iter()
        .chain(optional_input_schema.iter())
        .map(String::as_str)
        .collect();
    for key in input.keys() {
        if !allowed.contains(key.as_str()) {
            return Err(ContractError::NodePluginInputSchemaMismatch {
                plugin_id: plugin_id.to_owned(),
                detail: format!("input key {key} is not declared in manifest input_schema"),
            });
        }
    }

    for required in input_schema {
        if !input.contains_key(required) {
            return Err(ContractError::NodePluginInputSchemaMismatch {
                plugin_id: plugin_id.to_owned(),
                detail: format!("required input key {required} is missing"),
            });
        }
    }

    Ok(())
}
