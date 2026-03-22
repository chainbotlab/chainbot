//! [INPUT]
//! Version parsing, contract validation failures, DAG and namespace checks, runtime host failures, and CLI presentation requirements.
//!
//! [OUTPUT]
//! Defines typed contract and runtime errors plus stable CLI-facing exit-code mapping.
//!
//! [ROLE]
//! Centralizes failure taxonomy shared across the crate's validation, execution, and command surfaces.

use std::error::Error;
use std::fmt::{Display, Formatter};
use std::ops::Range;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliExitCode {
    Success = 0,
    Usage = 2,
    Validation = 3,
    State = 4,
    Unavailable = 5,
    Conflict = 6,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserFacingError {
    Usage { message: String },
    Validation { message: String },
    State { message: String },
    Unavailable { message: String },
    Conflict { message: String },
}

#[derive(Debug)]
pub enum ContractError {
    JsonDecode(serde_json::Error),
    TomlDecode {
        path: PathBuf,
        context: Option<TomlParseContext>,
        source: toml::de::Error,
    },
    TomlEncode {
        path: PathBuf,
        source: toml::ser::Error,
    },
    Io {
        path: PathBuf,
        operation: &'static str,
        source: std::io::Error,
    },
    MissingHomeDirectory,
    MissingFile {
        path: PathBuf,
        kind: &'static str,
    },
    MissingDirectory {
        path: PathBuf,
        kind: &'static str,
    },
    CliUsage {
        message: String,
    },
    InvalidRootConfigField {
        field: &'static str,
        detail: String,
    },
    InvalidVersionFormat {
        field: &'static str,
        value: String,
    },
    UnsupportedFutureMajorVersion {
        field: &'static str,
        major: u64,
        max_supported_major: u64,
    },
    UnsupportedMajorVersion {
        field: &'static str,
        major: u64,
        supported_major: u64,
    },
    InvalidSecretReferenceSyntax {
        value: String,
    },
    SecretFileNotFound {
        reference: String,
        path: PathBuf,
    },
    SecretDecryptFailed {
        reference: String,
        path: PathBuf,
    },
    SecretKeyNotFound {
        reference: String,
        key: String,
    },
    SecretPayloadEmpty {
        reference: String,
    },
    UnknownBuiltinNodeKind {
        workflow_id: String,
        node_id: String,
        kind: String,
    },
    DuplicateWorkflowId {
        workflow_id: String,
    },
    DuplicatePluginId {
        plugin_id: String,
    },
    UnknownWorkflowDefinition {
        workflow_id: String,
    },
    TriggerReferencesUnknownWorkflow {
        trigger_id: String,
        workflow_id: String,
    },
    PackageDirectoryIdentityMismatch {
        kind: &'static str,
        path: PathBuf,
        expected_id: String,
        directory_name: String,
    },
    SubflowDepthExceeded {
        workflow_id: String,
        max_depth: usize,
    },
    SubflowCycleDetected {
        workflow_chain: Vec<String>,
    },
    SchedulerStalled {
        workflow_id: String,
        blocked_node_ids: Vec<String>,
    },
    UnsupportedNodeKindForScheduler {
        workflow_id: String,
        node_id: String,
        kind: String,
    },
    SubflowExecutionFailed {
        workflow_id: String,
        node_id: String,
        child_workflow_id: String,
    },
    DuplicateNodeId {
        workflow_id: String,
        node_id: String,
    },
    UnknownNodeDependency {
        workflow_id: String,
        node_id: String,
        dependency_id: String,
    },
    MissingDagNodeIndex {
        workflow_id: String,
        node_id: String,
    },
    SelfDependency {
        workflow_id: String,
        node_id: String,
    },
    DagCycleDetected {
        workflow_id: String,
        node_ids: Vec<String>,
    },
    InvalidVariableReference {
        workflow_id: String,
        node_id: String,
        context: &'static str,
        namespace: String,
        key: String,
    },
    InvalidSubflowContract {
        workflow_id: String,
        node_id: String,
        detail: String,
    },
    UnexpectedSubflowNodeInputs {
        workflow_id: String,
        node_id: String,
    },
    MissingSubflowContract {
        workflow_id: String,
        node_id: String,
    },
    UnexpectedSubflowContract {
        workflow_id: String,
        node_id: String,
    },
    WorkerRequestIdMismatch {
        expected: String,
        actual: String,
    },
    TriggerPluginNotAllowlisted {
        plugin_id: String,
    },
    TriggerPluginInvalidKind {
        plugin_id: String,
        kind: String,
    },
    TriggerPluginMissingCapability {
        plugin_id: String,
        capability: String,
    },
    TriggerPluginCapabilityNotAllowed {
        plugin_id: String,
        capability: String,
    },
    TriggerPluginEntrypointMustBeRelative {
        plugin_id: String,
        entrypoint: String,
    },
    TriggerPluginEntrypointEscapesRoot {
        plugin_id: String,
        entrypoint: String,
        root: PathBuf,
    },
    TriggerPluginExecutableMissing {
        plugin_id: String,
        path: PathBuf,
    },
    TriggerPluginExecutableNotFile {
        plugin_id: String,
        path: PathBuf,
    },
    TriggerPluginExecutableNotExecutable {
        plugin_id: String,
        path: PathBuf,
    },
    TriggerPluginSpawnFailed {
        plugin_id: String,
        path: PathBuf,
        source: std::io::Error,
    },
    TriggerPluginProcessIo {
        plugin_id: String,
        operation: &'static str,
        source: std::io::Error,
    },
    TriggerPluginProcessFailed {
        plugin_id: String,
        status: i32,
        stderr: String,
    },
    TriggerPluginProtocolEncode {
        plugin_id: String,
        source: serde_json::Error,
    },
    TriggerPluginOutputDecode {
        plugin_id: String,
        source: serde_json::Error,
    },
    TriggerPluginProtocolContractViolation {
        plugin_id: String,
        detail: String,
    },
    TriggerPluginReturnedFailure {
        plugin_id: String,
        message: String,
    },
    NodePluginInvalidField {
        plugin_id: String,
        field: &'static str,
        detail: String,
    },
    NodePluginInvalidKind {
        plugin_id: String,
        kind: String,
    },
    NodePluginMissingExecutable {
        plugin_id: String,
    },
    NodePluginInvalidExecutablePath {
        plugin_id: String,
        executable: String,
        detail: String,
    },
    NodePluginCapabilityNotDeclared {
        plugin_id: String,
        capability: String,
    },
    NodePluginInputSchemaMismatch {
        plugin_id: String,
        detail: String,
    },
    NodePluginOutputSchemaMismatch {
        plugin_id: String,
        detail: String,
    },
    NodePluginSpawnFailed {
        plugin_id: String,
        executable: PathBuf,
        source: std::io::Error,
    },
    NodePluginProcessIo {
        plugin_id: String,
        operation: &'static str,
        source: std::io::Error,
    },
    NodePluginProcessFailed {
        plugin_id: String,
        exit_code: Option<i32>,
        stderr: String,
    },
    NodePluginProtocolEncode {
        plugin_id: String,
        source: serde_json::Error,
    },
    NodePluginProtocolDecode {
        plugin_id: String,
        source: serde_json::Error,
    },
    NodePluginProtocolContractViolation {
        plugin_id: String,
        detail: String,
    },
    NodePluginReturnedFailure {
        plugin_id: String,
        message: String,
    },
    UnknownTriggerPlugin {
        trigger_id: String,
        plugin_id: String,
    },
    DuplicateTriggerId {
        trigger_id: String,
    },
    InvalidTriggerDefinitionField {
        trigger_id: String,
        field: &'static str,
        detail: String,
    },
    UnknownTriggerKind {
        trigger_id: String,
        kind: String,
    },
    InvalidTriggerEmission {
        trigger_id: String,
        detail: String,
    },
}

#[derive(Debug)]
pub struct TomlParseContext {
    line: usize,
    column: usize,
    snippet: String,
    pointer: String,
}

impl CliExitCode {
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Success => 0,
            Self::Usage => 2,
            Self::Validation => 3,
            Self::State => 4,
            Self::Unavailable => 5,
            Self::Conflict => 6,
        }
    }
}

