//! [INPUT]
//! Builtin-trigger validation hooks, trigger manifest fields, path-backed package roots, and version-check helpers.
//!
//! [OUTPUT]
//! Defines validated trigger manifest contracts, trigger kinds, and helpers for builtin or external trigger interpretation.
//!
//! [ROLE]
//! Provides the backend-agnostic domain contract for trigger definitions.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::builtins::triggers::validate_builtin_trigger_definition;
use crate::errors::{assert_required_major, assert_supported_major, ContractError};
use crate::plugin::PluginActivationEnvelope;
use crate::secrets::SecretReference;

pub const CURRENT_API_MAJOR: u64 = 2;
pub const REQUIRED_TRIGGER_PLUGIN_CAPABILITY: &str = "trigger.listen.event";
pub const TRIGGER_KIND_BUILTIN: &str = "builtin";
pub const TRIGGER_KIND_EXTERNAL_PLUGIN: &str = "external_plugin";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerKind {
    Builtin,
    ExternalPlugin,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TriggerDefinition {
    #[serde(rename = "manifest_version")]
    pub api_version: String,
    pub trigger_id: String,
    pub kind: String,
    pub source: String,
    #[serde(default)]
    pub plugin: Option<String>,
    pub workflow_id: String,
    pub enabled: bool,
    #[serde(default)]
    pub params: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub input_mapping: BTreeMap<String, String>,
    #[serde(skip)]
    pub package_root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TriggerStartCommand {
    pub protocol_version: String,
    pub trigger_id: String,
    pub source: String,
    #[serde(default)]
    pub params: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub resume_checkpoint: Option<String>,
    #[serde(default)]
    pub activation: Option<PluginActivationEnvelope>,
    pub heartbeat_interval_ms: i64,
    pub shutdown_grace_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TriggerAck {
    pub checkpoint: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TriggerStop {
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TriggerHostMessage {
    Start(TriggerStartCommand),
    Ack(TriggerAck),
    Stop(TriggerStop),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TriggerReady {
    pub protocol_version: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TriggerEventFrame {
    pub checkpoint: String,
    pub event_key: String,
    pub occurred_at_ms: i64,
    #[serde(default)]
    pub payload: serde_json::Value,
    #[serde(default)]
    pub dedup_key: Option<String>,
    #[serde(default)]
    pub dedup_window_ms: Option<i64>,
    #[serde(default)]
    pub cooldown_key: Option<String>,
    #[serde(default)]
    pub cooldown_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TriggerHeartbeat {
    pub at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TriggerFatal {
    pub message: String,
    pub retryable: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TriggerPluginMessage {
    Ready(TriggerReady),
    Event(TriggerEventFrame),
    Heartbeat(TriggerHeartbeat),
    Fatal(TriggerFatal),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerPluginHostPolicy {
    pub allowlisted_plugin_ids: BTreeSet<String>,
    pub allowed_capabilities: BTreeSet<String>,
    pub plugin_root_dir: PathBuf,
    pub plugin_activation: BTreeMap<String, TriggerPluginActivationBindings>,
    pub secrets_root_dir: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TriggerPluginActivationBindings {
    pub secret_bindings: BTreeMap<String, SecretReference>,
    pub allowed_origins: Vec<String>,
}

impl TriggerDefinition {
    pub fn validate(&self) -> Result<(), ContractError> {
        assert_required_major(
            "trigger.manifest_version",
            &self.api_version,
            CURRENT_API_MAJOR,
        )?;
        validate_non_empty_field(&self.trigger_id, "trigger.trigger_id", "<unknown-trigger>")?;
        validate_non_empty_field(&self.source, "trigger.source", &self.trigger_id)?;
        validate_non_empty_field(&self.workflow_id, "trigger.workflow_id", &self.trigger_id)?;
        match self.kind()? {
            TriggerKind::Builtin => validate_builtin_trigger_definition(self)?,
            TriggerKind::ExternalPlugin => validate_non_empty_field(
                self.plugin.as_deref().unwrap_or_default(),
                "trigger.plugin",
                &self.trigger_id,
            )?,
        }
        Ok(())
    }

    pub fn kind(&self) -> Result<TriggerKind, ContractError> {
        match self.kind.as_str() {
            TRIGGER_KIND_BUILTIN => Ok(TriggerKind::Builtin),
            TRIGGER_KIND_EXTERNAL_PLUGIN => Ok(TriggerKind::ExternalPlugin),
            _ => Err(ContractError::UnknownTriggerKind {
                trigger_id: self.trigger_id.clone(),
                kind: self.kind.clone(),
            }),
        }
    }

    pub fn builtin_subtype(&self) -> Result<Option<&str>, ContractError> {
        match self.kind()? {
            TriggerKind::ExternalPlugin => Ok(None),
            TriggerKind::Builtin => {
                if self.kind == TRIGGER_KIND_BUILTIN {
                    Ok(Some(self.source.as_str()))
                } else {
                    Ok(Some(self.kind.as_str()))
                }
            }
        }
    }
}

impl TriggerStartCommand {
    pub fn validate(&self) -> Result<(), ContractError> {
        assert_supported_major(
            "trigger_start_command.protocol_version",
            &self.protocol_version,
            CURRENT_API_MAJOR,
        )?;
        if let Some(activation) = self.activation.as_ref() {
            for (slot, value) in &activation.secrets {
                validate_non_empty_field(
                    slot,
                    "trigger_start_command.activation.secrets",
                    &self.trigger_id,
                )?;
                validate_non_empty_field(
                    value,
                    "trigger_start_command.activation.secrets",
                    &self.trigger_id,
                )?;
            }
            for origin in &activation.allowed_origins {
                validate_non_empty_field(
                    origin,
                    "trigger_start_command.activation.allowed_origins",
                    &self.trigger_id,
                )?;
            }
        }
        Ok(())
    }
}

impl TriggerHostMessage {
    pub fn validate(&self) -> Result<(), ContractError> {
        match self {
            Self::Start(start) => start.validate(),
            Self::Ack(_) | Self::Stop(_) => Ok(()),
        }
    }
}

pub(crate) fn validate_non_empty_field(
    value: &str,
    field: &'static str,
    trigger_id: &str,
) -> Result<(), ContractError> {
    if value.trim().is_empty() {
        return Err(ContractError::InvalidTriggerDefinitionField {
            trigger_id: trigger_id.to_owned(),
            field,
            detail: "value cannot be empty".to_owned(),
        });
    }
    Ok(())
}
