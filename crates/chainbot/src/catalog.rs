//! [INPUT]
//! Builtin node/trigger descriptors, loaded plugin manifests, and CLI-facing filtering or reference parsing requests.
//!
//! [OUTPUT]
//! Builds stable catalog list/show read models plus human-readable renderers for builtin and plugin discoverability.
//!
//! [ROLE]
//! Owns the CLI capability-discovery surface independently from runtime registry internals.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::builtins::nodes::catalog as node_catalog;
use crate::builtins::triggers::catalog as trigger_catalog;
use crate::plugin::{
    PluginEventSchemaDescriptor, PluginKind, PluginManifest, PluginOperationDescriptor,
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
    pub capabilities: Vec<String>,
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
    pub capabilities: Vec<String>,
    pub schema_status: String,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StatusPluginSummaryView {
    pub installed_count: usize,
    pub builtin_count: usize,
    pub external_node_count: usize,
    pub external_trigger_count: usize,
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
                .map(|manifest| PluginSummary {
                    reference: format!("plugin:{}", manifest.plugin_id),
                    plugin_id: manifest.plugin_id.clone(),
                    kind: manifest.kind.clone(),
                    entrypoint: manifest.entrypoint.clone(),
                    capabilities: manifest.capabilities.clone(),
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
    }
}

fn build_plugin_detail(manifest: &PluginManifest) -> Result<PluginDetail, String> {
    let plugin_kind = manifest.kind().map_err(|error| error.to_string())?;
    match plugin_kind {
        PluginKind::Builtin => Ok(PluginDetail {
            plugin_id: manifest.plugin_id.clone(),
            plugin_kind: manifest.kind.clone(),
            entrypoint: manifest.entrypoint.clone(),
            capabilities: manifest.capabilities.clone(),
            schema_status: String::from("manifest_only"),
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
            capabilities: manifest.capabilities.clone(),
            schema_status: String::from("declared"),
            operations: manifest.operations.clone(),
            event_schema: None,
            protocol: None,
            input_schema: Vec::new(),
            output_schema: Vec::new(),
        }),
        PluginKind::ExternalTrigger => Ok(PluginDetail {
            plugin_id: manifest.plugin_id.clone(),
            plugin_kind: manifest.kind.clone(),
            entrypoint: manifest.entrypoint.clone(),
            capabilities: manifest.capabilities.clone(),
            schema_status: String::from("declared"),
            operations: Vec::new(),
            event_schema: manifest.event_schema.clone(),
            protocol: Some(PluginProtocolDetail {
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
            }),
            input_schema: Vec::new(),
            output_schema: Vec::new(),
        }),
    }
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
                lines.push(format!(
                    "  {:<32} {:<16} entrypoint={}",
                    entry.reference, entry.kind, entry.entrypoint
                ));
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
            lines.push(format!("  schema_status: {}", detail.schema_status));
            if !detail.capabilities.is_empty() {
                lines.push(format!(
                    "  capabilities: {}",
                    detail.capabilities.join(", ")
                ));
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
            }
            if let Some(protocol) = &detail.protocol {
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