impl UserFacingError {
    pub fn usage(message: impl Into<String>) -> Self {
        Self::Usage {
            message: message.into(),
        }
    }

    pub fn validation(message: impl Into<String>) -> Self {
        Self::Validation {
            message: message.into(),
        }
    }

    pub fn state(message: impl Into<String>) -> Self {
        Self::State {
            message: message.into(),
        }
    }

    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::Unavailable {
            message: message.into(),
        }
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::Conflict {
            message: message.into(),
        }
    }

    pub fn exit_code(&self) -> CliExitCode {
        match self {
            Self::Usage { .. } => CliExitCode::Usage,
            Self::Validation { .. } => CliExitCode::Validation,
            Self::State { .. } => CliExitCode::State,
            Self::Unavailable { .. } => CliExitCode::Unavailable,
            Self::Conflict { .. } => CliExitCode::Conflict,
        }
    }

    pub fn from_contract(error: ContractError) -> Self {
        match error {
            ContractError::CliUsage { message } => Self::usage(message),
            ContractError::MissingHomeDirectory => Self::validation(
                "Default root could not be resolved because neither CHAINBOT_CONFIG_DIR nor HOME is set.",
            ),
            ContractError::MissingFile { path, kind } => {
                let mut message = format!("Root is missing required {kind} file: {}", path.display());
                if kind == "root config" {
                    message.push_str(
                        " Check CHAINBOT_CONFIG_DIR or ensure ~/.chainbot/chainbot.toml exists (or migrate legacy ~/.chainbot/config/root.toml).",
                    );
                }
                Self::validation(message)
            }
            ContractError::MissingDirectory { path, kind } => {
                let mut message = format!(
                    "Root is missing required {kind} directory: {}",
                    path.display()
                );
                if kind == "root" {
                    message.push_str(
                        " Check CHAINBOT_CONFIG_DIR or ensure ~/.chainbot exists and is a valid root.",
                    );
                }
                Self::validation(message)
            }
            ContractError::Io {
                path,
                operation,
                source,
            } => Self::state(format!(
                "Failed to {operation} at {}: {source}",
                path.display()
            )),
            error @ ContractError::TomlDecode { .. } => Self::validation(error.to_string()),
            ContractError::TomlEncode { path, source } => Self::state(format!(
                "Definition file could not be serialized at {}: {source}",
                path.display()
            )),
            ContractError::JsonDecode(source) => {
                Self::validation(format!("Definition JSON is invalid: {source}"))
            }
            err @ (ContractError::SecretFileNotFound { .. }
            | ContractError::SecretDecryptFailed { .. }
            | ContractError::SecretKeyNotFound { .. }
            | ContractError::SecretPayloadEmpty { .. }) => Self::unavailable(err.to_string()),
            other => Self::validation(other.to_string()),
        }
    }
}

