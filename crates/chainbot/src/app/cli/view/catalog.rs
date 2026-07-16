//! [INPUT]
//! Builtin node/trigger descriptors, loaded plugin manifests, and CLI-facing filtering or reference parsing requests.
//!
//! [OUTPUT]
//! Builds stable catalog list/show read models plus human-readable renderers for builtin and plugin discoverability.
//!
//! [ROLE]
//! Owns the app-layer capability-discovery read model independently from runtime registry internals.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::builtins::nodes::catalog as node_catalog;
use crate::builtins::triggers::catalog as trigger_catalog;
use crate::plugin::{
    McpTransportKind, PluginEventSchemaDescriptor, PluginKind, PluginManifest,
    PluginOperationDescriptor, EXTERNAL_NODE_ENTRYPOINT_MCP_TOOL_V1,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogFilterKind {
    BuiltinNode,
    BuiltinTrigger,
    Plugin,
}

impl CatalogFilterKind {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "builtin_node" => Some(Self::BuiltinNode),
            "builtin_trigger" => Some(Self::BuiltinTrigger),
            "plugin" => Some(Self::Plugin),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogReference {
    BuiltinNode(String),
    BuiltinTrigger(String),
    Plugin(String),
}

impl CatalogReference {
    pub fn parse(value: &str) -> Result<Self, String> {
        let Some((kind, target)) = value.split_once(':') else {
            return Err(format!(
                "catalog reference must use <kind>:<value>, got `{value}`"
            ));
        };
        if target.trim().is_empty() {
            return Err(format!(
                "catalog reference must use <kind>:<value>, got `{value}`"
            ));
        }
        match kind {
            "builtin_node" => Ok(Self::BuiltinNode(target.to_owned())),
            "builtin_trigger" => Ok(Self::BuiltinTrigger(target.to_owned())),
            "plugin" => Ok(Self::Plugin(target.to_owned())),
            _ => Err(format!(
                "unsupported catalog reference kind `{kind}`; use builtin_node, builtin_trigger, or plugin"
            )),
        }
    }

    pub fn as_string(&self) -> String {
        match self {
            Self::BuiltinNode(kind) => format!("builtin_node:{kind}"),
            Self::BuiltinTrigger(source) => format!("builtin_trigger:{source}"),
            Self::Plugin(plugin_id) => format!("plugin:{plugin_id}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CatalogListOutput {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub builtin_nodes: Vec<BuiltinNodeSummary>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub builtin_triggers: Vec<BuiltinTriggerSummary>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub plugins: Vec<PluginSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BuiltinNodeSummary {
    pub reference: String,
    pub kind: String,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BuiltinTriggerSummary {
    pub reference: String,
    pub source: String,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PluginSummary {
    pub reference: String,
    pub plugin_id: String,
    pub kind: String,
    pub entrypoint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transport: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime: Option<String>,
    pub capabilities: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub surfaces: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CatalogShowOutput {
    pub reference: String,
    pub detail: CatalogDetail,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CatalogDetail {
    BuiltinNode(BuiltinNodeDetail),
    BuiltinTrigger(BuiltinTriggerDetail),
    Plugin(PluginDetail),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CatalogFieldDescriptor {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BuiltinNodeDetail {
    pub builtin_kind: String,
    pub summary: String,
    pub operations: Vec<String>,
    pub inputs: Vec<CatalogFieldDescriptor>,
    pub outputs: Vec<CatalogFieldDescriptor>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BuiltinTriggerDetail {
    pub source: String,
    pub summary: String,
    pub mode: String,
    pub params: Vec<CatalogFieldDescriptor>,
    pub payload: Vec<CatalogFieldDescriptor>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PluginDetail {
    pub plugin_id: String,
    pub plugin_kind: String,
    pub entrypoint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transport: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime: Option<String>,
    pub capabilities: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub surfaces: Vec<String>,
    pub schema_status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub operations: Vec<PluginOperationDescriptor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_schema: Option<PluginEventSchemaDescriptor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol: Option<PluginProtocolDetail>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub input_schema: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub output_schema: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PluginProtocolDetail {
    pub start_message: Vec<String>,
    pub event_message: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PluginRuntimeSurface {
    transport: String,
    runtime: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StatusPluginSummaryView {
    pub installed_count: usize,
    pub builtin_count: usize,
    pub external_node_count: usize,
    pub external_trigger_count: usize,
    pub surfaces: Vec<String>,
}

pub fn build_catalog_list(
    filter: Option<CatalogFilterKind>,
    plugins: &[PluginManifest],
) -> CatalogListOutput {
    CatalogListOutput {
        builtin_nodes: if matches!(filter, None | Some(CatalogFilterKind::BuiltinNode)) {
            node_catalog::builtin_node_descriptors()
                .iter()
                .map(|descriptor| BuiltinNodeSummary {
                    reference: format!("builtin_node:{}", descriptor.kind),
                    kind: descriptor.kind.to_owned(),
                    summary: descriptor.summary.to_owned(),
                })
                .collect()
        } else {
            Vec::new()
        },
        builtin_triggers: if matches!(filter, None | Some(CatalogFilterKind::BuiltinTrigger)) {
            trigger_catalog::builtin_trigger_descriptors()
                .iter()
                .map(|descriptor| BuiltinTriggerSummary {
                    reference: format!("builtin_trigger:{}", descriptor.source),
                    source: descriptor.source.to_owned(),
                    summary: descriptor.summary.to_owned(),
                })
                .collect()
        } else {
            Vec::new()
        },
        plugins: if matches!(filter, None | Some(CatalogFilterKind::Plugin)) {
            plugins
                .iter()
                .map(|manifest| {
                    let runtime_surface = plugin_runtime_surface(manifest);
                    PluginSummary {
                        reference: format!("plugin:{}", manifest.plugin_id),
                        plugin_id: manifest.plugin_id.clone(),
                        kind: manifest.kind.clone(),
                        entrypoint: manifest.entrypoint.clone(),
                        transport: runtime_surface
                            .as_ref()
                            .map(|detail| detail.transport.clone()),
                        runtime: runtime_surface
                            .as_ref()
                            .map(|detail| detail.runtime.clone()),
                        capabilities: manifest.capabilities.clone(),
                        surfaces: plugin_surface_summary(manifest),
                    }
                })
                .collect()
        } else {
            Vec::new()
        },
    }
}

pub fn build_catalog_show(
    reference: &CatalogReference,
    plugins: &[PluginManifest],
) -> Result<CatalogShowOutput, String> {
    let detail = match reference {
        CatalogReference::BuiltinNode(kind) => {
            let descriptor = node_catalog::builtin_node_descriptors()
                .iter()
                .find(|descriptor| descriptor.kind == kind)
                .ok_or_else(|| format!("unknown catalog entry `{}`", reference.as_string()))?;
            CatalogDetail::BuiltinNode(BuiltinNodeDetail {
                builtin_kind: descriptor.kind.to_owned(),
                summary: descriptor.summary.to_owned(),
                operations: descriptor
                    .operations
                    .iter()
                    .map(|v| (*v).to_owned())
                    .collect(),
                inputs: descriptor
                    .inputs
                    .iter()
                    .map(|name| CatalogFieldDescriptor {
                        name: (*name).to_owned(),
                    })
                    .collect(),
                outputs: descriptor
                    .outputs
                    .iter()
                    .map(|name| CatalogFieldDescriptor {
                        name: (*name).to_owned(),
                    })
                    .collect(),
            })
        }
        CatalogReference::BuiltinTrigger(source) => {
            let descriptor = trigger_catalog::builtin_trigger_descriptors()
                .iter()
                .find(|descriptor| descriptor.source == source)
                .ok_or_else(|| format!("unknown catalog entry `{}`", reference.as_string()))?;
            CatalogDetail::BuiltinTrigger(BuiltinTriggerDetail {
                source: descriptor.source.to_owned(),
                summary: descriptor.summary.to_owned(),
                mode: descriptor.mode.as_str().to_owned(),
                params: descriptor
                    .params
                    .iter()
                    .map(|name| CatalogFieldDescriptor {
                        name: (*name).to_owned(),
                    })
                    .collect(),
                payload: descriptor
                    .payload
                    .iter()
                    .map(|name| CatalogFieldDescriptor {
                        name: (*name).to_owned(),
                    })
                    .collect(),
            })
        }
        CatalogReference::Plugin(plugin_id) => {
            let manifest = plugins
                .iter()
                .find(|manifest| manifest.plugin_id == *plugin_id)
                .ok_or_else(|| format!("unknown catalog entry `{}`", reference.as_string()))?;
            CatalogDetail::Plugin(build_plugin_detail(manifest)?)
        }
    };
    Ok(CatalogShowOutput {
        reference: reference.as_string(),
        detail,
    })
}

pub fn build_status_plugin_summary(plugins: &[PluginManifest]) -> StatusPluginSummaryView {
    let mut by_kind = BTreeMap::<PluginKind, usize>::new();
    for manifest in plugins {
        if let Ok(kind) = manifest.kind() {
            *by_kind.entry(kind).or_default() += 1;
        }
    }
    StatusPluginSummaryView {
        installed_count: plugins.len(),
        builtin_count: *by_kind.get(&PluginKind::Builtin).unwrap_or(&0),
        external_node_count: *by_kind.get(&PluginKind::ExternalNode).unwrap_or(&0),
        external_trigger_count: *by_kind.get(&PluginKind::ExternalTrigger).unwrap_or(&0),
        surfaces: plugins
            .iter()
            .map(|manifest| {
                let summary = plugin_surface_summary(manifest);
                if summary.is_empty() {
                    manifest.plugin_id.clone()
                } else {
                    format!("{} [{}]", manifest.plugin_id, summary.join(" | "))
                }
            })
            .collect(),
    }
}

fn build_plugin_detail(manifest: &PluginManifest) -> Result<PluginDetail, String> {
    let plugin_kind = manifest.kind().map_err(|error| error.to_string())?;
    let runtime_surface = plugin_runtime_surface(manifest);
    match plugin_kind {
        PluginKind::Builtin => Ok(PluginDetail {
            plugin_id: manifest.plugin_id.clone(),
            plugin_kind: manifest.kind.clone(),
            entrypoint: manifest.entrypoint.clone(),
            transport: None,
            runtime: None,
            capabilities: manifest.capabilities.clone(),
            surfaces: Vec::new(),
            schema_status: String::from("manifest_only"),
            lifecycle: None,
            operations: Vec::new(),
            event_schema: None,
            protocol: None,
            input_schema: Vec::new(),
            output_schema: Vec::new(),
        }),
        PluginKind::ExternalNode => Ok(PluginDetail {
            plugin_id: manifest.plugin_id.clone(),
            plugin_kind: manifest.kind.clone(),
            entrypoint: manifest.entrypoint.clone(),
            transport: runtime_surface
                .as_ref()
                .map(|detail| detail.transport.clone()),
            runtime: runtime_surface
                .as_ref()
                .map(|detail| detail.runtime.clone()),
            capabilities: manifest.capabilities.clone(),
            surfaces: plugin_surface_summary(manifest),
            schema_status: String::from("declared"),
            lifecycle: None,
            operations: manifest.operations.clone(),
            event_schema: None,
            protocol: None,
            input_schema: Vec::new(),
            output_schema: Vec::new(),
        }),
        PluginKind::ExternalTrigger => {
            let lifecycle = manifest
                .trigger_runtime
                .as_ref()
                .and_then(|runtime| runtime.lifecycle)
                .map(|l| match l {
                    crate::plugin::TriggerRuntimeLifecycle::ProcessShortLived => {
                        String::from("process_short_lived")
                    }
                    crate::plugin::TriggerRuntimeLifecycle::ProcessDaemonSession => {
                        String::from("process_daemon_session")
                    }
                    crate::plugin::TriggerRuntimeLifecycle::WasmDaemonPersistentSession => {
                        String::from("wasm_daemon_persistent_session")
                    }
                });

            let is_wasm = lifecycle.as_deref() == Some("wasm_daemon_persistent_session");

            let protocol = if is_wasm {
                None
            } else {
                Some(PluginProtocolDetail {
                    start_message: vec![
                        String::from("protocol_version"),
                        String::from("trigger_id"),
                        String::from("source"),
                        String::from("params"),
                        String::from("resume_checkpoint"),
                    ],
                    event_message: vec![
                        String::from("checkpoint"),
                        String::from("event_key"),
                        String::from("occurred_at_ms"),
                        String::from("payload"),
                        String::from("dedup_key"),
                        String::from("cooldown_key"),
                    ],
                })
            };

            Ok(PluginDetail {
                plugin_id: manifest.plugin_id.clone(),
                plugin_kind: manifest.kind.clone(),
                entrypoint: manifest.entrypoint.clone(),
                transport: None,
                runtime: None,
                capabilities: manifest.capabilities.clone(),
                surfaces: plugin_surface_summary(manifest),
                schema_status: String::from("declared"),
                lifecycle,
                operations: Vec::new(),
                event_schema: manifest.event_schema.clone(),
                protocol,
                input_schema: Vec::new(),
                output_schema: Vec::new(),
            })
        }
    }
}

fn plugin_runtime_surface(manifest: &PluginManifest) -> Option<PluginRuntimeSurface> {
    if manifest.entrypoint != EXTERNAL_NODE_ENTRYPOINT_MCP_TOOL_V1 {
        return None;
    }

    let mcp = manifest.mcp.as_ref()?;
    let (transport, runtime) = match mcp.transport {
        McpTransportKind::Stdio => (
            String::from("stdio"),
            String::from("per_invocation_stdio_session"),
        ),
        McpTransportKind::StreamableHttp => (
            String::from("streamable_http"),
            String::from("per_invocation_streamable_http_session"),
        ),
    };

    Some(PluginRuntimeSurface { transport, runtime })
}

pub fn render_catalog_list(
    output: &CatalogListOutput,
    filter: Option<CatalogFilterKind>,
) -> String {
    let mut lines = Vec::new();

    if matches!(filter, None | Some(CatalogFilterKind::BuiltinNode)) {
        lines.push(String::from("Builtin nodes"));
        if output.builtin_nodes.is_empty() {
            lines.push(String::from("  none"));
        } else {
            for entry in &output.builtin_nodes {
                lines.push(format!("  {:<38} {}", entry.reference, entry.summary));
            }
        }
    }

    if matches!(filter, None | Some(CatalogFilterKind::BuiltinTrigger)) {
        if !lines.is_empty() {
            lines.push(String::new());
        }
        lines.push(String::from("Builtin triggers"));
        if output.builtin_triggers.is_empty() {
            lines.push(String::from("  none"));
        } else {
            for entry in &output.builtin_triggers {
                lines.push(format!("  {:<38} {}", entry.reference, entry.summary));
            }
        }
    }

    if matches!(filter, None | Some(CatalogFilterKind::Plugin)) {
        if !lines.is_empty() {
            lines.push(String::new());
        }
        lines.push(String::from("Installed plugins"));
        if output.plugins.is_empty() {
            lines.push(String::from("  none"));
        } else {
            for entry in &output.plugins {
                let mut line = format!(
                    "  {:<32} {:<16} entrypoint={}",
                    entry.reference, entry.kind, entry.entrypoint
                );
                if let Some(transport) = entry.transport.as_deref() {
                    line.push_str(&format!(" transport={transport}"));
                }
                if let Some(runtime) = entry.runtime.as_deref() {
                    line.push_str(&format!(" runtime={runtime}"));
                }
                if !entry.surfaces.is_empty() {
                    line.push_str(&format!(" surfaces={}", entry.surfaces.join(" | ")));
                }
                lines.push(line);
            }
        }
    }
    lines.join("\n")
}

pub fn render_catalog_show(output: &CatalogShowOutput) -> String {
    let mut lines = vec![
        String::from("Catalog entry"),
        format!("  reference: {}", output.reference),
    ];
    match &output.detail {
        CatalogDetail::BuiltinNode(detail) => {
            lines.push(format!("  builtin_kind: {}", detail.builtin_kind));
            lines.push(format!("  summary: {}", detail.summary));
            if !detail.operations.is_empty() {
                lines.push(String::new());
                lines.push(String::from("Operations"));
                for operation in &detail.operations {
                    lines.push(format!("  - {operation}"));
                }
            }
            lines.push(String::new());
            lines.push(String::from("Inputs"));
            if detail.inputs.is_empty() {
                lines.push(String::from("  none"));
            } else {
                for field in &detail.inputs {
                    lines.push(format!("  - {}", field.name));
                }
            }
            lines.push(String::new());
            lines.push(String::from("Outputs"));
            if detail.outputs.is_empty() {
                lines.push(String::from("  none"));
            } else {
                for field in &detail.outputs {
                    lines.push(format!("  - {}", field.name));
                }
            }
        }
        CatalogDetail::BuiltinTrigger(detail) => {
            lines.push(format!("  source: {}", detail.source));
            lines.push(format!("  summary: {}", detail.summary));
            lines.push(format!("  mode: {}", detail.mode));
            lines.push(String::new());
            lines.push(String::from("Params"));
            if detail.params.is_empty() {
                lines.push(String::from("  none"));
            } else {
                for field in &detail.params {
                    lines.push(format!("  - {}", field.name));
                }
            }
            lines.push(String::new());
            lines.push(String::from("Payload"));
            if detail.payload.is_empty() {
                lines.push(String::from("  none"));
            } else {
                for field in &detail.payload {
                    lines.push(format!("  - {}", field.name));
                }
            }
        }
        CatalogDetail::Plugin(detail) => {
            lines.push(format!("  plugin_id: {}", detail.plugin_id));
            lines.push(format!("  plugin_kind: {}", detail.plugin_kind));
            lines.push(format!("  entrypoint: {}", detail.entrypoint));
            if let Some(transport) = detail.transport.as_deref() {
                lines.push(format!("  transport: {transport}"));
            }
            if let Some(runtime) = detail.runtime.as_deref() {
                lines.push(format!("  runtime: {runtime}"));
                if runtime == "per_invocation_stdio_session" {
                    lines.push(String::from(
                        "    MCP tool adapter: starts a stdio client session for each invocation",
                    ));
                } else if runtime == "per_invocation_streamable_http_session" {
                    lines.push(String::from(
                        "    MCP tool adapter: opens a Streamable HTTP client session for each invocation",
                    ));
                }
            }
            lines.push(format!("  schema_status: {}", detail.schema_status));
            if let Some(ref lifecycle) = detail.lifecycle {
                lines.push(format!("  lifecycle: {}", lifecycle));
                if lifecycle == "process_short_lived" {
                    lines.push(String::from(
                        "    Short-lived process adapter: poll-based, supervisor-orchestrated",
                    ));
                } else if lifecycle == "wasm_daemon_persistent_session" {
                    lines.push(String::from(
                        "    Daemon-persistent wasm session: long-lived Wasmtime ownership",
                    ));
                }
            }
            if !detail.capabilities.is_empty() {
                lines.push(format!(
                    "  capabilities: {}",
                    detail.capabilities.join(", ")
                ));
            }
            if !detail.surfaces.is_empty() {
                lines.push(format!("  surfaces: {}", detail.surfaces.join(" | ")));
            }
            if !detail.operations.is_empty() {
                lines.push(String::new());
                lines.push(String::from("Operations"));
                for operation in &detail.operations {
                    lines.push(format!("  {}", operation.name));
                    if let Some(summary) = &operation.summary {
                        lines.push(format!("    summary: {summary}"));
                    }
                    if !operation.input_schema.is_empty() {
                        lines.push(format!("    inputs: {}", operation.input_schema.join(", ")));
                    }
                    if !operation.output_schema.is_empty() {
                        lines.push(format!(
                            "    outputs: {}",
                            operation.output_schema.join(", ")
                        ));
                    }
                    let operation_kind = match operation.kind {
                        crate::plugin::PluginOperationKind::Generic => "generic",
                        crate::plugin::PluginOperationKind::Read => "read",
                        crate::plugin::PluginOperationKind::Write => "write",
                        crate::plugin::PluginOperationKind::Transfer => "transfer",
                        crate::plugin::PluginOperationKind::RawRead => "raw_read",
                        crate::plugin::PluginOperationKind::RawWrite => "raw_write",
                    };
                    lines.push(format!("    kind: {operation_kind}"));
                    if operation.requires_managed_signing {
                        lines.push(String::from("    managed_signing: required"));
                    }
                    if let Some(default_confirmation) = operation.default_confirmation.as_deref() {
                        lines.push(format!("    default_confirmation: {default_confirmation}"));
                    }
                }
            } else if !detail.input_schema.is_empty() || !detail.output_schema.is_empty() {
                lines.push(String::new());
                lines.push(String::from("Manifest schema"));
                if !detail.input_schema.is_empty() {
                    lines.push(format!("  inputs: {}", detail.input_schema.join(", ")));
                }
                if !detail.output_schema.is_empty() {
                    lines.push(format!("  outputs: {}", detail.output_schema.join(", ")));
                }
            }
            if let Some(event_schema) = &detail.event_schema {
                lines.push(String::new());
                lines.push(String::from("Event schema"));
                if let Some(summary) = &event_schema.summary {
                    lines.push(format!("  summary: {summary}"));
                }
                if event_schema.fields.is_empty() {
                    lines.push(String::from("  fields: none"));
                } else {
                    lines.push(format!("  fields: {}", event_schema.fields.join(", ")));
                }
                if !event_schema.listener_modes.is_empty() {
                    lines.push(format!(
                        "  listener_modes: {}",
                        event_schema
                            .listener_modes
                            .iter()
                            .map(|mode| match mode {
                                crate::plugin::PluginTriggerListenerMode::EventLog => "event_log",
                                crate::plugin::PluginTriggerListenerMode::StateChange => "state_change",
                            })
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
            }
            if detail.lifecycle.as_deref() == Some("wasm_daemon_persistent_session") {
                lines.push(String::new());
                lines.push(String::from("Host callback protocol"));
                lines.push(String::from(
                    "  daemon-persistent session with host callback push semantics",
                ));
                lines.push(String::from(
                    "  durable ack returned after store persist succeeds",
                ));
                lines.push(String::from("  host outcomes: retryable (queue_saturated/budget_exhausted) or terminal (lease_lost/shutting_down)"));
            } else if let Some(protocol) = &detail.protocol {
                lines.push(String::new());
                lines.push(String::from("Protocol"));
                lines.push(format!(
                    "  start_message: {}",
                    protocol.start_message.join(", ")
                ));
                lines.push(format!(
                    "  event_message: {}",
                    protocol.event_message.join(", ")
                ));
            }
        }
    }
    lines.join("\n")
}

fn plugin_surface_summary(manifest: &PluginManifest) -> Vec<String> {
    let mut summary = Vec::new();
    if matches!(manifest.plugin_id.as_str(), "eth-node" | "eth-trigger") {
        summary.push(String::from("chain=ethereum"));
    } else if matches!(manifest.plugin_id.as_str(), "solana-node" | "solana-trigger") {
        summary.push(String::from("chain=solana"));
    }

    if !manifest.operations.is_empty() {
        let operation_kinds = manifest
            .operations
            .iter()
            .map(|operation| match operation.kind {
                crate::plugin::PluginOperationKind::Generic => operation.name.clone(),
                crate::plugin::PluginOperationKind::Read => format!("{}:read", operation.name),
                crate::plugin::PluginOperationKind::Write => format!("{}:write", operation.name),
                crate::plugin::PluginOperationKind::Transfer => {
                    format!("{}:transfer", operation.name)
                }
                crate::plugin::PluginOperationKind::RawRead => {
                    format!("{}:raw_read", operation.name)
                }
                crate::plugin::PluginOperationKind::RawWrite => {
                    format!("{}:raw_write", operation.name)
                }
            })
            .collect::<Vec<_>>()
            .join(", ");
        summary.push(format!("operations={operation_kinds}"));
    }

    if let Some(event_schema) = manifest.event_schema.as_ref()
        && !event_schema.listener_modes.is_empty()
    {
        let listeners = event_schema
            .listener_modes
            .iter()
            .map(|mode| match mode {
                crate::plugin::PluginTriggerListenerMode::EventLog => "event_log",
                crate::plugin::PluginTriggerListenerMode::StateChange => "state_change",
            })
            .collect::<Vec<_>>()
            .join(", ");
        summary.push(format!("listeners={listeners}"));
    }

    summary
}
