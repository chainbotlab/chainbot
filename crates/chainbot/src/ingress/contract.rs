//! [INPUT]
//! Listener-backed trigger definitions, trigger params payloads, and runtime state/storage failure types.
//!
//! [OUTPUT]
//! Defines ingress transport params, normalized listener specs, and runtime error contracts shared by reconciliation and transport handlers.
//!
//! [ROLE]
//! Centralizes ingress-specific contracts so webhook and websocket listeners share one explicit schema.

use std::error::Error;
use std::fmt::{Display, Formatter};
use std::hash::{Hash, Hasher};

use serde::Deserialize;

use crate::domain::trigger::TriggerDefinition;
use crate::errors::ContractError;
use crate::infrastructure::state::RuntimeStateError;

pub const BUILTIN_TRIGGER_WEBHOOK_KIND: &str = "webhook";
pub const BUILTIN_TRIGGER_WEBSOCKET_KIND: &str = "websocket";

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum IngressAuthConfig {
    HeaderToken { header_name: String, token: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebhookTriggerParams {
    pub bind: String,
    pub path: String,
    pub method: String,
    #[serde(default)]
    pub auth: Option<IngressAuthConfig>,
    pub max_body_bytes: usize,
    #[serde(default)]
    pub content_type: Option<String>,
    #[serde(default)]
    pub idempotency_header: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebSocketTriggerParams {
    pub bind: String,
    pub path: String,
    #[serde(default)]
    pub auth: Option<IngressAuthConfig>,
    pub max_connections: usize,
    pub max_message_bytes: usize,
    #[serde(default)]
    pub idle_timeout_ms: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum IngressTransportKind {
    Webhook,
    WebSocket,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngressListenerSpec {
    pub trigger_id: String,
    pub workflow_id: String,
    pub transport: IngressTransportKind,
    pub bind: String,
    pub path: String,
    pub method: Option<String>,
    pub auth: Option<IngressAuthConfig>,
    pub max_body_bytes: Option<usize>,
    pub max_message_bytes: Option<usize>,
    pub max_connections: Option<usize>,
    pub idle_timeout_ms: Option<i64>,
    pub content_type: Option<String>,
    pub idempotency_header: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DesiredIngressState {
    pub listeners: Vec<IngressListenerSpec>,
}

#[derive(Debug)]
pub enum IngressRuntimeError {
    InvalidConfig(String),
    RuntimeState(RuntimeStateError),
    ListenerBind {
        bind: String,
        source: std::io::Error,
    },
    ControlChannelClosed,
    Runtime(String),
}

impl DesiredIngressState {
    pub fn fingerprint(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.listeners.hash(&mut hasher);
        hasher.finish()
    }
}

impl Hash for IngressAuthConfig {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self {
            Self::HeaderToken { header_name, token } => {
                "header_token".hash(state);
                header_name.hash(state);
                token.hash(state);
            }
        }
    }
}

impl Hash for IngressListenerSpec {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.trigger_id.hash(state);
        self.workflow_id.hash(state);
        self.transport.hash(state);
        self.bind.hash(state);
        self.path.hash(state);
        self.method.hash(state);
        self.auth.hash(state);
        self.max_body_bytes.hash(state);
        self.max_message_bytes.hash(state);
        self.max_connections.hash(state);
        self.idle_timeout_ms.hash(state);
        self.content_type.hash(state);
        self.idempotency_header.hash(state);
    }
}

pub fn decode_webhook_params(
    definition: &TriggerDefinition,
) -> Result<WebhookTriggerParams, ContractError> {
    let params: WebhookTriggerParams = decode_params(definition)?;
    validate_non_empty(definition, "trigger.params.bind", params.bind.trim())?;
    validate_non_empty(definition, "trigger.params.path", params.path.trim())?;
    validate_non_empty(definition, "trigger.params.method", params.method.trim())?;
    if params.max_body_bytes == 0 {
        return Err(invalid_field(
            definition,
            "trigger.params.max_body_bytes",
            "value must be greater than 0",
        ));
    }
    validate_auth(definition, params.auth.as_ref())?;
    if let Some(value) = params.content_type.as_deref() {
        validate_non_empty(definition, "trigger.params.content_type", value.trim())?;
    }
    if let Some(value) = params.idempotency_header.as_deref() {
        validate_non_empty(
            definition,
            "trigger.params.idempotency_header",
            value.trim(),
        )?;
    }
    Ok(params)
}

pub fn decode_websocket_params(
    definition: &TriggerDefinition,
) -> Result<WebSocketTriggerParams, ContractError> {
    let params: WebSocketTriggerParams = decode_params(definition)?;
    validate_non_empty(definition, "trigger.params.bind", params.bind.trim())?;
    validate_non_empty(definition, "trigger.params.path", params.path.trim())?;
    if params.max_connections == 0 {
        return Err(invalid_field(
            definition,
            "trigger.params.max_connections",
            "value must be greater than 0",
        ));
    }
    if params.max_message_bytes == 0 {
        return Err(invalid_field(
            definition,
            "trigger.params.max_message_bytes",
            "value must be greater than 0",
        ));
    }
    if let Some(value) = params.idle_timeout_ms {
        if value <= 0 {
            return Err(invalid_field(
                definition,
                "trigger.params.idle_timeout_ms",
                "value must be greater than 0 when set",
            ));
        }
    }
    validate_auth(definition, params.auth.as_ref())?;
    Ok(params)
}

pub fn normalize_bind(value: &str) -> String {
    value.trim().to_owned()
}

pub fn normalize_path(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return String::from("/");
    }
    let with_leading = if trimmed.starts_with('/') {
        trimmed.to_owned()
    } else {
        format!("/{trimmed}")
    };
    if with_leading.len() > 1 {
        with_leading.trim_end_matches('/').to_owned()
    } else {
        with_leading
    }
}

pub fn normalize_method(value: &str) -> String {
    value.trim().to_ascii_uppercase()
}

fn decode_params<T>(definition: &TriggerDefinition) -> Result<T, ContractError>
where
    T: for<'de> Deserialize<'de>,
{
    serde_json::from_value(serde_json::to_value(&definition.params).map_err(|source| {
        ContractError::InvalidTriggerDefinitionField {
            trigger_id: definition.trigger_id.clone(),
            field: "trigger.params",
            detail: format!("failed to serialize trigger params: {source}"),
        }
    })?)
    .map_err(|source| ContractError::InvalidTriggerDefinitionField {
        trigger_id: definition.trigger_id.clone(),
        field: "trigger.params",
        detail: source.to_string(),
    })
}

fn validate_non_empty(
    definition: &TriggerDefinition,
    field: &'static str,
    value: &str,
) -> Result<(), ContractError> {
    if value.is_empty() {
        return Err(invalid_field(definition, field, "value cannot be empty"));
    }
    Ok(())
}

fn validate_auth(
    definition: &TriggerDefinition,
    auth: Option<&IngressAuthConfig>,
) -> Result<(), ContractError> {
    match auth {
        None => Ok(()),
        Some(IngressAuthConfig::HeaderToken { header_name, token }) => {
            validate_non_empty(
                definition,
                "trigger.params.auth.header_name",
                header_name.trim(),
            )?;
            validate_non_empty(definition, "trigger.params.auth.token", token.trim())
        }
    }
}

fn invalid_field(
    definition: &TriggerDefinition,
    field: &'static str,
    detail: impl Into<String>,
) -> ContractError {
    ContractError::InvalidTriggerDefinitionField {
        trigger_id: definition.trigger_id.clone(),
        field,
        detail: detail.into(),
    }
}

impl Display for IngressRuntimeError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidConfig(detail) => write!(f, "invalid ingress config: {detail}"),
            Self::RuntimeState(source) => write!(f, "ingress runtime-state error: {source}"),
            Self::ListenerBind { bind, source } => {
                write!(f, "failed to bind ingress listener at {bind}: {source}")
            }
            Self::ControlChannelClosed => write!(f, "ingress supervisor control channel closed"),
            Self::Runtime(detail) => write!(f, "ingress runtime error: {detail}"),
        }
    }
}

impl Error for IngressRuntimeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::RuntimeState(source) => Some(source),
            Self::ListenerBind { source, .. } => Some(source),
            Self::InvalidConfig(_) | Self::ControlChannelClosed | Self::Runtime(_) => None,
        }
    }
}

impl From<RuntimeStateError> for IngressRuntimeError {
    fn from(value: RuntimeStateError) -> Self {
        Self::RuntimeState(value)
    }
}