impl Display for ContractError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::JsonDecode(err) => write!(f, "failed to decode contract json: {err}"),
            Self::TomlDecode {
                path,
                context,
                source,
            } => {
                writeln!(f, "definition file is invalid TOML")?;
                writeln!(f, "  file: {}", path.display())?;
                writeln!(f, "  message: {}", source.message())?;
                if let Some(context) = context {
                    writeln!(f, "  line: {}, column: {}", context.line, context.column)?;
                    let gutter = context.line.to_string().len();
                    writeln!(f)?;
                    writeln!(f, "{:>width$} | {}", context.line, context.snippet, width = gutter)?;
                    write!(f, "{} | {}", " ".repeat(gutter), context.pointer)?;
                }
                Ok(())
            }
            Self::TomlEncode { path, source } => {
                write!(f, "failed to encode TOML at {}: {source}", path.display())
            }
            Self::Io {
                path,
                operation,
                source,
            } => write!(
                f,
                "failed to {operation} at {}: {source}",
                path.display()
            ),
            Self::MissingHomeDirectory => {
                write!(f, "HOME is not set; cannot resolve default root ~/.chainbot")
            }
            Self::MissingFile { path, kind } => {
                write!(f, "missing required {kind} file: {}", path.display())
            }
            Self::MissingDirectory { path, kind } => {
                write!(f, "missing required {kind} directory: {}", path.display())
            }
            Self::CliUsage { message } => write!(f, "{message}"),
            Self::InvalidRootConfigField { field, detail } => {
                write!(f, "root config has invalid field {field}: {detail}")
            }
            Self::InvalidVersionFormat { field, value } => {
                write!(f, "{field} must start with a numeric major version: {value}")
            }
            Self::UnsupportedFutureMajorVersion {
                field,
                major,
                max_supported_major,
            } => write!(
                f,
                "{field} major version {major} is unsupported; max supported major is {max_supported_major}"
            ),
            Self::UnsupportedMajorVersion {
                field,
                major,
                supported_major,
            } => write!(
                f,
                "{field} major version {major} is unsupported; required major is {supported_major}"
            ),
            Self::InvalidSecretReferenceSyntax { value } => {
                write!(f, "invalid secret reference syntax: {value}")
            }
            Self::SecretFileNotFound { reference, path } => write!(
                f,
                "secret reference {reference} did not resolve to an encrypted file at {}",
                path.display()
            ),
            Self::SecretDecryptFailed { reference, path } => write!(
                f,
                "secret decryption failed for reference {reference} at {} (plaintext redacted)",
                path.display()
            ),
            Self::SecretKeyNotFound { reference, key } => {
                write!(f, "secret reference {reference} is missing key {key}")
            }
            Self::SecretPayloadEmpty { reference } => {
                write!(f, "secret reference {reference} resolved to an empty payload")
            }
            Self::UnknownBuiltinNodeKind {
                workflow_id,
                node_id,
                kind,
            } => write!(
                f,
                "workflow {workflow_id} node {node_id} references unknown builtin kind {kind}"
            ),
            Self::DuplicateWorkflowId { workflow_id } => {
                write!(f, "duplicate workflow_id detected: {workflow_id}")
            }
            Self::DuplicatePluginId { plugin_id } => {
                write!(f, "duplicate plugin_id detected: {plugin_id}")
            }
            Self::UnknownWorkflowDefinition { workflow_id } => {
                write!(f, "unknown workflow definition: {workflow_id}")
            }
            Self::TriggerReferencesUnknownWorkflow {
                trigger_id,
                workflow_id,
            } => write!(
                f,
                "trigger {trigger_id} references unknown workflow definition {workflow_id}"
            ),
            Self::PackageDirectoryIdentityMismatch {
                kind,
                path,
                expected_id,
                directory_name,
            } => write!(
                f,
                "{kind} package at {} must use directory name {expected_id}, found {directory_name}",
                path.display()
            ),
            Self::SubflowDepthExceeded {
                workflow_id,
                max_depth,
            } => write!(
                f,
                "workflow {workflow_id} exceeded max subflow depth {max_depth}"
            ),
            Self::SubflowCycleDetected { workflow_chain } => write!(
                f,
                "subflow cycle detected: {}",
                workflow_chain.join(" -> ")
            ),
            Self::SchedulerStalled {
                workflow_id,
                blocked_node_ids,
            } => write!(
                f,
                "workflow {workflow_id} scheduler stalled with blocked nodes: {}",
                blocked_node_ids.join(", ")
            ),
            Self::UnsupportedNodeKindForScheduler {
                workflow_id,
                node_id,
                kind,
            } => write!(
                f,
                "workflow {workflow_id} node {node_id} has unsupported scheduler kind {kind}"
            ),
            Self::SubflowExecutionFailed {
                workflow_id,
                node_id,
                child_workflow_id,
            } => write!(
                f,
                "workflow {workflow_id} node {node_id} failed while executing child workflow {child_workflow_id}"
            ),
            Self::UnexpectedSubflowNodeInputs {
                workflow_id,
                node_id,
            } => write!(
                f,
                "workflow {workflow_id} node {node_id} must not define node.inputs; subflow nodes only accept inputs through nodes.call.with"
            ),
            Self::DuplicateNodeId {
                workflow_id,
                node_id,
            } => write!(
                f,
                "workflow {workflow_id} contains duplicate node_id {node_id}"
            ),
            Self::UnknownNodeDependency {
                workflow_id,
                node_id,
                dependency_id,
            } => write!(
                f,
                "workflow {workflow_id} node {node_id} depends on unknown node {dependency_id}"
            ),
            Self::MissingDagNodeIndex {
                workflow_id,
                node_id,
            } => write!(
                f,
                "workflow {workflow_id} node {node_id} is missing in internal DAG index"
            ),
            Self::SelfDependency {
                workflow_id,
                node_id,
            } => write!(
                f,
                "workflow {workflow_id} node {node_id} cannot depend on itself"
            ),
            Self::DagCycleDetected {
                workflow_id,
                node_ids,
            } => write!(
                f,
                "workflow {workflow_id} contains a dependency cycle involving nodes: {}",
                node_ids.join(", ")
            ),
            Self::InvalidVariableReference {
                workflow_id,
                node_id,
                context,
                namespace,
                key,
            } => write!(
                f,
                "workflow {workflow_id} node {node_id} has invalid {context} reference {namespace}:{key}"
            ),
            Self::InvalidSubflowContract {
                workflow_id,
                node_id,
                detail,
            } => write!(
                f,
                "workflow {workflow_id} node {node_id} has invalid subflow contract: {detail}"
            ),
            Self::MissingSubflowContract {
                workflow_id,
                node_id,
            } => write!(
                f,
                "workflow {workflow_id} node {node_id} has kind=subflow but no subflow contract"
            ),
            Self::UnexpectedSubflowContract {
                workflow_id,
                node_id,
            } => write!(
                f,
                "workflow {workflow_id} node {node_id} has a subflow contract but kind is not subflow"
            ),
            Self::WorkerRequestIdMismatch { expected, actual } => write!(
                f,
                "worker response request_id mismatch: expected {expected}, got {actual}"
            ),
            Self::TriggerPluginNotAllowlisted { plugin_id } => {
                write!(f, "trigger plugin {plugin_id} is not allowlisted")
            }
            Self::TriggerPluginInvalidKind { plugin_id, kind } => {
                write!(f, "trigger plugin {plugin_id} has unsupported kind {kind}")
            }
            Self::TriggerPluginMissingCapability {
                plugin_id,
                capability,
            } => write!(
                f,
                "trigger plugin {plugin_id} is missing required capability {capability}"
            ),
            Self::TriggerPluginCapabilityNotAllowed {
                plugin_id,
                capability,
            } => write!(
                f,
                "trigger plugin {plugin_id} requested non-allowlisted capability {capability}"
            ),
            Self::TriggerPluginEntrypointMustBeRelative {
                plugin_id,
                entrypoint,
            } => write!(
                f,
                "trigger plugin {plugin_id} entrypoint must be relative to plugin root: {entrypoint}"
            ),
            Self::TriggerPluginEntrypointEscapesRoot {
                plugin_id,
                entrypoint,
                root,
            } => write!(
                f,
                "trigger plugin {plugin_id} entrypoint {entrypoint} escapes plugin root {}",
                root.display()
            ),
            Self::TriggerPluginExecutableMissing { plugin_id, path } => write!(
                f,
                "trigger plugin {plugin_id} executable does not exist: {}",
                path.display()
            ),
            Self::TriggerPluginExecutableNotFile { plugin_id, path } => write!(
                f,
                "trigger plugin {plugin_id} executable path is not a file: {}",
                path.display()
            ),
            Self::TriggerPluginExecutableNotExecutable { plugin_id, path } => write!(
                f,
                "trigger plugin {plugin_id} executable path is not executable: {}",
                path.display()
            ),
            Self::TriggerPluginSpawnFailed {
                plugin_id,
                path,
                source,
            } => write!(
                f,
                "failed to spawn trigger plugin {plugin_id} at {}: {source}",
                path.display()
            ),
            Self::TriggerPluginProcessIo {
                plugin_id,
                operation,
                source,
            } => write!(f, "trigger plugin {plugin_id} failed to {operation}: {source}"),
            Self::TriggerPluginProcessFailed {
                plugin_id,
                status,
                stderr,
            } => write!(
                f,
                "trigger plugin {plugin_id} exited with status {status}: {stderr}"
            ),
            Self::TriggerPluginProtocolEncode { plugin_id, source } => write!(
                f,
                "failed to encode trigger plugin {plugin_id} input JSON: {source}"
            ),
            Self::TriggerPluginOutputDecode { plugin_id, source } => write!(
                f,
                "failed to decode trigger plugin {plugin_id} output JSON: {source}"
            ),
            Self::TriggerPluginProtocolContractViolation { plugin_id, detail } => write!(
                f,
                "trigger plugin {plugin_id} protocol contract violation: {detail}"
            ),
            Self::TriggerPluginReturnedFailure { plugin_id, message } => {
                write!(f, "trigger plugin {plugin_id} returned failure: {message}")
            }
            Self::NodePluginInvalidField {
                plugin_id,
                field,
                detail,
            } => write!(
                f,
                "node plugin {plugin_id} has invalid field {field}: {detail}"
            ),
            Self::NodePluginInvalidKind { plugin_id, kind } => {
                write!(f, "node plugin {plugin_id} has unsupported kind {kind}")
            }
            Self::NodePluginMissingExecutable { plugin_id } => {
                write!(f, "node plugin {plugin_id} is missing executable")
            }
            Self::NodePluginInvalidExecutablePath {
                plugin_id,
                executable,
                detail,
            } => write!(
                f,
                "node plugin {plugin_id} has invalid executable path {executable}: {detail}"
            ),
            Self::NodePluginCapabilityNotDeclared {
                plugin_id,
                capability,
            } => write!(
                f,
                "node plugin {plugin_id} requested undeclared capability {capability}"
            ),
            Self::NodePluginInputSchemaMismatch { plugin_id, detail } => {
                write!(f, "node plugin {plugin_id} input schema mismatch: {detail}")
            }
            Self::NodePluginOutputSchemaMismatch { plugin_id, detail } => {
                write!(f, "node plugin {plugin_id} output schema mismatch: {detail}")
            }
            Self::NodePluginSpawnFailed {
                plugin_id,
                executable,
                source,
            } => write!(
                f,
                "failed to spawn node plugin {plugin_id} at {}: {source}",
                executable.display()
            ),
            Self::NodePluginProcessIo {
                plugin_id,
                operation,
                source,
            } => write!(
                f,
                "failed to {operation} for node plugin {plugin_id}: {source}"
            ),
            Self::NodePluginProcessFailed {
                plugin_id,
                exit_code,
                stderr,
            } => write!(
                f,
                "node plugin {plugin_id} exited with status {:?}: {}",
                exit_code,
                stderr.trim()
            ),
            Self::NodePluginProtocolEncode { plugin_id, source } => write!(
                f,
                "failed to encode request for node plugin {plugin_id}: {source}"
            ),
            Self::NodePluginProtocolDecode { plugin_id, source } => write!(
                f,
                "failed to decode response from node plugin {plugin_id}: {source}"
            ),
            Self::NodePluginProtocolContractViolation { plugin_id, detail } => {
                write!(f, "node plugin {plugin_id} protocol contract violation: {detail}")
            }
            Self::NodePluginReturnedFailure { plugin_id, message } => {
                write!(f, "node plugin {plugin_id} returned failure: {message}")
            }
            Self::UnknownTriggerPlugin {
                trigger_id,
                plugin_id,
            } => write!(
                f,
                "trigger {trigger_id} references unknown trigger plugin {plugin_id}"
            ),
            Self::DuplicateTriggerId { trigger_id } => {
                write!(f, "duplicate trigger_id detected: {trigger_id}")
            }
            Self::InvalidTriggerDefinitionField {
                trigger_id,
                field,
                detail,
            } => write!(
                f,
                "trigger {trigger_id} has invalid field {field}: {detail}"
            ),
            Self::UnknownTriggerKind { trigger_id, kind } => {
                write!(f, "trigger {trigger_id} has unsupported kind {kind}")
            }
            Self::InvalidTriggerEmission { trigger_id, detail } => {
                write!(f, "trigger {trigger_id} emitted invalid event: {detail}")
            }
        }
    }
}

impl Display for UserFacingError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Usage { message }
            | Self::Validation { message }
            | Self::State { message }
            | Self::Unavailable { message }
            | Self::Conflict { message } => write!(f, "{message}"),
        }
    }
}

impl Error for UserFacingError {}

impl Error for ContractError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::JsonDecode(err) => Some(err),
            Self::TomlDecode { source, .. } => Some(source),
            Self::Io { source, .. } => Some(source),
            Self::TriggerPluginSpawnFailed { source, .. } => Some(source),
            Self::TriggerPluginProcessIo { source, .. } => Some(source),
            Self::TriggerPluginProtocolEncode { source, .. } => Some(source),
            Self::TriggerPluginOutputDecode { source, .. } => Some(source),
            Self::NodePluginSpawnFailed { source, .. } => Some(source),
            Self::NodePluginProcessIo { source, .. } => Some(source),
            Self::NodePluginProtocolEncode { source, .. } => Some(source),
            Self::NodePluginProtocolDecode { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl From<serde_json::Error> for ContractError {
    fn from(value: serde_json::Error) -> Self {
        Self::JsonDecode(value)
    }
}

impl ContractError {
    pub fn toml_decode(path: PathBuf, input: &str, source: toml::de::Error) -> Self {
        let context = build_toml_parse_context(input, source.span());
        Self::TomlDecode {
            path,
            context,
            source,
        }
    }
}

fn build_toml_parse_context(input: &str, span: Option<Range<usize>>) -> Option<TomlParseContext> {
    let span = span?;
    if input.is_empty() {
        return None;
    }

    let last_index = input.len().saturating_sub(1);
    let safe_start = span.start.min(last_index);
    let line_start = input[..safe_start]
        .rfind('\n')
        .map(|index| index + 1)
        .unwrap_or(0);
    let line_end = input[safe_start..]
        .find('\n')
        .map(|offset| safe_start + offset)
        .unwrap_or(input.len());
    let snippet = input[line_start..line_end].to_owned();
    let line = input[..line_start]
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        + 1;
    let column_offset = input[line_start..safe_start].chars().count();
    let snippet_width = snippet.chars().count();
    let highlight_len = span.end.saturating_sub(span.start).max(1);
    let available_width = snippet_width.saturating_sub(column_offset).max(1);
    let pointer = format!(
        "{}{}",
        " ".repeat(column_offset),
        "^".repeat(highlight_len.min(available_width).max(1))
    );

    Some(TomlParseContext {
        line,
        column: column_offset + 1,
        snippet,
        pointer,
    })
}

pub fn assert_supported_major(
    field: &'static str,
    version: &str,
    max_supported_major: u64,
) -> Result<(), ContractError> {
    let major_segment = version.split('.').next().unwrap_or_default().trim();
    let major = major_segment
        .parse::<u64>()
        .map_err(|_| ContractError::InvalidVersionFormat {
            field,
            value: version.to_owned(),
        })?;

    if major > max_supported_major {
        return Err(ContractError::UnsupportedFutureMajorVersion {
            field,
            major,
            max_supported_major,
        });
    }

    Ok(())
}

pub fn assert_required_major(
    field: &'static str,
    version: &str,
    required_major: u64,
) -> Result<(), ContractError> {
    let major_segment = version.split('.').next().unwrap_or_default().trim();
    let major = major_segment
        .parse::<u64>()
        .map_err(|_| ContractError::InvalidVersionFormat {
            field,
            value: version.to_owned(),
        })?;

    if major != required_major {
        return Err(ContractError::UnsupportedMajorVersion {
            field,
            major,
            supported_major: required_major,
        });
    }

    Ok(())
}
