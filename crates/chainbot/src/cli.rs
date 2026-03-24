//! [INPUT]
//! Process arguments, environment-resolved ChainBot roots, and runtime services from config, state, trigger, executor, worker, and secrets modules.
//!
//! [OUTPUT]
//! Parses commands, executes help, init, status, observe, trigger, validate, run, serve, and list-runs flows, and maps failures to stable CLI output and exit codes.
//!
//! [ROLE]
//! Owns the user-facing command boundary for the `chainbot` binary.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::builtins::nodes::script_worker::{WorkerHost, WorkerHostLimits};
use crate::builtins::{
    build_builtin_registry, build_builtin_trigger_emissions, BuiltinRuntimeContext,
    SecretDecryptMode,
};
use crate::catalog::{
    build_catalog_list, build_catalog_show, build_status_plugin_summary, render_catalog_list,
    render_catalog_show, CatalogFilterKind, CatalogReference, StatusPluginSummaryView,
};
use crate::config::{
    load_effective_root_layout, load_trigger_definitions, resolve_root_layout, set_trigger_enabled,
    LocalStorageDefinition, RawDebugArtifactsDefinition, RootConfigDefinition,
    RootDefinitionBundle, RootLayout, RootPathOverrides, RuntimeHistoryRetentionDefinition,
    RuntimeStorageConfig, StorageDefinition, StorageMode, TriggerToggleResult,
};
use crate::errors::UserFacingError;
use crate::executor::{ExecutionPlane, NormalizedRunRequest, WorkflowRunStatus};
use crate::ingress::{
    build_desired_ingress_state, drain_ingress_emissions, DesiredIngressState, IngressRuntimeError,
    TriggerIngressSupervisor,
};
use crate::plugin::{PluginKind, PluginManifest};
use crate::state::{
    sanitize_path_component, LeaseAcquireResult, RunRecordSummary, RunStatus, ServeLeaseState,
    TriggerEventRecord, TriggerSnapshotRecord, WorkflowRuntimeLogEntry, SERVE_OWNER_ID_PREFIX,
};
use crate::state_db::{
    RuntimeDaemonStatus, RuntimeHistoryArchiveCounts, RuntimeStateError, RuntimeStateStore,
};
use crate::trigger::{
    TriggerDefinition, TriggerPlane, TriggerPlaneError, TriggerPluginHostPolicy, TriggerRunRequest,
    REQUIRED_TRIGGER_PLUGIN_CAPABILITY,
};
use crate::workflow::WorkflowDefinition;

const SERVE_LEASE_TTL_MS: i64 = 30_000;
const SERVE_LEASE_RENEW_INTERVAL_MS: i64 = 10_000;
const SERVE_IDLE_POLL_INTERVAL_MS: u64 = 250;
const SERVE_ERROR_BACKOFF_MS: u64 = 1_000;
const SERVE_START_ACK_TIMEOUT_MS: u64 = 5_000;
const SERVE_STOP_TIMEOUT_MS: u64 = 5_000;
const INGRESS_INBOX_BATCH_LIMIT: usize = 256;
const REPLAYABLE_TRIGGER_BATCH_LIMIT: usize = 256;
const CHAINBOT_SECRET_DECRYPTOR_ENV: &str = "CHAINBOT_SECRET_DECRYPTOR";
const CHAINBOT_TEST_DAEMON_START_DELAY_MS_ENV: &str = "CHAINBOT_TEST_DAEMON_START_DELAY_MS";
const CHAINBOT_TEST_DAEMON_START_ACK_TIMEOUT_MS_ENV: &str =
    "CHAINBOT_TEST_DAEMON_START_ACK_TIMEOUT_MS";
const SECRET_DECRYPTOR_PLAINTEXT: &str = "plaintext";
const INIT_MANIFEST_VERSION: &str = "2.0.0";
const CHAINBOT_VERSION: &str = env!("CARGO_PKG_VERSION");
const GENERAL_HELP_EXAMPLE_HINT: &str =
    "Use `chainbot help validate` for end-to-end config examples.";
const ROOT_CONFIG_EXAMPLE: &str = r#"# chainbot.toml
manifest_version = "2.0.0"
chainbot_version = "2.2.0"
profile = "basic"
secret_refs = ["secret://ops/slack/webhook#token"]

[paths]
workflows_dir = "workflows"
triggers_dir = "triggers"
plugins_dir = "plugins"
secrets_dir = "secrets"
state_dir = "state"

[runtime_defaults]
timezone = "UTC"
"#;
const WORKFLOW_CONFIG_EXAMPLE: &str = r#"# workflows/wf-alpha/config.toml
[workflow]
manifest_version = "2.0.0"
id = "wf-alpha"
name = "alpha"
description = "Normalize a quote payload"

[runtime.defaults]
symbol = "BTCUSDT"

[[nodes]]
manifest_version = "2.0.0"
id = "normalize"
kind = "plugin"
plugin = "quote-plugin"
operation = "normalize"
depends_on = []
"#;
const TRIGGER_CONFIG_EXAMPLE: &str = r#"# triggers/tr-market/config.toml
manifest_version = "2.0.0"
trigger_id = "tr-market"
kind = "builtin"
source = "market_tick"
workflow_id = "wf-alpha"
enabled = true

[params]
symbol = "BTCUSDT"

[input_mapping]
symbol = "payload.symbol"
price = "payload.price"
"#;
const WEBHOOK_TRIGGER_CONFIG_EXAMPLE: &str = r#"# triggers/tr-webhook/config.toml
manifest_version = "2.0.0"
trigger_id = "tr-webhook"
kind = "builtin"
source = "webhook"
workflow_id = "wf-alpha"
enabled = true

[params]
bind = "127.0.0.1:8080"
path = "/ingress/webhook"
method = "POST"
max_body_bytes = 65536
content_type = "application/json"
idempotency_header = "x-event-id"

[params.auth]
kind = "header_token"
header_name = "x-chainbot-token"
token = "dev-webhook-token"

[input_mapping]
symbol = "payload.symbol"
price = "payload.price"
event_id = "payload.id"
"#;
const WEBSOCKET_TRIGGER_CONFIG_EXAMPLE: &str = r#"# triggers/tr-websocket/config.toml
manifest_version = "2.0.0"
trigger_id = "tr-websocket"
kind = "builtin"
source = "websocket"
workflow_id = "wf-alpha"
enabled = true

[params]
bind = "127.0.0.1:8081"
path = "/ingress/ws"
max_connections = 32
max_message_bytes = 65536
idle_timeout_ms = 30000

[params.auth]
kind = "header_token"
header_name = "x-chainbot-token"
token = "dev-websocket-token"

[input_mapping]
symbol = "payload.symbol"
price = "payload.price"
event = "payload.event"
"#;
const PLUGIN_CONFIG_EXAMPLE: &str = r#"# plugins/quote-plugin/config.toml
manifest_version = "2.0.0"
plugin_id = "quote-plugin"
kind = "external_node"
entrypoint = "node.exec.v1"
capabilities = ["node:execute"]
executable = "bin/quote-plugin.sh"

[[operations]]
name = "normalize"
summary = "Normalize quote payload"
input_schema = ["symbol", "token"]
output_schema = ["decision"]
"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliCommand {
    Help(HelpTopic),
    Version,
    Init,
    Status,
    Observe,
    Catalog,
    Stop,
    Trigger,
    Validate,
    Run,
    Serve,
    InternalServeDaemon,
    ListRuns,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelpTopic {
    General,
    Version,
    Init,
    Status,
    Observe,
    Catalog,
    Stop,
    Trigger,
    Validate,
    Run,
    Serve,
    ListRuns,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliRequest {
    pub command: CliCommand,
    pub json_output: bool,
    pub trigger_operation: Option<TriggerOperation>,
    pub catalog_request: Option<CatalogRequest>,
    pub observe_request: Option<ObserveRequest>,
    pub daemon_owner_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliOutput {
    stdout: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TriggerOperation {
    List,
    Enable { trigger_id: String },
    Disable { trigger_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObserveRequest {
    pub limit: usize,
    pub trigger_id: Option<String>,
    pub run_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogRequest {
    List { filter: Option<CatalogFilterKind> },
    Show { reference: CatalogReference },
}

#[derive(Debug)]
struct InitResult {
    root: PathBuf,
    created_paths: Vec<PathBuf>,
    reused_paths: Vec<PathBuf>,
}

#[derive(Debug)]
struct RuntimeContext {
    root_layout: RootLayout,
    definitions: RootDefinitionBundle,
    storage_config: RuntimeStorageConfig,
    state_store: RuntimeStateStore,
    secret_mode: SecretDecryptMode,
    worker_host: WorkerHost,
}

#[derive(Debug)]
struct SingleRunResult {
    run_id: String,
    workflow_id: String,
    status: RunStatus,
}

#[derive(Debug, Clone)]
struct ServeLeaseSupervisor {
    storage_config: RuntimeStorageConfig,
    owner_id: String,
    pid: i64,
    lease_ttl_ms: i64,
    renew_interval_ms: i64,
    next_renew_at_ms: i64,
}

impl ServeLeaseSupervisor {
    fn new(
        storage_config: RuntimeStorageConfig,
        owner_id: String,
        pid: i64,
        acquired_at_ms: i64,
    ) -> Self {
        Self {
            storage_config,
            owner_id,
            pid,
            lease_ttl_ms: SERVE_LEASE_TTL_MS,
            renew_interval_ms: SERVE_LEASE_RENEW_INTERVAL_MS,
            next_renew_at_ms: acquired_at_ms.saturating_add(SERVE_LEASE_RENEW_INTERVAL_MS),
        }
    }

    fn maybe_renew(&mut self, now_ms: i64) -> Result<(), UserFacingError> {
        if now_ms < self.next_renew_at_ms {
            return Ok(());
        }

        let mut store = RuntimeStateStore::open(&self.storage_config, now_ms)
            .map_err(|error| map_runtime_state_error("open runtime state store", error))?;
        match store
            .try_acquire_serve_lease(&self.owner_id, now_ms, self.lease_ttl_ms)
            .map_err(|error| map_runtime_state_error("renew serve lease", error))?
        {
            LeaseAcquireResult::Acquired | LeaseAcquireResult::Renewed => {
                store
                    .heartbeat_daemon(
                        &self.owner_id,
                        Some(self.pid),
                        now_ms,
                        now_ms.saturating_add(self.lease_ttl_ms),
                    )
                    .map_err(|error| map_runtime_state_error("heartbeat daemon session", error))?;
                self.next_renew_at_ms = now_ms.saturating_add(self.renew_interval_ms);
                Ok(())
            }
            LeaseAcquireResult::Rejected {
                current_owner,
                expires_at_ms,
            } => Err(UserFacingError::conflict(format!(
                "Serve lease renewal was rejected (current owner: {current_owner}, expires_at_ms: {expires_at_ms})."
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
struct StatusOutput {
    root: StatusRootView,
    serve: StatusServeView,
    workflows: Vec<StatusWorkflowView>,
    triggers: Vec<StatusTriggerView>,
    plugins: StatusPluginSummaryView,
    summary: StatusSummaryView,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
struct StatusRootView {
    path: String,
    profile: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
struct StatusServeView {
    state: ServeLeaseState,
    owner: Option<String>,
    pid: Option<i64>,
    started_at_ms: Option<i64>,
    last_heartbeat_at_ms: Option<i64>,
    lease_expires_at_ms: Option<i64>,
    last_reload_at_ms: Option<i64>,
    stop_requested_at_ms: Option<i64>,
    last_error_code: Option<String>,
    last_error_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
struct StatusWorkflowView {
    workflow_id: String,
    last_run_status: Option<RunStatus>,
    last_run_id: Option<String>,
    last_started_at_ms: Option<i64>,
    last_finished_at_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
struct StatusTriggerView {
    trigger_id: String,
    enabled: bool,
    workflow_id: String,
    last_event_id: Option<String>,
    last_accepted_at_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
struct StatusSummaryView {
    workflow_count: usize,
    trigger_count: usize,
    run_count: usize,
    running_run_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
struct ObserveOutput {
    summary: ObserveSummaryView,
    runs: Vec<RunRecordSummary>,
    workflow_logs: Vec<WorkflowRuntimeLogEntry>,
    trigger_events: Vec<TriggerEventRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
struct ObserveSummaryView {
    requested_limit: usize,
    archived: RuntimeHistoryArchiveCounts,
}

pub fn run_from_env() -> Result<CliOutput, UserFacingError> {
    CliRequest::from_env()?.execute()
}

impl CliRequest {
    pub fn from_env() -> Result<Self, UserFacingError> {
        Self::parse_from_args(std::env::args_os().skip(1))
    }

    pub fn parse_from_args<I>(mut args: I) -> Result<Self, UserFacingError>
    where
        I: Iterator<Item = OsString>,
    {
        let Some(command_raw) = args.next() else {
            return Err(UserFacingError::usage(format!(
                "No command provided. Run `chainbot --help` to see available commands.\n\n{}",
                general_help_text()
            )));
        };

        let command_raw = command_raw.to_string_lossy().into_owned();
        match command_raw.as_str() {
            "-h" | "--help" => Ok(Self {
                command: CliCommand::Help(HelpTopic::General),
                json_output: false,
                trigger_operation: None,
                catalog_request: None,
                observe_request: None,
                daemon_owner_id: None,
            }),
            "-V" | "--version" => Ok(Self {
                command: CliCommand::Version,
                json_output: false,
                trigger_operation: None,
                catalog_request: None,
                observe_request: None,
                daemon_owner_id: None,
            }),
            "help" => Self::parse_help_args(args),
            "version" => Self::parse_command_args(CliCommand::Version, args),
            "init" => Self::parse_command_args(CliCommand::Init, args),
            "status" => Self::parse_command_args(CliCommand::Status, args),
            "observe" => Self::parse_observe_args(args),
            "catalog" => Self::parse_catalog_args(args),
            "stop" => Self::parse_command_args(CliCommand::Stop, args),
            "trigger" => Self::parse_trigger_args(args),
            "validate" => Self::parse_command_args(CliCommand::Validate, args),
            "run" => Self::parse_command_args(CliCommand::Run, args),
            "serve" => Self::parse_command_args(CliCommand::Serve, args),
            "__serve-daemon" => Self::parse_internal_daemon_args(args),
            "list-runs" => Self::parse_command_args(CliCommand::ListRuns, args),
            other => Err(UserFacingError::usage(format!(
                "{}",
                unsupported_command_message(other)
            ))),
        }
    }

    pub fn execute(&self) -> Result<CliOutput, UserFacingError> {
        match self.command {
            CliCommand::Help(topic) => Ok(CliOutput::text(help_text(topic))),
            CliCommand::Version => Ok(CliOutput::text(render_version_output())),
            CliCommand::Init => self.execute_init(),
            CliCommand::Status => self.execute_status(),
            CliCommand::Observe => self.execute_observe(),
            CliCommand::Catalog => self.execute_catalog(),
            CliCommand::Stop => self.execute_stop(),
            CliCommand::Trigger => self.execute_trigger(),
            CliCommand::Validate => self.execute_validate(),
            CliCommand::Run => self.execute_run(),
            CliCommand::Serve => self.execute_serve(),
            CliCommand::InternalServeDaemon => self.execute_internal_serve_daemon(),
            CliCommand::ListRuns => self.execute_list_runs(),
        }
    }

    fn parse_help_args<I>(mut args: I) -> Result<Self, UserFacingError>
    where
        I: Iterator<Item = OsString>,
    {
        let topic = match args.next() {
            None => HelpTopic::General,
            Some(value) => parse_help_topic(&value.to_string_lossy())?,
        };

        if let Some(extra) = args.next() {
            return Err(UserFacingError::usage(format!(
                "Unexpected argument #2 for `chainbot help`: `{}`. Usage: `chainbot help [command]`.",
                extra.to_string_lossy()
            )));
        }

        Ok(Self {
            command: CliCommand::Help(topic),
            json_output: false,
            trigger_operation: None,
            catalog_request: None,
            observe_request: None,
            daemon_owner_id: None,
        })
    }

    fn parse_command_args<I>(command: CliCommand, mut args: I) -> Result<Self, UserFacingError>
    where
        I: Iterator<Item = OsString>,
    {
        let mut json_output = false;
        let mut position = 0usize;
        while let Some(arg) = args.next() {
            position += 1;
            let raw = arg.to_string_lossy().into_owned();
            match raw.as_str() {
                "-h" | "--help" => {
                    return Ok(Self {
                        command: CliCommand::Help(help_topic_for(command)),
                        json_output: false,
                        trigger_operation: None,
                        catalog_request: None,
                        observe_request: None,
                        daemon_owner_id: None,
                    });
                }
                "--json" if matches!(command, CliCommand::Status | CliCommand::Observe) => {
                    json_output = true;
                }
                _ => {
                    if let Some((flag, value)) = raw.split_once('=') {
                        if flag == "--json"
                            && matches!(command, CliCommand::Status | CliCommand::Observe)
                        {
                            json_output = parse_bool_flag_value(
                                "--json",
                                value,
                                position,
                                &format!("chainbot {}", command_name(command)),
                            )?;
                            continue;
                        }
                    }

                    return Err(UserFacingError::usage(format!(
                        "Unexpected argument #{position} after `chainbot {}`: `{raw}`. Run `chainbot help {}` for valid forms.",
                        command_name(command),
                        command_name(command)
                    )));
                }
            }
        }

        Ok(Self {
            command,
            json_output,
            trigger_operation: None,
            catalog_request: None,
            observe_request: None,
            daemon_owner_id: None,
        })
    }

    fn parse_internal_daemon_args<I>(mut args: I) -> Result<Self, UserFacingError>
    where
        I: Iterator<Item = OsString>,
    {
        let mut daemon_owner_id = None;
        let mut position = 0usize;

        while let Some(arg) = args.next() {
            position += 1;
            let raw = arg.to_string_lossy().into_owned();
            match raw.as_str() {
                "--owner-id" => {
                    let Some(value) = args.next() else {
                        return Err(UserFacingError::usage_with_code(
                            "daemon_start_failed",
                            "`chainbot __serve-daemon --owner-id` requires a daemon owner identifier.",
                        ));
                    };
                    position += 1;
                    daemon_owner_id = Some(value.to_string_lossy().into_owned());
                }
                _ => {
                    if let Some((flag, value)) = raw.split_once('=') {
                        if flag == "--owner-id" {
                            daemon_owner_id = Some(value.to_owned());
                            continue;
                        }
                    }

                    return Err(UserFacingError::usage_with_code(
                        "daemon_start_failed",
                        format!(
                            "Unexpected argument #{position} after `chainbot __serve-daemon`: `{raw}`."
                        ),
                    ));
                }
            }
        }

        let daemon_owner_id = daemon_owner_id.ok_or_else(|| {
            UserFacingError::usage_with_code(
                "daemon_start_failed",
                "`chainbot __serve-daemon` requires `--owner-id <owner-id>`.",
            )
        })?;

        Ok(Self {
            command: CliCommand::InternalServeDaemon,
            json_output: false,
            trigger_operation: None,
            catalog_request: None,
            observe_request: None,
            daemon_owner_id: Some(daemon_owner_id),
        })
    }

    fn parse_catalog_args<I>(mut args: I) -> Result<Self, UserFacingError>
    where
        I: Iterator<Item = OsString>,
    {
        let Some(action) = args.next() else {
            return Err(UserFacingError::usage(
                "`chainbot catalog` requires `list` or `show`. Run `chainbot help catalog`.",
            ));
        };
        let action = action.to_string_lossy().into_owned();
        if matches!(action.as_str(), "-h" | "--help") {
            return Ok(Self {
                command: CliCommand::Help(HelpTopic::Catalog),
                json_output: false,
                trigger_operation: None,
                catalog_request: None,
                observe_request: None,
                daemon_owner_id: None,
            });
        }

        match action.as_str() {
            "list" => {
                let mut json_output = false;
                let mut filter = None;
                let mut position = 1usize;
                while let Some(arg) = args.next() {
                    position += 1;
                    let raw = arg.to_string_lossy().into_owned();
                    match raw.as_str() {
                        "--json" => json_output = true,
                        "--kind" => {
                            let Some(value) = args.next() else {
                                return Err(UserFacingError::usage(
                                    "`chainbot catalog list --kind` requires builtin_node, builtin_trigger, or plugin.",
                                ));
                            };
                            position += 1;
                            filter = Some(parse_catalog_filter_kind(
                                &value.to_string_lossy(),
                                position,
                            )?);
                        }
                        _ => {
                            if let Some((flag, value)) = raw.split_once('=') {
                                match flag {
                                    "--json" => {
                                        json_output = parse_bool_flag_value(
                                            "--json",
                                            value,
                                            position,
                                            "chainbot catalog list",
                                        )?;
                                        continue;
                                    }
                                    "--kind" => {
                                        filter = Some(parse_catalog_filter_kind(value, position)?);
                                        continue;
                                    }
                                    _ => {}
                                }
                            }
                            return Err(UserFacingError::usage(format!(
                                "Unexpected argument #{position} after `chainbot catalog list`: `{raw}`. Run `chainbot help catalog` for valid forms."
                            )));
                        }
                    }
                }

                Ok(Self {
                    command: CliCommand::Catalog,
                    json_output,
                    trigger_operation: None,
                    catalog_request: Some(CatalogRequest::List { filter }),
                    observe_request: None,
                    daemon_owner_id: None,
                })
            }
            "show" => {
                let mut json_output = false;
                let mut reference = None;
                let mut position = 1usize;
                while let Some(arg) = args.next() {
                    position += 1;
                    let raw = arg.to_string_lossy().into_owned();
                    match raw.as_str() {
                        "--json" => json_output = true,
                        _ => {
                            if let Some((flag, value)) = raw.split_once('=') {
                                if flag == "--json" {
                                    json_output = parse_bool_flag_value(
                                        "--json",
                                        value,
                                        position,
                                        "chainbot catalog show",
                                    )?;
                                    continue;
                                }
                            }
                            if reference.is_none() {
                                reference = Some(CatalogReference::parse(&raw).map_err(|error| {
                                    UserFacingError::usage(format!(
                                        "Unsupported catalog reference at argument #{position} after `chainbot catalog show`: {error}. Run `chainbot catalog list` first."
                                    ))
                                })?);
                                continue;
                            }
                            return Err(UserFacingError::usage(format!(
                                "Unexpected argument #{position} after `chainbot catalog show`: `{raw}`. Run `chainbot help catalog` for valid forms."
                            )));
                        }
                    }
                }
                let reference = reference.ok_or_else(|| {
                    UserFacingError::usage(
                        "`chainbot catalog show` requires a <kind>:<value> reference. Run `chainbot catalog list`."
                    )
                })?;

                Ok(Self {
                    command: CliCommand::Catalog,
                    json_output,
                    trigger_operation: None,
                    catalog_request: Some(CatalogRequest::Show { reference }),
                    observe_request: None,
                    daemon_owner_id: None,
                })
            }
            other => Err(UserFacingError::usage(format!(
                "Unsupported catalog action at argument #1 after `chainbot catalog`: `{other}`. Use `list` or `show`."
            ))),
        }
    }

    fn parse_observe_args<I>(mut args: I) -> Result<Self, UserFacingError>
    where
        I: Iterator<Item = OsString>,
    {
        let mut json_output = false;
        let mut limit = 10usize;
        let mut trigger_id = None;
        let mut run_id = None;
        let mut position = 0usize;

        while let Some(arg) = args.next() {
            position += 1;
            let raw = arg.to_string_lossy().into_owned();
            match raw.as_str() {
                "-h" | "--help" => {
                    return Ok(Self {
                        command: CliCommand::Help(HelpTopic::Observe),
                        json_output: false,
                        trigger_operation: None,
                        catalog_request: None,
                        observe_request: None,
                        daemon_owner_id: None,
                    });
                }
                "--json" => {
                    json_output = true;
                }
                "--limit" => {
                    let Some(value) = args.next() else {
                        return Err(UserFacingError::usage(
                            "`chainbot observe --limit` requires a positive integer value.",
                        ));
                    };
                    position += 1;
                    limit = parse_observe_limit(&value.to_string_lossy(), position)?;
                }
                "--trigger-id" => {
                    let Some(value) = args.next() else {
                        return Err(UserFacingError::usage(
                            "`chainbot observe --trigger-id` requires a trigger identifier.",
                        ));
                    };
                    position += 1;
                    trigger_id = Some(value.to_string_lossy().into_owned());
                }
                "--run-id" => {
                    let Some(value) = args.next() else {
                        return Err(UserFacingError::usage(
                            "`chainbot observe --run-id` requires a run identifier.",
                        ));
                    };
                    position += 1;
                    run_id = Some(value.to_string_lossy().into_owned());
                }
                _ => {
                    if let Some((flag, value)) = raw.split_once('=') {
                        match flag {
                            "--json" => {
                                json_output = parse_bool_flag_value(
                                    "--json",
                                    value,
                                    position,
                                    "chainbot observe",
                                )?;
                                continue;
                            }
                            "--limit" => {
                                limit = parse_observe_limit(value, position)?;
                                continue;
                            }
                            "--trigger-id" => {
                                trigger_id = Some(value.to_owned());
                                continue;
                            }
                            "--run-id" => {
                                run_id = Some(value.to_owned());
                                continue;
                            }
                            _ => {}
                        }
                    }

                    return Err(UserFacingError::usage(format!(
                        "Unexpected argument #{position} after `chainbot observe`: `{raw}`. Run `chainbot help observe` for valid forms."
                    )));
                }
            }
        }

        Ok(Self {
            command: CliCommand::Observe,
            json_output,
            trigger_operation: None,
            catalog_request: None,
            observe_request: Some(ObserveRequest {
                limit,
                trigger_id,
                run_id,
            }),
            daemon_owner_id: None,
        })
    }

    fn parse_trigger_args<I>(mut args: I) -> Result<Self, UserFacingError>
    where
        I: Iterator<Item = OsString>,
    {
        let Some(action) = args.next() else {
            return Err(UserFacingError::usage(
                "`chainbot trigger` requires `list`, `enable`, or `disable`. Run `chainbot help trigger`.",
            ));
        };
        let action = action.to_string_lossy().into_owned();
        if matches!(action.as_str(), "-h" | "--help") {
            return Ok(Self {
                command: CliCommand::Help(HelpTopic::Trigger),
                json_output: false,
                trigger_operation: None,
                catalog_request: None,
                observe_request: None,
                daemon_owner_id: None,
            });
        }

        if action == "list" {
            let mut json_output = false;
            let mut position = 1usize;
            while let Some(arg) = args.next() {
                position += 1;
                let raw = arg.to_string_lossy().into_owned();
                match raw.as_str() {
                    "--json" => {
                        json_output = true;
                    }
                    _ => {
                        if let Some((flag, value)) = raw.split_once('=') {
                            if flag == "--json" {
                                json_output = parse_bool_flag_value(
                                    "--json",
                                    value,
                                    position,
                                    "chainbot trigger list",
                                )?;
                                continue;
                            }
                        }

                        return Err(UserFacingError::usage(format!(
                            "Unexpected argument #{position} after `chainbot trigger list`: `{raw}`. Run `chainbot help trigger` for valid forms."
                        )));
                    }
                }
            }

            return Ok(Self {
                command: CliCommand::Trigger,
                json_output,
                trigger_operation: Some(TriggerOperation::List),
                catalog_request: None,
                observe_request: None,
                daemon_owner_id: None,
            });
        }

        let enabled = match action.as_str() {
            "enable" => true,
            "disable" => false,
            other => {
                return Err(UserFacingError::usage(format!(
                    "Unsupported trigger action at argument #1 after `chainbot trigger`: `{other}`. Use `list`, `enable`, or `disable`."
                )));
            }
        };

        let mut trigger_id = None;
        let mut position = 1usize;
        while let Some(arg) = args.next() {
            position += 1;
            let raw = arg.to_string_lossy().into_owned();
            match raw.as_str() {
                "-h" | "--help" => {
                    return Ok(Self {
                        command: CliCommand::Help(HelpTopic::Trigger),
                        json_output: false,
                        trigger_operation: None,
                        catalog_request: None,
                        observe_request: None,
                        daemon_owner_id: None,
                    });
                }
                _ => {
                    if trigger_id.is_none() {
                        trigger_id = Some(raw);
                        continue;
                    }

                    return Err(UserFacingError::usage(format!(
                        "Unexpected argument #{position} after `chainbot trigger {}`: `{raw}`. Run `chainbot help trigger` for valid forms.",
                        if enabled { "enable" } else { "disable" }
                    )));
                }
            }
        }

        let trigger_id = trigger_id.ok_or_else(|| {
            UserFacingError::usage(
                "`chainbot trigger enable|disable` requires a <trigger-id>. Run `chainbot help trigger`.",
            )
        })?;

        Ok(Self {
            command: CliCommand::Trigger,
            json_output: false,
            trigger_operation: Some(if enabled {
                TriggerOperation::Enable { trigger_id }
            } else {
                TriggerOperation::Disable { trigger_id }
            }),
            catalog_request: None,
            observe_request: None,
            daemon_owner_id: None,
        })
    }

    fn execute_init(&self) -> Result<CliOutput, UserFacingError> {
        let root_layout = resolve_root_layout().map_err(UserFacingError::from_contract)?;
        let init_result = initialize_root_layout(&root_layout)?;
        let _ = RootDefinitionBundle::load(&root_layout).map_err(UserFacingError::from_contract)?;
        Ok(CliOutput::text(render_init_output(&init_result)))
    }

    fn execute_status(&self) -> Result<CliOutput, UserFacingError> {
        let root_layout = self.resolve_existing_root()?;
        let definitions =
            RootDefinitionBundle::load(&root_layout).map_err(UserFacingError::from_contract)?;
        let storage_config = definitions
            .root_config
            .resolve_runtime_storage(&root_layout.root)
            .map_err(UserFacingError::from_contract)?;
        let mut state_store = RuntimeStateStore::open(&storage_config, current_time_ms()?)
            .map_err(|error| map_runtime_state_error("open runtime state store", error))?;
        let observed_at_ms = current_time_ms()?;
        let run_summaries = state_store
            .list_run_summaries()
            .map_err(|error| map_runtime_state_error("list run summaries", error))?;
        let trigger_snapshots = state_store
            .list_trigger_snapshots()
            .map_err(|error| map_runtime_state_error("load trigger snapshots", error))?;
        let daemon_status = state_store
            .inspect_daemon_status(observed_at_ms)
            .map_err(|error| map_runtime_state_error("inspect daemon status", error))?;

        let payload = build_status_output(
            &root_layout,
            definitions.root_config.profile,
            &definitions.workflows,
            &definitions.triggers,
            &definitions.plugins,
            &run_summaries,
            &trigger_snapshots,
            daemon_status,
        );

        if self.json_output {
            let stdout = serde_json::to_string_pretty(&payload).map_err(|source| {
                UserFacingError::state(format!("Failed to serialize status payload: {source}"))
            })?;
            return Ok(CliOutput::text(stdout));
        }

        Ok(CliOutput::text(render_status_output(&payload)))
    }

    fn execute_observe(&self) -> Result<CliOutput, UserFacingError> {
        let root_layout = self.resolve_existing_root()?;
        let definitions =
            RootDefinitionBundle::load(&root_layout).map_err(UserFacingError::from_contract)?;
        let storage_config = definitions
            .root_config
            .resolve_runtime_storage(&root_layout.root)
            .map_err(UserFacingError::from_contract)?;
        let mut state_store = RuntimeStateStore::open(&storage_config, current_time_ms()?)
            .map_err(|error| map_runtime_state_error("open runtime state store", error))?;
        let observe_request = self.observe_request.as_ref().ok_or_else(|| {
            UserFacingError::usage("Missing observe request. Run `chainbot help observe`.")
        })?;
        let payload = ObserveOutput {
            summary: ObserveSummaryView {
                requested_limit: observe_request.limit,
                archived: state_store.archived_history_counts().map_err(|error| {
                    map_runtime_state_error("count archived runtime history", error)
                })?,
            },
            runs: state_store
                .list_recent_run_summaries(observe_request.limit)
                .map_err(|error| map_runtime_state_error("list recent run summaries", error))?,
            workflow_logs: state_store
                .list_recent_workflow_log_entries(
                    observe_request.limit,
                    observe_request.run_id.as_deref(),
                )
                .map_err(|error| map_runtime_state_error("list recent workflow logs", error))?,
            trigger_events: state_store
                .list_recent_trigger_records(
                    observe_request.limit,
                    observe_request.trigger_id.as_deref(),
                )
                .map_err(|error| map_runtime_state_error("list recent trigger events", error))?,
        };

        if self.json_output {
            let stdout = serde_json::to_string_pretty(&payload).map_err(|source| {
                UserFacingError::state(format!("Failed to serialize observe payload: {source}"))
            })?;
            return Ok(CliOutput::text(stdout));
        }

        Ok(CliOutput::text(render_observe_output(&payload)))
    }

    fn execute_catalog(&self) -> Result<CliOutput, UserFacingError> {
        let catalog_request = self.catalog_request.as_ref().ok_or_else(|| {
            UserFacingError::usage("Missing catalog request. Run `chainbot help catalog`.")
        })?;
        match catalog_request {
            CatalogRequest::List { filter } => {
                let plugins = match filter {
                    Some(CatalogFilterKind::BuiltinNode | CatalogFilterKind::BuiltinTrigger) => {
                        Vec::new()
                    }
                    Some(CatalogFilterKind::Plugin) | None => load_catalog_plugins_if_available()?,
                };
                let payload = build_catalog_list(*filter, &plugins);
                if self.json_output {
                    let stdout = serde_json::to_string_pretty(&payload).map_err(|source| {
                        UserFacingError::state(format!(
                            "Failed to serialize catalog list payload: {source}"
                        ))
                    })?;
                    return Ok(CliOutput::text(stdout));
                }
                Ok(CliOutput::text(render_catalog_list(&payload, *filter)))
            }
            CatalogRequest::Show { reference } => {
                let plugins = match reference {
                    CatalogReference::BuiltinNode(_) | CatalogReference::BuiltinTrigger(_) => {
                        Vec::new()
                    }
                    CatalogReference::Plugin(_) => load_catalog_plugins_strict()?,
                };
                let payload = build_catalog_show(reference, &plugins).map_err(|error| {
                    UserFacingError::usage(format!(
                        "{error}. Run `chainbot catalog list` to inspect available references."
                    ))
                })?;
                if self.json_output {
                    let stdout = serde_json::to_string_pretty(&payload).map_err(|source| {
                        UserFacingError::state(format!(
                            "Failed to serialize catalog detail payload: {source}"
                        ))
                    })?;
                    return Ok(CliOutput::text(stdout));
                }
                Ok(CliOutput::text(render_catalog_show(&payload)))
            }
        }
    }

    fn execute_validate(&self) -> Result<CliOutput, UserFacingError> {
        let layout = self.load_definition_root()?;
        Ok(CliOutput::text(format!(
            "validated root: {}",
            layout.root.display()
        )))
    }

    fn execute_trigger(&self) -> Result<CliOutput, UserFacingError> {
        let root_layout = self.resolve_trigger_root()?;
        let operation = self.trigger_operation.as_ref().ok_or_else(|| {
            UserFacingError::usage(
                "Missing trigger operation. Run `chainbot help trigger` for usage.",
            )
        })?;
        let result = match operation {
            TriggerOperation::List => {
                let triggers = load_trigger_definitions(&root_layout)
                    .map_err(UserFacingError::from_contract)?;
                if self.json_output {
                    let stdout = serde_json::to_string_pretty(&triggers).map_err(|source| {
                        UserFacingError::state(format!(
                            "Failed to serialize trigger list payload: {source}"
                        ))
                    })?;
                    return Ok(CliOutput::text(stdout));
                }
                return Ok(CliOutput::text(render_trigger_list_output(&triggers)));
            }
            TriggerOperation::Enable { trigger_id } => {
                set_trigger_enabled(&root_layout, trigger_id, true)
            }
            TriggerOperation::Disable { trigger_id } => {
                set_trigger_enabled(&root_layout, trigger_id, false)
            }
        }
        .map_err(UserFacingError::from_contract)?;

        Ok(CliOutput::text(render_trigger_operation_output(&result)))
    }

    fn execute_list_runs(&self) -> Result<CliOutput, UserFacingError> {
        let layout = self.resolve_existing_root()?;
        let definitions =
            RootDefinitionBundle::load(&layout).map_err(UserFacingError::from_contract)?;
        let storage_config = definitions
            .root_config
            .resolve_runtime_storage(&layout.root)
            .map_err(UserFacingError::from_contract)?;
        let mut store = RuntimeStateStore::open(&storage_config, current_time_ms()?)
            .map_err(|error| map_runtime_state_error("open runtime state store", error))?;
        let summaries = store
            .list_run_summaries()
            .map_err(|error| map_runtime_state_error("list run summaries", error))?;
        let payload = serde_json::to_string_pretty(&summaries).map_err(|source| {
            UserFacingError::state(format!(
                "Failed to serialize run summaries for output: {source}"
            ))
        })?;
        Ok(CliOutput::text(payload))
    }

    fn execute_run(&self) -> Result<CliOutput, UserFacingError> {
        let mut runtime = self.load_runtime_context()?;
        let workflow = select_manual_run_workflow(&runtime.definitions.workflows)?;

        let mut request = NormalizedRunRequest::new(
            format!(
                "manual-{}-{}",
                sanitize_path_component(&workflow.workflow_id),
                current_time_ms()?
            ),
            workflow.workflow_id.clone(),
        );
        request
            .manual_invocation_input
            .insert("manual_invocation".to_owned(), serde_json::json!(true));

        let run_result = execute_single_run(&mut runtime, request, current_time_ms()?)?;
        Ok(CliOutput::text(format!(
            "run completed: run_id={} workflow_id={} status={}",
            run_result.run_id,
            run_result.workflow_id,
            render_run_status(run_result.status)
        )))
    }

    fn execute_serve(&self) -> Result<CliOutput, UserFacingError> {
        let root_layout = self.resolve_existing_root()?;
        let definitions =
            RootDefinitionBundle::load(&root_layout).map_err(UserFacingError::from_contract)?;
        let storage_config = definitions
            .root_config
            .resolve_runtime_storage(&root_layout.root)
            .map_err(UserFacingError::from_contract)?;
        let now_ms = current_time_ms()?;
        let owner_id = format!("{SERVE_OWNER_ID_PREFIX}{}-{now_ms}", std::process::id());
        let mut store = RuntimeStateStore::open(&storage_config, now_ms)
            .map_err(|error| map_runtime_state_error("open runtime state store", error))?;

        match store
            .try_acquire_serve_lease(&owner_id, now_ms, SERVE_LEASE_TTL_MS)
            .map_err(|error| map_runtime_state_error("acquire serve lease", error))?
        {
            LeaseAcquireResult::Acquired | LeaseAcquireResult::Renewed => {
                store
                    .register_daemon_start(&owner_id, None, now_ms, now_ms + SERVE_LEASE_TTL_MS)
                    .map_err(|error| map_runtime_state_error("register daemon start", error))?;
                let mut child = match spawn_serve_daemon_process(&owner_id) {
                    Ok(child) => child,
                    Err(error) => {
                        let cleanup_at_ms = current_time_ms().unwrap_or(now_ms);
                        let _ = store.mark_daemon_stopped(&owner_id, cleanup_at_ms);
                        let _ = store.release_serve_lease(&owner_id);
                        return Err(error);
                    }
                };
                if !store
                    .attach_daemon_pid(&owner_id, i64::from(child.id()))
                    .map_err(|error| map_runtime_state_error("attach daemon pid", error))?
                {
                    let cleanup_at_ms = current_time_ms().unwrap_or(now_ms);
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = store.mark_daemon_stopped(&owner_id, cleanup_at_ms);
                    let _ = store.release_serve_lease(&owner_id);
                    return Err(UserFacingError::state_with_code(
                        "daemon_start_failed",
                        "The daemon session disappeared before the child PID could be recorded.",
                    ));
                }
                wait_for_daemon_start(&storage_config, &owner_id, child.id(), now_ms)
                    .or_else(|error| {
                        let _ = child.kill();
                        let _ = child.wait();
                        let cleanup_at_ms = current_time_ms().unwrap_or(now_ms);
                        let _ = store.mark_daemon_stopped(&owner_id, cleanup_at_ms);
                        let _ = store.release_serve_lease(&owner_id);
                        Err(error)
                    })?;
                Ok(CliOutput::text(format!(
                    "serve started: owner={} pid={} root={}\nuse `chainbot status --json` for health and `chainbot observe` for persisted runtime output",
                    owner_id,
                    child.id(),
                    root_layout.root.display()
                )))
            }
            LeaseAcquireResult::Rejected {
                current_owner,
                expires_at_ms,
            } => Err(UserFacingError::conflict_with_code("daemon_already_running", format!(
                "Serve is already active for this root (owner: {current_owner}, expires_at_ms: {expires_at_ms})."
            ))),
        }
    }

    fn execute_stop(&self) -> Result<CliOutput, UserFacingError> {
        let root_layout = self.resolve_existing_root()?;
        let definitions =
            RootDefinitionBundle::load(&root_layout).map_err(UserFacingError::from_contract)?;
        let storage_config = definitions
            .root_config
            .resolve_runtime_storage(&root_layout.root)
            .map_err(UserFacingError::from_contract)?;
        let now_ms = current_time_ms()?;
        let mut store = RuntimeStateStore::open(&storage_config, now_ms)
            .map_err(|error| map_runtime_state_error("open runtime state store", error))?;
        let status = store
            .inspect_daemon_status(now_ms)
            .map_err(|error| map_runtime_state_error("inspect daemon status", error))?;

        match status.state {
            ServeLeaseState::Idle => {
                return Ok(CliOutput::text("stop completed: no active daemon"));
            }
            ServeLeaseState::Stale => {
                if let Some(owner_id) = status.owner_id.as_deref() {
                    let _ = store.mark_daemon_stopped(owner_id, now_ms);
                    let _ = store.release_serve_lease(owner_id);
                }
                return Ok(CliOutput::text(
                    "stop completed: stale daemon state cleaned up",
                ));
            }
            ServeLeaseState::Active => {}
        }

        if !store
            .request_daemon_stop(now_ms)
            .map_err(|error| map_runtime_state_error("request daemon stop", error))?
        {
            if let Some(owner_id) = status.owner_id.as_deref() {
                let _ = store.mark_daemon_stopped(owner_id, now_ms);
                let _ = store.release_serve_lease(owner_id);
                return Ok(CliOutput::text("stop completed: active lease cleared"));
            }
            return Ok(CliOutput::text("stop completed: no active daemon"));
        }

        wait_for_daemon_stop(&storage_config)?;
        Ok(CliOutput::text("stop completed: daemon shutdown requested"))
    }

    fn execute_internal_serve_daemon(&self) -> Result<CliOutput, UserFacingError> {
        let owner_id = self.daemon_owner_id.clone().ok_or_else(|| {
            UserFacingError::state_with_code(
                "daemon_start_failed",
                "Internal daemon start is missing the daemon owner identifier.",
            )
        })?;
        let now_ms = current_time_ms()?;
        let root_layout = self.resolve_existing_root()?;
        let definitions =
            RootDefinitionBundle::load(&root_layout).map_err(UserFacingError::from_contract)?;
        let storage_config = definitions
            .root_config
            .resolve_runtime_storage(&root_layout.root)
            .map_err(UserFacingError::from_contract)?;
        let pid = i64::from(std::process::id());

        {
            let mut store = RuntimeStateStore::open(&storage_config, now_ms)
                .map_err(|error| map_runtime_state_error("open runtime state store", error))?;
            let status = store
                .inspect_daemon_status(now_ms)
                .map_err(|error| map_runtime_state_error("inspect daemon status", error))?;
            if status.owner_id.as_deref() != Some(owner_id.as_str()) {
                return Ok(CliOutput::text(String::new()));
            }
            if status.stop_requested_at_ms.is_some() {
                return Ok(CliOutput::text(String::new()));
            }
            maybe_delay_daemon_start_for_tests()?;
            let status = store
                .inspect_daemon_status(current_time_ms()?)
                .map_err(|error| map_runtime_state_error("inspect daemon status", error))?;
            if status.owner_id.as_deref() != Some(owner_id.as_str()) {
                return Ok(CliOutput::text(String::new()));
            }
            if status.stop_requested_at_ms.is_some() {
                return Ok(CliOutput::text(String::new()));
            }
            let acked_at_ms = status
                .started_at_ms
                .map(|started_at_ms| {
                    current_time_ms()
                        .unwrap_or(now_ms)
                        .max(started_at_ms.saturating_add(1))
                })
                .unwrap_or_else(|| current_time_ms().unwrap_or(now_ms));
            store
                .heartbeat_daemon(
                    &owner_id,
                    Some(pid),
                    acked_at_ms,
                    acked_at_ms + SERVE_LEASE_TTL_MS,
                )
                .map_err(|error| map_runtime_state_error("acknowledge daemon start", error))?;
            if store
                .daemon_stop_requested(&owner_id)
                .map_err(|error| map_runtime_state_error("inspect daemon stop request", error))?
            {
                return Ok(CliOutput::text(String::new()));
            }
        }

        let loop_result = run_internal_serve_daemon_loop(self, &storage_config, &owner_id, pid);
        let stopped_at_ms = current_time_ms().unwrap_or(now_ms);
        if let Ok(mut store) = RuntimeStateStore::open(&storage_config, stopped_at_ms) {
            let _ = store.mark_daemon_stopped(&owner_id, stopped_at_ms);
            let _ = store.release_serve_lease(&owner_id);
        }

        loop_result?;
        Ok(CliOutput::text(String::new()))
    }

    fn resolve_existing_root(&self) -> Result<RootLayout, UserFacingError> {
        let bootstrap_layout = resolve_root_layout().map_err(UserFacingError::from_contract)?;
        maybe_prepare_e2e_root(&bootstrap_layout)?;
        let layout = load_effective_root_layout(&bootstrap_layout)
            .map_err(UserFacingError::from_contract)?;
        layout
            .validate_paths_exist()
            .map_err(UserFacingError::from_contract)?;
        Ok(layout)
    }

    fn load_definition_root(&self) -> Result<RootLayout, UserFacingError> {
        let layout = self.resolve_existing_root()?;
        let _ = RootDefinitionBundle::load(&layout).map_err(UserFacingError::from_contract)?;
        Ok(layout)
    }

    fn resolve_trigger_root(&self) -> Result<RootLayout, UserFacingError> {
        let bootstrap_layout = resolve_root_layout().map_err(UserFacingError::from_contract)?;
        maybe_prepare_e2e_root(&bootstrap_layout)?;
        load_trigger_definitions(&bootstrap_layout).map_err(UserFacingError::from_contract)?;
        load_effective_root_layout(&bootstrap_layout).map_err(UserFacingError::from_contract)
    }

    fn load_runtime_context(&self) -> Result<RuntimeContext, UserFacingError> {
        let root_layout = self.resolve_existing_root()?;
        let definitions =
            RootDefinitionBundle::load(&root_layout).map_err(UserFacingError::from_contract)?;
        let storage_config = definitions
            .root_config
            .resolve_runtime_storage(&root_layout.root)
            .map_err(UserFacingError::from_contract)?;
        let recovered_at_ms = current_time_ms()?;
        let mut state_store = RuntimeStateStore::open(&storage_config, recovered_at_ms)
            .map_err(|error| map_runtime_state_error("open runtime state store", error))?;
        let _ = state_store
            .recover_runtime_state(recovered_at_ms)
            .map_err(|error| map_runtime_state_error("recover runtime state", error))?;
        if let Some(policy) = storage_config.history_retention.as_ref() {
            let _ = state_store
                .apply_history_retention(policy, recovered_at_ms)
                .map_err(|error| {
                    map_runtime_state_error("apply runtime history retention", error)
                })?;
        }

        Ok(RuntimeContext {
            root_layout,
            definitions,
            storage_config,
            state_store,
            secret_mode: secret_decrypt_mode_from_env(),
            worker_host: WorkerHost::new(WorkerHostLimits::default()),
        })
    }
}

fn select_manual_run_workflow<'a>(
    workflows: &'a [WorkflowDefinition],
) -> Result<&'a WorkflowDefinition, UserFacingError> {
    match workflows {
        [] => Err(UserFacingError::validation(
            "No workflows were found in the configured workflows directory.",
        )),
        [workflow] => Ok(workflow),
        _ => {
            let referenced_child_ids = workflows
                .iter()
                .flat_map(|workflow| workflow.nodes.iter())
                .filter_map(|node| node.subflow.as_ref())
                .map(|contract| contract.workflow_id.as_str())
                .collect::<BTreeSet<_>>();

            let top_level_workflows = workflows
                .iter()
                .filter(|workflow| !referenced_child_ids.contains(workflow.workflow_id.as_str()))
                .collect::<Vec<_>>();

            match top_level_workflows.as_slice() {
                [workflow] => Ok(*workflow),
                [] => Err(UserFacingError::usage(
                    "chainbot run could not infer a top-level workflow from the configured subflow graph; use a dedicated root or add workflow selection support.",
                )),
                _ => Err(UserFacingError::usage(
                    "chainbot run found multiple top-level workflows in the root; use a dedicated root or add workflow selection support.",
                )),
            }
        }
    }
}

impl CliOutput {
    pub fn stdout(&self) -> &str {
        &self.stdout
    }

    fn text(text: impl Into<String>) -> Self {
        let mut stdout = text.into();
        if !stdout.is_empty() && !stdout.ends_with('\n') {
            stdout.push('\n');
        }
        Self { stdout }
    }
}

fn parse_help_topic(value: &str) -> Result<HelpTopic, UserFacingError> {
    match value {
        "version" => Ok(HelpTopic::Version),
        "init" => Ok(HelpTopic::Init),
        "status" => Ok(HelpTopic::Status),
        "observe" => Ok(HelpTopic::Observe),
        "catalog" => Ok(HelpTopic::Catalog),
        "stop" => Ok(HelpTopic::Stop),
        "trigger" => Ok(HelpTopic::Trigger),
        "validate" => Ok(HelpTopic::Validate),
        "run" => Ok(HelpTopic::Run),
        "serve" => Ok(HelpTopic::Serve),
        "list-runs" => Ok(HelpTopic::ListRuns),
        other => Err(UserFacingError::usage(format!(
            "Unsupported help topic at argument #1 after `chainbot help`: `{other}`. Run `chainbot help` to see available command skills."
        ))),
    }
}

fn help_topic_for(command: CliCommand) -> HelpTopic {
    match command {
        CliCommand::Help(topic) => topic,
        CliCommand::Version => HelpTopic::Version,
        CliCommand::Init => HelpTopic::Init,
        CliCommand::Status => HelpTopic::Status,
        CliCommand::Observe => HelpTopic::Observe,
        CliCommand::Catalog => HelpTopic::Catalog,
        CliCommand::Stop => HelpTopic::Stop,
        CliCommand::Trigger => HelpTopic::Trigger,
        CliCommand::Validate => HelpTopic::Validate,
        CliCommand::Run => HelpTopic::Run,
        CliCommand::Serve => HelpTopic::Serve,
        CliCommand::InternalServeDaemon => HelpTopic::Serve,
        CliCommand::ListRuns => HelpTopic::ListRuns,
    }
}

fn command_name(command: CliCommand) -> &'static str {
    match command {
        CliCommand::Help(_) => "help",
        CliCommand::Version => "version",
        CliCommand::Init => "init",
        CliCommand::Status => "status",
        CliCommand::Observe => "observe",
        CliCommand::Catalog => "catalog",
        CliCommand::Stop => "stop",
        CliCommand::Trigger => "trigger",
        CliCommand::Validate => "validate",
        CliCommand::Run => "run",
        CliCommand::Serve => "serve",
        CliCommand::InternalServeDaemon => "__serve-daemon",
        CliCommand::ListRuns => "list-runs",
    }
}

fn push_help_list_section(lines: &mut Vec<String>, title: &str, items: &[&str]) {
    if items.is_empty() {
        return;
    }

    if !lines.is_empty() {
        lines.push(String::new());
    }
    lines.push(format!("{title}:"));
    for item in items {
        lines.push(format!("  - {item}"));
    }
}

fn push_help_code_block(lines: &mut Vec<String>, title: &str, language: &str, body: &str) {
    if !lines.is_empty() {
        lines.push(String::new());
    }
    lines.push(format!("{title}:"));
    lines.push(format!("```{language}"));
    lines.extend(body.lines().map(str::to_owned));
    lines.push(String::from("```"));
}

fn render_help_card(
    name: &str,
    summary: &str,
    usage: &[&str],
    use_when: &[&str],
    reads: &[&str],
    writes: &[&str],
    does_not_execute: &[&str],
    outputs: &[&str],
    root_resolution: &[&str],
    config_examples: &[(&str, &str, &str)],
    failure_navigation: &[&str],
    examples: &[&str],
    see_also: &[&str],
) -> String {
    let mut lines = vec![format!("{name} - {summary}")];
    push_help_list_section(&mut lines, "Usage", usage);
    push_help_list_section(&mut lines, "Use when", use_when);
    push_help_list_section(&mut lines, "Reads", reads);
    push_help_list_section(&mut lines, "Writes", writes);
    push_help_list_section(&mut lines, "Does not execute", does_not_execute);
    push_help_list_section(&mut lines, "Outputs", outputs);
    push_help_list_section(&mut lines, "Root resolution", root_resolution);
    for (title, language, body) in config_examples {
        push_help_code_block(&mut lines, title, language, body);
    }
    push_help_list_section(&mut lines, "Failure navigation", failure_navigation);
    push_help_list_section(&mut lines, "Examples", examples);
    push_help_list_section(&mut lines, "See also", see_also);
    lines.join("\n")
}

fn general_help_text() -> String {
    let version_line = format!("chainbot {CHAINBOT_VERSION}");
    let mut lines = vec![String::from("ChainBot command skills")];
    push_help_list_section(&mut lines, "Version", &[version_line.as_str()]);
    push_help_list_section(
        &mut lines,
        "Usage",
        &[
            "chainbot help [command]",
            "chainbot version",
            "chainbot init",
            "chainbot status [--json]",
            "chainbot observe [--json] [--limit <n>] [--trigger-id <id>] [--run-id <id>]",
            "chainbot catalog list [--json] [--kind <builtin_node|builtin_trigger|plugin>]",
            "chainbot catalog show <reference> [--json]",
            "chainbot stop",
            "chainbot trigger list [--json]",
            "chainbot trigger <enable|disable> <trigger-id>",
            "chainbot validate",
            "chainbot list-runs",
            "chainbot run",
            "chainbot serve",
        ],
    );
    push_help_list_section(
        &mut lines,
        "Root resolution",
        &[
            "use CHAINBOT_CONFIG_DIR when it is set to a non-empty path",
            "otherwise fall back to ~/.chainbot",
            "root config must be <root>/chainbot.toml",
        ],
    );
    push_help_list_section(
        &mut lines,
        "Command catalog",
        &[
            "version    Print the running ChainBot version.",
            "init       Bootstrap a minimal ChainBot root.",
            "status     Inspect runtime state without executing workflows.",
            "observe    Inspect persisted trigger events, workflow logs, and runs.",
            "catalog    Discover builtin capabilities and installed plugin contracts.",
            "stop       Request graceful daemon shutdown.",
            "trigger    Inspect or persist trigger package state.",
            "validate   Validate config and package contracts.",
            "list-runs  Print persisted run summaries as JSON.",
            "run        Execute one single-shot manual run.",
            "serve      Start the background daemon control plane.",
        ],
    );
    push_help_list_section(
        &mut lines,
        "AI workflow hints",
        &[
            "start with `chainbot help <command>` before generating automation around a command",
            GENERAL_HELP_EXAMPLE_HINT,
            "prefer `chainbot status --json` and `chainbot trigger list --json` for machine-readable snapshots",
            "use `chainbot catalog list --json` when an agent needs a capability inventory",
            "use `chainbot catalog show <reference> --json` for one capability contract",
            "use `chainbot observe --json` when an agent needs recent persisted events or logs",
        ],
    );
    lines.join("\n")
}

fn help_text(topic: HelpTopic) -> String {
    match topic {
        HelpTopic::General => general_help_text(),
        HelpTopic::Version => render_help_card(
            "version",
            "Print the running ChainBot version",
            &["chainbot version", "chainbot --version"],
            &[
                "you need to confirm the installed CLI release",
                "you want to compare the binary version against root config metadata",
            ],
            &[],
            &[],
            &[],
            &["prints `chainbot <version>`"],
            &[],
            &[],
            &["if the reported version mismatches your root metadata, run `chainbot validate` next"],
            &["chainbot version", "chainbot --version"],
            &["init", "validate"],
        ),
        HelpTopic::Init => render_help_card(
            "init",
            "Bootstrap a minimal ChainBot root",
            &["chainbot init"],
            &[
                "you need a new ChainBot root that validates immediately",
                "you want the canonical single-file root config without manual setup",
                "you are preparing a fresh local root for workflows, triggers, plugins, secrets, and state",
            ],
            &[],
            &[
                "resolved root directory",
                "<root>/chainbot.toml when no root config exists yet",
                "default package directories under the resolved root",
            ],
            &["workflow runs", "trigger snapshots"],
            &[
                "prints created and reused bootstrap paths",
                "reuses existing canonical files and directories instead of overwriting them",
            ],
            &[
                "uses CHAINBOT_CONFIG_DIR when it is set to a non-empty path",
                "otherwise bootstraps ~/.chainbot",
            ],
            &[("Root config example", "toml", ROOT_CONFIG_EXAMPLE)],
            &[
                "directory/file collisions are reported with the exact path that blocks bootstrap",
                "invalid existing chainbot.toml is reported with file and line context",
            ],
            &["chainbot init", "CHAINBOT_CONFIG_DIR=/tmp/demo-root chainbot init"],
            &["validate", "status", "trigger"],
        ),
        HelpTopic::Status => render_help_card(
            "status",
            "Inspect runtime state without executing workflows",
            &["chainbot status", "chainbot status --json"],
            &[
                "you want to know whether serve is active",
                "you want the latest workflow run result without opening state files manually",
                "you want trigger activity and summary counts in one snapshot",
            ],
            &[
                "configured root config",
                "configured workflow packages",
                "configured trigger packages",
                "configured state runs directory",
                "configured trigger record directory",
                "configured coordination store",
            ],
            &[],
            &["workflow runs", "trigger snapshots", "runtime recovery"],
            &[
                "prints a human-readable Root / Workflows / Triggers / Summary snapshot by default",
                "prints stable JSON when `--json` is enabled",
            ],
            &[
                "resolve the root from CHAINBOT_CONFIG_DIR before reading workspace state",
                "return validation errors instead of partial snapshots when config is invalid",
            ],
            &[],
            &[
                "unexpected flags are reported with their argument position after `chainbot status`",
                "invalid root config is reported with file and line context before any runtime access",
            ],
            &[
                "chainbot status",
                "CHAINBOT_CONFIG_DIR=/tmp/demo-root chainbot status",
                "chainbot status --json",
            ],
            &["validate", "list-runs", "serve"],
        ),
        HelpTopic::Observe => render_help_card(
            "observe",
            "Inspect persisted trigger events, workflow logs, and run history",
            &[
                "chainbot observe",
                "chainbot observe --json",
                "chainbot observe --limit 20 --trigger-id tr-market",
                "chainbot observe --run-id manual-wf-alpha-1710000000000",
            ],
            &[
                "you need the recent persisted trigger-event stream without opening the database manually",
                "you want workflow log lines and run summaries that agree with runtime history",
                "you need to see whether older data has already moved into archive tables",
            ],
            &[
                "persisted run summaries",
                "persisted workflow runtime logs",
                "persisted trigger event records",
                "archive table counts when retention is enabled",
            ],
            &[],
            &["workflow execution", "trigger collection", "runtime recovery"],
            &[
                "prints recent runs, workflow logs, trigger events, and archive counts",
                "supports JSON output for automation and optional trigger/run filters",
            ],
            &[
                "resolve root config before reading runtime history",
                "respect the configured runtime storage backend without mutating active history",
            ],
            &[],
            &[
                "invalid `--limit` values are rejected with the exact argv position",
                "storage read failures surface as state errors without partial output",
            ],
            &[
                "chainbot observe",
                "chainbot observe --json --limit 5",
                "CHAINBOT_CONFIG_DIR=/tmp/demo-root chainbot observe --trigger-id tr-market",
            ],
            &["status", "list-runs", "serve", "catalog"],
        ),
        HelpTopic::Catalog => render_help_card(
            "catalog",
            "Discover builtin capabilities and installed plugin contracts",
            &[
                "chainbot catalog list [--json] [--kind <builtin_node|builtin_trigger|plugin>]",
                "chainbot catalog show <reference> [--json]",
            ],
            &[
                "you need builtin node or trigger inventory without opening source code",
                "you want installed plugin callable or event structure details",
                "an agent needs stable machine-readable capability discovery before generating config",
            ],
            &["builtin descriptors", "installed plugin manifests when a root is available"],
            &[],
            &[],
            &[
                "prints grouped human-readable capability lists by default",
                "prints stable JSON read models when `--json` is enabled",
            ],
            &[
                "builtin descriptors are always available",
                "installed plugins are loaded from the resolved root when one exists",
            ],
            &[],
            &[
                "malformed references are rejected with the expected `<kind>:<value>` format",
                "unknown references direct you back to `chainbot catalog list`",
            ],
            &[
                "chainbot catalog list",
                "chainbot catalog list --json --kind plugin",
                "chainbot catalog show plugin:quote-node-plugin",
            ],
            &["help", "status", "validate"],
        ),
        HelpTopic::Stop => render_help_card(
            "stop",
            "Request graceful daemon shutdown",
            &["chainbot stop"],
            &[
                "you want the running daemon to stop without using kill manually",
                "you need an operator-safe lifecycle command for automation",
            ],
            &["persisted daemon session state"],
            &["persisted daemon stop request state"],
            &[],
            &[
                "prints whether a shutdown was requested or no daemon was running",
                "waits briefly for the daemon to release its lease",
            ],
            &[
                "resolve root config before inspecting or updating daemon state",
                "return success when the daemon is already inactive or stale",
            ],
            &[],
            &[
                "timeout paths surface a stable daemon stop error code",
                "invalid root config is reported before any stop request is written",
            ],
            &["chainbot stop", "CHAINBOT_CONFIG_DIR=/tmp/demo-root chainbot stop"],
            &["serve", "status", "observe"],
        ),
        HelpTopic::Trigger => render_help_card(
            "trigger",
            "Inspect or persist trigger package state",
            &[
                "chainbot trigger list [--json]",
                "chainbot trigger enable <trigger-id>",
                "chainbot trigger disable <trigger-id>",
            ],
            &[
                "you need to inspect configured triggers without opening TOML manually",
                "you need to stop or re-enable a trigger without editing config by hand",
                "you want a machine-readable trigger inventory for automation",
            ],
            &["configured root config", "configured trigger packages"],
            &["target trigger package config.toml for enable or disable actions"],
            &["workflow runs", "trigger snapshots"],
            &[
                "prints a trigger table or JSON list for `list`",
                "prints the exact trigger config path touched by enable/disable",
            ],
            &[
                "resolve root config before loading trigger packages",
                "for `list`, only trigger package inputs are required beyond root config",
            ],
            &[("Trigger package example", "toml", TRIGGER_CONFIG_EXAMPLE)],
            &[
                "unsupported actions and extra arguments are reported with their exact argument position",
                "unknown trigger IDs direct you to `chainbot trigger list`",
            ],
            &[
                "chainbot trigger list",
                "chainbot trigger list --json",
                "chainbot trigger enable tr-market",
                "CHAINBOT_CONFIG_DIR=/tmp/demo-root chainbot trigger disable tr-market",
            ],
            &["init", "status", "validate", "serve", "catalog"],
        ),
        HelpTopic::Validate => render_help_card(
            "validate",
            "Validate config and package contracts",
            &["chainbot validate"],
            &[
                "you want to confirm a root is structurally valid",
                "you changed config and want a fast contract check before `run` or `serve`",
                "you want canonical examples for root, workflow, trigger, and plugin packages",
            ],
            &[
                "configured root config",
                "configured workflow packages",
                "configured trigger packages",
                "configured plugin packages",
            ],
            &[],
            &["workflow runs", "trigger snapshots"],
            &[
                "prints `validated root: <path>` on success",
                "prints file-aware validation diagnostics on failure",
            ],
            &[
                "load chainbot.toml first, then workflows, then triggers, then plugins",
                "reject absolute path overrides and any root-relative path that contains `..`",
            ],
            &[
                ("Root config example", "toml", ROOT_CONFIG_EXAMPLE),
                ("Workflow package example", "toml", WORKFLOW_CONFIG_EXAMPLE),
                ("Trigger package example", "toml", TRIGGER_CONFIG_EXAMPLE),
                ("Webhook trigger example", "toml", WEBHOOK_TRIGGER_CONFIG_EXAMPLE),
                ("WebSocket trigger example", "toml", WEBSOCKET_TRIGGER_CONFIG_EXAMPLE),
                ("Plugin package example", "toml", PLUGIN_CONFIG_EXAMPLE),
            ],
            &[
                "invalid TOML is reported with file path, line, column, and highlighted source",
                "semantic validation failures report the owning contract field or package identity",
            ],
            &[
                "chainbot validate",
                "CHAINBOT_CONFIG_DIR=/tmp/demo-root chainbot validate",
            ],
            &["status", "run", "serve", "catalog"],
        ),
        HelpTopic::ListRuns => render_help_card(
            "list-runs",
            "Print persisted run summaries as JSON",
            &["chainbot list-runs"],
            &[
                "you need machine-readable workflow run summaries",
                "you want raw persisted run status output without higher-level aggregation",
            ],
            &["configured state runs directory"],
            &[],
            &["workflow runs", "trigger snapshots"],
            &["prints a JSON array of persisted run summaries"],
            &[
                "resolve root config and runtime state before reading summaries",
                "runtime recovery is applied before listing persisted runs",
            ],
            &[],
            &[
                "unexpected flags are reported with their exact argument position",
                "serialization failures are surfaced as state errors",
            ],
            &[
                "chainbot list-runs",
                "CHAINBOT_CONFIG_DIR=/tmp/demo-root chainbot list-runs",
            ],
            &["status", "serve"],
        ),
        HelpTopic::Run => render_help_card(
            "run",
            "Execute one manual workflow run",
            &["chainbot run"],
            &[
                "you want a single manual execution without a serve lease",
                "your root contains exactly one workflow package or one inferable top-level workflow",
            ],
            &[
                "configured root config",
                "configured workflow packages",
                "configured plugin packages",
                "configured secrets directory",
            ],
            &[
                "configured state runs directory",
                "configured workflow log directory",
            ],
            &[],
            &[
                "prints run_id, workflow_id, and terminal status on success",
                "persists run summary and workflow log entries",
            ],
            &[
                "resolve root config before selecting a manual-run workflow",
                "manual run inference fails fast when multiple top-level workflows are present",
            ],
            &[("Workflow package example", "toml", WORKFLOW_CONFIG_EXAMPLE)],
            &[
                "top-level workflow inference failures are returned as usage errors",
                "execution failures surface the run_id so you can inspect state artifacts immediately",
            ],
            &[
                "chainbot run",
                "CHAINBOT_CONFIG_DIR=/tmp/demo-root chainbot run",
            ],
            &["status", "validate", "serve"],
        ),
        HelpTopic::Serve => render_help_card(
            "serve",
            "Start the background daemon control plane",
            &["chainbot serve"],
            &[
                "you want a long-running daemon that keeps trigger evaluation active",
                "you need a stable control-plane entrypoint for automation",
                "you want `status --json` and `observe` to read persisted daemon truth",
            ],
            &[
                "configured root config",
                "configured workflow packages",
                "configured trigger packages",
                "configured plugin packages",
                "configured secrets directory",
            ],
            &[
                "configured state runs directory",
                "configured workflow log directory",
                "configured trigger record directory",
                "configured coordination store",
            ],
            &[],
            &[
                "prints a start acknowledgement with daemon owner information",
                "returns conflict errors when another daemon already holds the lease",
            ],
            &[
                "acquire the daemon lease before spawning the background child",
                "the daemon reloads config and evaluates triggers on loop boundaries",
            ],
            &[
                ("Trigger package example", "toml", TRIGGER_CONFIG_EXAMPLE),
                ("Webhook trigger example", "toml", WEBHOOK_TRIGGER_CONFIG_EXAMPLE),
                ("WebSocket trigger example", "toml", WEBSOCKET_TRIGGER_CONFIG_EXAMPLE),
                ("Plugin package example", "toml", PLUGIN_CONFIG_EXAMPLE),
            ],
            &[
                "conflict paths expose a stable daemon-already-running error code",
                "daemon start failures release the preflight lease before returning",
            ],
            &[
                "chainbot serve",
                "CHAINBOT_CONFIG_DIR=/tmp/demo-root chainbot serve",
            ],
            &["status", "observe", "stop"],
        ),
    }
}

fn render_version_output() -> String {
    format!("chainbot {CHAINBOT_VERSION}")
}

fn build_status_output(
    root_layout: &RootLayout,
    profile: Option<String>,
    workflows: &[WorkflowDefinition],
    triggers: &[TriggerDefinition],
    plugins: &[PluginManifest],
    run_summaries: &[RunRecordSummary],
    trigger_snapshots: &[TriggerSnapshotRecord],
    daemon_status: RuntimeDaemonStatus,
) -> StatusOutput {
    let mut latest_runs = BTreeMap::<String, RunRecordSummary>::new();
    for summary in run_summaries {
        match latest_runs.get(&summary.workflow_id) {
            Some(current) if !run_summary_is_newer(summary, current) => {}
            _ => {
                latest_runs.insert(summary.workflow_id.clone(), summary.clone());
            }
        }
    }

    let latest_trigger_events = trigger_snapshots
        .iter()
        .map(|snapshot| (snapshot.trigger_id.clone(), snapshot.clone()))
        .collect::<BTreeMap<_, _>>();

    let workflow_views = workflows
        .iter()
        .map(|workflow| {
            let latest_run = latest_runs.get(&workflow.workflow_id);
            StatusWorkflowView {
                workflow_id: workflow.workflow_id.clone(),
                last_run_status: latest_run.map(|summary| summary.status),
                last_run_id: latest_run.map(|summary| summary.run_id.clone()),
                last_started_at_ms: latest_run.map(|summary| summary.started_at_ms),
                last_finished_at_ms: latest_run.and_then(|summary| summary.finished_at_ms),
            }
        })
        .collect::<Vec<_>>();

    let trigger_views = triggers
        .iter()
        .map(|trigger| {
            let latest_record = latest_trigger_events.get(&trigger.trigger_id);
            StatusTriggerView {
                trigger_id: trigger.trigger_id.clone(),
                enabled: trigger.enabled,
                workflow_id: trigger.workflow_id.clone(),
                last_event_id: latest_record.and_then(|record| record.last_event_id.clone()),
                last_accepted_at_ms: latest_record.and_then(|record| record.last_accepted_at_ms),
            }
        })
        .collect::<Vec<_>>();

    StatusOutput {
        root: StatusRootView {
            path: root_layout.root.display().to_string(),
            profile,
        },
        serve: StatusServeView {
            state: daemon_status.state,
            owner: daemon_status.owner_id,
            pid: daemon_status.pid,
            started_at_ms: daemon_status.started_at_ms,
            last_heartbeat_at_ms: daemon_status.last_heartbeat_at_ms,
            lease_expires_at_ms: daemon_status.lease_expires_at_ms,
            last_reload_at_ms: daemon_status.last_reload_at_ms,
            stop_requested_at_ms: daemon_status.stop_requested_at_ms,
            last_error_code: daemon_status.last_error_code,
            last_error_message: daemon_status.last_error_message,
        },
        workflows: workflow_views,
        triggers: trigger_views,
        plugins: build_status_plugin_summary(plugins),
        summary: StatusSummaryView {
            workflow_count: workflows.len(),
            trigger_count: triggers.len(),
            run_count: run_summaries.len(),
            running_run_count: run_summaries
                .iter()
                .filter(|summary| summary.status == RunStatus::Running)
                .count(),
        },
    }
}

fn render_status_output(status: &StatusOutput) -> String {
    let mut lines = vec![
        String::from("Root"),
        format!("  path: {}", status.root.path),
        format!(
            "  profile: {}",
            status.root.profile.as_deref().unwrap_or("none")
        ),
        format!("  serve: {}", render_serve_lease_state(status.serve.state)),
    ];

    if let Some(owner) = &status.serve.owner {
        lines.push(format!("  serve_owner: {owner}"));
    }
    if let Some(pid) = status.serve.pid {
        lines.push(format!("  serve_pid: {pid}"));
    }
    if let Some(started_at_ms) = status.serve.started_at_ms {
        lines.push(format!("  serve_started_at_ms: {started_at_ms}"));
    }
    if let Some(last_heartbeat_at_ms) = status.serve.last_heartbeat_at_ms {
        lines.push(format!(
            "  serve_last_heartbeat_at_ms: {last_heartbeat_at_ms}"
        ));
    }
    if let Some(lease_expires_at_ms) = status.serve.lease_expires_at_ms {
        lines.push(format!(
            "  serve_lease_expires_at_ms: {lease_expires_at_ms}"
        ));
    }
    if let Some(last_reload_at_ms) = status.serve.last_reload_at_ms {
        lines.push(format!("  serve_last_reload_at_ms: {last_reload_at_ms}"));
    }
    if let Some(stop_requested_at_ms) = status.serve.stop_requested_at_ms {
        lines.push(format!(
            "  serve_stop_requested_at_ms: {stop_requested_at_ms}"
        ));
    }
    if let Some(last_error_code) = &status.serve.last_error_code {
        lines.push(format!("  serve_last_error_code: {last_error_code}"));
    }
    if let Some(last_error_message) = &status.serve.last_error_message {
        lines.push(format!("  serve_last_error_message: {last_error_message}"));
    }

    lines.push(String::from("Workflows"));
    if status.workflows.is_empty() {
        lines.push(String::from("  none"));
    } else {
        for workflow in &status.workflows {
            lines.push(format!(
                "  {}  {}  last_run={}",
                workflow.workflow_id,
                workflow
                    .last_run_status
                    .map(render_run_status)
                    .unwrap_or("none"),
                workflow.last_run_id.as_deref().unwrap_or("none")
            ));
        }
    }

    lines.push(String::new());
    lines.push(String::from("Triggers"));
    if status.triggers.is_empty() {
        lines.push(String::from("  none"));
    } else {
        for trigger in &status.triggers {
            lines.push(format!(
                "  {}  {}  workflow={}  last_event={}",
                trigger.trigger_id,
                if trigger.enabled {
                    "enabled"
                } else {
                    "disabled"
                },
                trigger.workflow_id,
                trigger.last_event_id.as_deref().unwrap_or("none")
            ));
        }
    }

    lines.push(String::new());
    lines.push(String::from("Plugins"));
    lines.push(format!(
        "  installed={} builtin={} external_node={} external_trigger={}",
        status.plugins.installed_count,
        status.plugins.builtin_count,
        status.plugins.external_node_count,
        status.plugins.external_trigger_count
    ));
    lines.push(String::from(
        "  use `chainbot catalog list` for capability details",
    ));

    lines.push(String::new());
    lines.push(String::from("Summary"));
    lines.push(format!(
        "  workflows={} triggers={} runs={} running={}",
        status.summary.workflow_count,
        status.summary.trigger_count,
        status.summary.run_count,
        status.summary.running_run_count
    ));

    lines.join("\n")
}

fn parse_catalog_filter_kind(
    value: &str,
    position: usize,
) -> Result<CatalogFilterKind, UserFacingError> {
    CatalogFilterKind::parse(value).ok_or_else(|| {
        UserFacingError::usage(format!(
            "Unsupported --kind value at argument #{position} after `chainbot catalog list`: `{value}`. Use builtin_node, builtin_trigger, or plugin."
        ))
    })
}

fn load_catalog_plugins_strict() -> Result<Vec<PluginManifest>, UserFacingError> {
    let root_layout = resolve_root_layout().map_err(UserFacingError::from_contract)?;
    let bundle =
        RootDefinitionBundle::load(&root_layout).map_err(UserFacingError::from_contract)?;
    Ok(bundle.plugins)
}

fn load_catalog_plugins_if_available() -> Result<Vec<PluginManifest>, UserFacingError> {
    let root_layout = resolve_root_layout().map_err(UserFacingError::from_contract)?;
    match RootDefinitionBundle::load(&root_layout) {
        Ok(bundle) => Ok(bundle.plugins),
        Err(crate::errors::ContractError::MissingDirectory { kind: "root", .. })
        | Err(crate::errors::ContractError::MissingFile {
            kind: "root config",
            ..
        }) => Ok(Vec::new()),
        Err(error) => Err(UserFacingError::from_contract(error)),
    }
}

fn render_observe_output(output: &ObserveOutput) -> String {
    let mut lines = vec![
        String::from("Observe"),
        format!("  requested_limit: {}", output.summary.requested_limit),
        format!(
            "  archived_runs: {} archived_logs: {} archived_trigger_events: {}",
            output.summary.archived.run_summaries,
            output.summary.archived.workflow_logs,
            output.summary.archived.trigger_events
        ),
        String::new(),
        String::from("Runs"),
    ];

    if output.runs.is_empty() {
        lines.push(String::from("  none"));
    } else {
        for run in &output.runs {
            lines.push(format!(
                "  {}  workflow={}  status={}  started_at_ms={}",
                run.run_id,
                run.workflow_id,
                render_run_status(run.status),
                run.started_at_ms
            ));
        }
    }

    lines.push(String::new());
    lines.push(String::from("Workflow Logs"));
    if output.workflow_logs.is_empty() {
        lines.push(String::from("  none"));
    } else {
        for entry in &output.workflow_logs {
            lines.push(format!(
                "  {}#{}  {}  occurred_at_ms={}  {}",
                entry.run_id, entry.sequence, entry.event, entry.occurred_at_ms, entry.message
            ));
        }
    }

    lines.push(String::new());
    lines.push(String::from("Trigger Events"));
    if output.trigger_events.is_empty() {
        lines.push(String::from("  none"));
    } else {
        for event in &output.trigger_events {
            lines.push(format!(
                "  {}#{}  workflow={}  event_id={}  accepted_at_ms={}",
                event.trigger_id,
                event.sequence,
                event.workflow_id,
                event.event_id,
                event.accepted_at_ms
            ));
        }
    }

    lines.join("\n")
}

fn run_summary_is_newer(candidate: &RunRecordSummary, current: &RunRecordSummary) -> bool {
    (
        candidate.started_at_ms,
        candidate.finished_at_ms.unwrap_or(i64::MIN),
        &candidate.run_id,
    ) > (
        current.started_at_ms,
        current.finished_at_ms.unwrap_or(i64::MIN),
        &current.run_id,
    )
}

fn render_serve_lease_state(state: ServeLeaseState) -> &'static str {
    match state {
        ServeLeaseState::Idle => "idle",
        ServeLeaseState::Active => "active",
        ServeLeaseState::Stale => "stale",
    }
}

fn parse_bool_flag_value(
    flag_name: &str,
    value: &str,
    position: usize,
    command_path: &str,
) -> Result<bool, UserFacingError> {
    match value {
        "true" | "1" => Ok(true),
        "false" | "0" => Ok(false),
        other => Err(UserFacingError::usage(format!(
            "Unsupported {flag_name} value at argument #{position} after `{command_path}`: `{other}`. Use true, false, 1, or 0."
        ))),
    }
}

fn parse_observe_limit(value: &str, position: usize) -> Result<usize, UserFacingError> {
    let parsed = value.parse::<usize>().map_err(|_| {
        UserFacingError::usage(format!(
            "Unsupported --limit value at argument #{position} after `chainbot observe`: `{value}`. Use a positive integer."
        ))
    })?;
    if parsed == 0 {
        return Err(UserFacingError::usage(format!(
            "Unsupported --limit value at argument #{position} after `chainbot observe`: `{value}`. Use a positive integer."
        )));
    }
    Ok(parsed)
}

fn unsupported_command_message(value: &str) -> String {
    match suggest_command(value) {
        Some(suggestion) => format!(
            "Unsupported command at argv[1]: `{value}`. Did you mean `{suggestion}`? Run `chainbot help` to see available command skills."
        ),
        None => format!(
            "Unsupported command at argv[1]: `{value}`. Run `chainbot help` to see available command skills."
        ),
    }
}

fn suggest_command(value: &str) -> Option<&'static str> {
    let normalized = value.trim();
    if normalized.is_empty() {
        return None;
    }

    let mut best_match = None;
    let mut best_distance = usize::MAX;

    for candidate in [
        "help",
        "version",
        "status",
        "observe",
        "init",
        "trigger",
        "validate",
        "list-runs",
        "run",
        "serve",
    ] {
        let distance = levenshtein_distance(normalized, candidate);
        if distance < best_distance {
            best_distance = distance;
            best_match = Some(candidate);
        }
    }

    match (best_match, best_distance) {
        (Some(candidate), distance) if distance <= 3 => Some(candidate),
        _ => None,
    }
}

fn levenshtein_distance(left: &str, right: &str) -> usize {
    if left == right {
        return 0;
    }

    let left_chars = left.chars().collect::<Vec<_>>();
    let right_chars = right.chars().collect::<Vec<_>>();
    let mut costs = (0..=right_chars.len()).collect::<Vec<_>>();

    for (left_index, left_char) in left_chars.iter().enumerate() {
        let mut previous_diagonal = costs[0];
        costs[0] = left_index + 1;
        for (right_index, right_char) in right_chars.iter().enumerate() {
            let insertion = costs[right_index + 1] + 1;
            let deletion = costs[right_index] + 1;
            let substitution = previous_diagonal + usize::from(left_char != right_char);
            previous_diagonal = costs[right_index + 1];
            costs[right_index + 1] = insertion.min(deletion).min(substitution);
        }
    }

    costs[right_chars.len()]
}

fn render_trigger_operation_output(result: &TriggerToggleResult) -> String {
    let state = if result.enabled {
        "enabled"
    } else {
        "disabled"
    };
    let outcome = if result.changed {
        "updated"
    } else {
        "unchanged"
    };
    format!(
        "trigger {outcome}: trigger_id={} workflow_id={} state={} config={}",
        result.trigger_id,
        result.workflow_id,
        state,
        result.config_path.display()
    )
}

fn render_trigger_list_output(triggers: &[TriggerDefinition]) -> String {
    let mut lines = vec![String::from("Triggers")];
    if triggers.is_empty() {
        lines.push(String::from("  none"));
        return lines.join("\n");
    }

    for trigger in triggers {
        lines.push(format!(
            "  {}  {}  workflow={}  kind={}  source={}",
            trigger.trigger_id,
            if trigger.enabled {
                "enabled"
            } else {
                "disabled"
            },
            trigger.workflow_id,
            trigger.kind,
            trigger.source
        ));
    }

    lines.join("\n")
}

fn initialize_root_layout(layout: &RootLayout) -> Result<InitResult, UserFacingError> {
    let mut created_paths = Vec::new();
    let mut reused_paths = Vec::new();

    ensure_directory(&layout.root, &mut created_paths, &mut reused_paths)?;
    let root_config = ensure_root_config(layout, &mut created_paths, &mut reused_paths)?;
    let effective_layout = layout
        .apply_root_config(&root_config)
        .map_err(UserFacingError::from_contract)?;

    ensure_directory(
        &effective_layout.workflows_dir,
        &mut created_paths,
        &mut reused_paths,
    )?;
    ensure_directory(
        &effective_layout.triggers_dir,
        &mut created_paths,
        &mut reused_paths,
    )?;
    ensure_directory(
        &effective_layout.plugins_dir,
        &mut created_paths,
        &mut reused_paths,
    )?;
    ensure_directory(
        &effective_layout.plugins_dir.join("bin"),
        &mut created_paths,
        &mut reused_paths,
    )?;
    ensure_directory(
        &effective_layout.secrets_dir,
        &mut created_paths,
        &mut reused_paths,
    )?;
    ensure_directory(
        &effective_layout.state_dir,
        &mut created_paths,
        &mut reused_paths,
    )?;

    Ok(InitResult {
        root: layout.root.clone(),
        created_paths,
        reused_paths,
    })
}

fn ensure_directory(
    path: &Path,
    created_paths: &mut Vec<PathBuf>,
    reused_paths: &mut Vec<PathBuf>,
) -> Result<(), UserFacingError> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_dir() => {
            reused_paths.push(path.to_path_buf());
            Ok(())
        }
        Ok(_) => Err(UserFacingError::conflict(format!(
            "Init expected a directory at {}, but found a file. Move it or choose another CHAINBOT_CONFIG_DIR.",
            path.display()
        ))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir_all(path).map_err(|source| {
                UserFacingError::state(format!(
                    "Failed to create init directory {}: {source}",
                    path.display()
                ))
            })?;
            created_paths.push(path.to_path_buf());
            Ok(())
        }
        Err(error) => Err(UserFacingError::state(format!(
            "Failed to inspect init path {}: {error}",
            path.display()
        ))),
    }
}

fn ensure_root_config(
    layout: &RootLayout,
    created_paths: &mut Vec<PathBuf>,
    reused_paths: &mut Vec<PathBuf>,
) -> Result<RootConfigDefinition, UserFacingError> {
    let path = layout.root_config_path();
    match fs::metadata(&path) {
        Ok(metadata) if metadata.is_file() => {
            reused_paths.push(path.clone());
            let contents = fs::read_to_string(&path).map_err(|source| {
                UserFacingError::state(format!(
                    "Failed to read init root config at {}: {source}",
                    path.display()
                ))
            })?;
            let root_config = toml::from_str::<RootConfigDefinition>(&contents).map_err(
                |source| {
                    UserFacingError::from_contract(crate::errors::ContractError::toml_decode(
                        path.clone(),
                        &contents,
                        source,
                    ))
                },
            )?;
            root_config
                .validate()
                .map_err(UserFacingError::from_contract)?;
            Ok(root_config)
        }
        Ok(_) => Err(UserFacingError::conflict(format!(
            "Init expected a file at {}, but found a directory. Remove it or choose another CHAINBOT_CONFIG_DIR.",
            path.display()
        ))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let root_config = default_root_config();
            let contents = toml::to_string_pretty(&root_config).map_err(|source| {
                UserFacingError::state(format!(
                    "Failed to serialize init root config at {}: {source}",
                    path.display()
                ))
            })?;
            fs::write(&path, contents).map_err(|source| {
                UserFacingError::state(format!(
                    "Failed to write init root config at {}: {source}",
                    path.display()
                ))
            })?;
            created_paths.push(path);
            Ok(root_config)
        }
        Err(error) => Err(UserFacingError::state(format!(
            "Failed to inspect init root config {}: {error}",
            path.display()
        ))),
    }
}

fn default_root_config() -> RootConfigDefinition {
    RootConfigDefinition {
        schema_version: INIT_MANIFEST_VERSION.to_owned(),
        chainbot_version: Some(CHAINBOT_VERSION.to_owned()),
        profile: Some(String::from("default")),
        secret_refs: Vec::new(),
        runtime_defaults: BTreeMap::from([(
            String::from("timezone"),
            serde_json::Value::String(String::from("UTC")),
        )]),
        paths: RootPathOverrides {
            workflows_dir: Some(String::from("workflows")),
            triggers_dir: Some(String::from("triggers")),
            plugins_dir: Some(String::from("plugins")),
            secrets_dir: Some(String::from("secrets")),
            state_dir: Some(String::from("state")),
        },
        storage: StorageDefinition {
            mode: StorageMode::Local,
            local: Some(LocalStorageDefinition {
                database_path: Some(String::from("state/runtime.sqlite3")),
            }),
            postgres: None,
            retention: RuntimeHistoryRetentionDefinition::default(),
            raw_debug: RawDebugArtifactsDefinition::default(),
        },
    }
}

fn render_init_output(result: &InitResult) -> String {
    let mut lines = vec![format!("init completed: root={}", result.root.display())];
    if result.created_paths.is_empty() {
        lines.push(String::from("created:"));
        lines.push(String::from("  none"));
    } else {
        lines.push(String::from("created:"));
        for path in &result.created_paths {
            lines.push(format!("  {}", path.display()));
        }
    }

    if !result.reused_paths.is_empty() {
        lines.push(String::from("reused:"));
        for path in &result.reused_paths {
            lines.push(format!("  {}", path.display()));
        }
    }

    lines.join("\n")
}

fn current_time_ms() -> Result<i64, UserFacingError> {
    let duration = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|_| {
        UserFacingError::state("System clock is before UNIX_EPOCH; cannot continue.")
    })?;
    Ok(duration.as_millis().min(i64::MAX as u128) as i64)
}

fn serve_start_ack_timeout_ms() -> u64 {
    std::env::var(CHAINBOT_TEST_DAEMON_START_ACK_TIMEOUT_MS_ENV)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(SERVE_START_ACK_TIMEOUT_MS)
}

fn maybe_delay_daemon_start_for_tests() -> Result<(), UserFacingError> {
    let Some(delay_ms) = std::env::var(CHAINBOT_TEST_DAEMON_START_DELAY_MS_ENV)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
    else {
        return Ok(());
    };
    thread::sleep(std::time::Duration::from_millis(delay_ms));
    Ok(())
}

fn map_runtime_state_error(action: &str, error: RuntimeStateError) -> UserFacingError {
    UserFacingError::state(format!("Failed to {action}: {error}"))
}

fn map_trigger_error(error: TriggerPlaneError) -> UserFacingError {
    match error {
        TriggerPlaneError::Contract(source) => UserFacingError::from_contract(source),
        TriggerPlaneError::RuntimeState(source) => {
            map_runtime_state_error("evaluate trigger-plane runtime state", source)
        }
    }
}

fn map_ingress_error(error: IngressRuntimeError) -> UserFacingError {
    match error {
        IngressRuntimeError::RuntimeState(source) => {
            map_runtime_state_error("operate ingress runtime state", source)
        }
        other => UserFacingError::state_with_code("ingress_runtime_error", other.to_string()),
    }
}

fn maybe_prepare_e2e_root(layout: &RootLayout) -> Result<(), UserFacingError> {
    if layout.root.exists() {
        return Ok(());
    }

    let e2e_suffix = Path::new("target").join("test-roots").join("e2e");
    if !layout.root.ends_with(&e2e_suffix) {
        return Ok(());
    }

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let fixture_source = manifest_dir
        .join("tests")
        .join("fixtures")
        .join("e2e")
        .join("success");

    copy_directory_recursive(&fixture_source, &layout.root).map_err(|source| {
        UserFacingError::state(format!("Failed to prepare e2e fixture root: {source}"))
    })
}

fn copy_directory_recursive(source: &Path, destination: &Path) -> Result<(), std::io::Error> {
    fs::create_dir_all(destination)?;
    let entries = fs::read_dir(source)?;
    for entry in entries {
        let entry = entry?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());

        if source_path.is_dir() {
            copy_directory_recursive(&source_path, &destination_path)?;
        } else {
            if let Some(parent) = destination_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(&source_path, &destination_path)?;
        }
    }

    Ok(())
}

fn spawn_serve_daemon_process(owner_id: &str) -> Result<std::process::Child, UserFacingError> {
    std::process::Command::new(std::env::current_exe().map_err(|source| {
        UserFacingError::state_with_code(
            "daemon_start_failed",
            format!("Failed to resolve the running chainbot binary for daemon spawn: {source}"),
        )
    })?)
    .args(["__serve-daemon", "--owner-id", owner_id])
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .spawn()
    .map_err(|source| {
        UserFacingError::state_with_code(
            "daemon_start_failed",
            format!("Failed to spawn the background daemon process: {source}"),
        )
    })
}

fn wait_for_daemon_start(
    storage_config: &RuntimeStorageConfig,
    owner_id: &str,
    child_pid: u32,
    started_at_ms: i64,
) -> Result<(), UserFacingError> {
    let deadline_ms = started_at_ms.saturating_add(serve_start_ack_timeout_ms() as i64);
    while current_time_ms()? <= deadline_ms {
        let observed_at_ms = current_time_ms()?;
        let mut store = RuntimeStateStore::open(storage_config, observed_at_ms)
            .map_err(|error| map_runtime_state_error("open runtime state store", error))?;
        let daemon_status = store
            .inspect_daemon_status(observed_at_ms)
            .map_err(|error| map_runtime_state_error("inspect daemon status", error))?;
        if daemon_status.owner_id.as_deref() == Some(owner_id)
            && daemon_status.pid == Some(i64::from(child_pid))
            && daemon_status.started_at_ms == Some(started_at_ms)
            && daemon_status
                .last_heartbeat_at_ms
                .is_some_and(|heartbeat_at_ms| heartbeat_at_ms > started_at_ms)
        {
            return Ok(());
        }
        thread::sleep(std::time::Duration::from_millis(50));
    }

    Err(UserFacingError::unavailable_with_code(
        "daemon_start_failed",
        "The background daemon did not become observable before the start acknowledgement timeout.",
    ))
}

fn wait_for_daemon_stop(storage_config: &RuntimeStorageConfig) -> Result<(), UserFacingError> {
    let started_wait_ms = current_time_ms()?;
    let deadline_ms = started_wait_ms.saturating_add(SERVE_STOP_TIMEOUT_MS as i64);
    while current_time_ms()? <= deadline_ms {
        let observed_at_ms = current_time_ms()?;
        let mut store = RuntimeStateStore::open(storage_config, observed_at_ms)
            .map_err(|error| map_runtime_state_error("open runtime state store", error))?;
        let daemon_status = store
            .inspect_daemon_status(observed_at_ms)
            .map_err(|error| map_runtime_state_error("inspect daemon status", error))?;
        if daemon_status.state == ServeLeaseState::Idle {
            return Ok(());
        }
        thread::sleep(std::time::Duration::from_millis(100));
    }

    Err(UserFacingError::unavailable_with_code(
        "daemon_stop_timeout",
        "Timed out while waiting for the daemon to stop gracefully.",
    ))
}

fn run_internal_serve_daemon_loop(
    request: &CliRequest,
    storage_config: &RuntimeStorageConfig,
    owner_id: &str,
    pid: i64,
) -> Result<(), UserFacingError> {
    let ingress_supervisor =
        TriggerIngressSupervisor::start(storage_config.clone()).map_err(map_ingress_error)?;
    let mut lease_supervisor = ServeLeaseSupervisor::new(
        storage_config.clone(),
        owner_id.to_owned(),
        pid,
        current_time_ms()?,
    );

    let loop_result = (|| loop {
        let observed_at_ms = current_time_ms()?;
        let mut control_store = RuntimeStateStore::open(storage_config, observed_at_ms)
            .map_err(|error| map_runtime_state_error("open runtime state store", error))?;
        if control_store
            .daemon_stop_requested(owner_id)
            .map_err(|error| map_runtime_state_error("inspect daemon stop request", error))?
        {
            return Ok(());
        }

        lease_supervisor.maybe_renew(observed_at_ms)?;

        let mut runtime = match request.load_runtime_context() {
            Ok(runtime) => runtime,
            Err(error) => {
                let _ = ingress_supervisor.reconcile(DesiredIngressState::default());
                let mut error_store = RuntimeStateStore::open(storage_config, current_time_ms()?)
                    .map_err(|open_error| {
                    map_runtime_state_error("open runtime state store", open_error)
                })?;
                error_store
                    .record_daemon_error(owner_id, error.error_code(), &error.to_string())
                    .map_err(|store_error| {
                        map_runtime_state_error("record daemon error", store_error)
                    })?;
                thread::sleep(std::time::Duration::from_millis(SERVE_ERROR_BACKOFF_MS));
                continue;
            }
        };
        let desired_ingress = build_desired_ingress_state(&runtime.definitions.triggers)
            .map_err(UserFacingError::from_contract)?;
        ingress_supervisor
            .reconcile(desired_ingress)
            .map_err(map_ingress_error)?;
        let mut iteration_store = RuntimeStateStore::open(&runtime.storage_config, observed_at_ms)
            .map_err(|error| map_runtime_state_error("open runtime state store", error))?;
        iteration_store
            .mark_daemon_reload(owner_id, observed_at_ms)
            .map_err(|error| map_runtime_state_error("mark daemon reload", error))?;

        match serve_once_with_lease(&mut runtime, observed_at_ms, &mut lease_supervisor) {
            Ok(_) => {
                thread::sleep(std::time::Duration::from_millis(
                    SERVE_IDLE_POLL_INTERVAL_MS,
                ));
            }
            Err(error) => {
                let mut error_store = RuntimeStateStore::open(storage_config, current_time_ms()?)
                    .map_err(|open_error| {
                    map_runtime_state_error("open runtime state store", open_error)
                })?;
                error_store
                    .record_daemon_error(owner_id, error.error_code(), &error.to_string())
                    .map_err(|store_error| {
                        map_runtime_state_error("record daemon error", store_error)
                    })?;
                thread::sleep(std::time::Duration::from_millis(SERVE_ERROR_BACKOFF_MS));
            }
        }
    })();

    let shutdown_result = ingress_supervisor.shutdown().map_err(map_ingress_error);
    loop_result.and(shutdown_result)
}

fn serve_once_with_lease(
    runtime: &mut RuntimeContext,
    accepted_at_ms: i64,
    lease_supervisor: &mut ServeLeaseSupervisor,
) -> Result<CliOutput, UserFacingError> {
    lease_supervisor.maybe_renew(accepted_at_ms)?;
    let replay_requests =
        load_replayable_trigger_requests(runtime, REPLAYABLE_TRIGGER_BATCH_LIMIT)?;
    let trigger_manifests = runtime
        .definitions
        .plugins
        .iter()
        .filter_map(|manifest| {
            manifest
                .kind()
                .ok()
                .filter(|kind| *kind == PluginKind::ExternalTrigger)
                .map(|_| manifest.clone())
        })
        .collect::<Vec<_>>();
    let policy = build_trigger_host_policy(&trigger_manifests, &runtime.root_layout.plugins_dir);
    let builtin_events =
        build_builtin_trigger_emissions(&runtime.definitions.triggers, accepted_at_ms)
            .map_err(UserFacingError::from_contract)?;
    let drained_ingress = drain_ingress_emissions(
        &mut runtime.state_store,
        &runtime.definitions.triggers,
        INGRESS_INBOX_BATCH_LIMIT,
    )
    .map_err(map_trigger_error)?;
    let mut builtin_events = builtin_events;
    for (trigger_id, mut ingress_events) in drained_ingress.emissions {
        builtin_events
            .entry(trigger_id)
            .or_default()
            .append(&mut ingress_events);
    }

    let trigger_store = RuntimeStateStore::open(&runtime.storage_config, accepted_at_ms)
        .map_err(|error| map_runtime_state_error("open trigger runtime state store", error))?;

    let mut trigger_plane = TriggerPlane::open_with_store(
        trigger_store,
        runtime.definitions.triggers.clone(),
        trigger_manifests,
        policy,
        builtin_events,
    )
    .map_err(map_trigger_error)?;

    let mut renew_progress = || {
        let now_ms = current_time_ms().map_err(|error| {
            TriggerPlaneError::Contract(crate::errors::ContractError::InvalidTriggerEmission {
                trigger_id: String::from("serve"),
                detail: error.to_string(),
            })
        })?;
        lease_supervisor.maybe_renew(now_ms).map_err(|error| {
            TriggerPlaneError::Contract(crate::errors::ContractError::InvalidTriggerEmission {
                trigger_id: String::from("serve"),
                detail: error.to_string(),
            })
        })
    };
    let run_requests = trigger_plane
        .collect_run_requests_with_progress(accepted_at_ms, &mut renew_progress)
        .map_err(map_trigger_error)?;
    for inbox_id in drained_ingress.inbox_ids {
        runtime
            .state_store
            .mark_ingress_inbox_processed(&inbox_id, accepted_at_ms)
            .map_err(|error| map_runtime_state_error("mark ingress inbox processed", error))?;
    }
    if run_requests.is_empty() {
        if replay_requests.is_empty() {
            return Ok(CliOutput::text(
                "serve completed: no accepted trigger events",
            ));
        }
    }

    let run_requests = merge_trigger_requests(replay_requests, run_requests);

    let mut completed_runs = Vec::with_capacity(run_requests.len());
    let mut failures = Vec::new();

    for trigger_request in &run_requests {
        lease_supervisor.maybe_renew(current_time_ms()?)?;
        let normalized_request = normalized_request_from_trigger(trigger_request);
        match execute_single_run(runtime, normalized_request, accepted_at_ms) {
            Ok(run_result) => completed_runs.push((trigger_request, run_result)),
            Err(error) => failures.push(format!(
                "trigger_id={} event_id={} workflow_id={} error={error}",
                trigger_request.trigger_id, trigger_request.event_id, trigger_request.workflow_id
            )),
        }
    }

    if !failures.is_empty() {
        return Err(UserFacingError::unavailable(format!(
            "Serve evaluated {} accepted trigger event(s) and observed {} failure(s): {}",
            run_requests.len(),
            failures.len(),
            failures.join("; ")
        )));
    }

    let mut output_lines = Vec::with_capacity(completed_runs.len().saturating_add(1));
    output_lines.push(format!(
        "serve completed: executed {} accepted trigger event(s)",
        completed_runs.len()
    ));
    for (trigger_request, run_result) in completed_runs {
        output_lines.push(format!(
            "serve run: run_id={} workflow_id={} status={} trigger_id={} event_id={}",
            run_result.run_id,
            run_result.workflow_id,
            render_run_status(run_result.status),
            trigger_request.trigger_id,
            trigger_request.event_id
        ));
    }

    Ok(CliOutput::text(output_lines.join("\n")))
}

fn execute_single_run(
    runtime: &mut RuntimeContext,
    request: NormalizedRunRequest,
    started_at_ms: i64,
) -> Result<SingleRunResult, UserFacingError> {
    let run_id = request.run_id.clone();
    let workflow_id = request.workflow_id.clone();

    write_run_status(
        &mut runtime.state_store,
        &run_id,
        &workflow_id,
        RunStatus::Running,
        started_at_ms,
        None,
    )
    .map_err(|error| map_runtime_state_error("persist running run summary", error))?;

    write_log_entry(
        &mut runtime.state_store,
        &run_id,
        "run_started",
        "run accepted for execution",
        started_at_ms,
    )
    .map_err(|error| map_runtime_state_error("write run_started log", error))?;

    let plane = build_execution_plane(runtime)?;
    let report = plane.execute(&request);
    let finished_at_ms = current_time_ms()?;

    match report {
        Ok(report) => {
            let status = if report.status == WorkflowRunStatus::Succeeded {
                RunStatus::Succeeded
            } else {
                RunStatus::Failed
            };

            write_log_entry(
                &mut runtime.state_store,
                &run_id,
                "run_finished",
                &format!(
                    "execution finished with {} and {} node failure(s)",
                    render_run_status(status),
                    report.node_failures.len()
                ),
                finished_at_ms,
            )
            .map_err(|error| map_runtime_state_error("write run_finished log", error))?;

            write_run_status(
                &mut runtime.state_store,
                &run_id,
                &workflow_id,
                status,
                started_at_ms,
                Some(finished_at_ms),
            )
            .map_err(|error| map_runtime_state_error("persist finished run summary", error))?;

            if status == RunStatus::Failed {
                return Err(UserFacingError::unavailable(format!(
                    "Run {run_id} finished with failed node(s): {}",
                    report
                        .node_failures
                        .keys()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", ")
                )));
            }

            Ok(SingleRunResult {
                run_id,
                workflow_id,
                status,
            })
        }
        Err(error) => {
            let detail = error.to_string();
            write_log_entry(
                &mut runtime.state_store,
                &run_id,
                "run_failed",
                &detail,
                finished_at_ms,
            )
            .map_err(|write_error| map_runtime_state_error("write run_failed log", write_error))?;

            write_run_status(
                &mut runtime.state_store,
                &run_id,
                &workflow_id,
                RunStatus::Failed,
                started_at_ms,
                Some(finished_at_ms),
            )
            .map_err(|write_error| {
                map_runtime_state_error("persist failed run summary", write_error)
            })?;

            Err(UserFacingError::unavailable(format!(
                "Run {run_id} failed: {detail}"
            )))
        }
    }
}

fn build_execution_plane(runtime: &RuntimeContext) -> Result<ExecutionPlane, UserFacingError> {
    let registry = build_builtin_registry(BuiltinRuntimeContext {
        root_layout: runtime.root_layout.clone(),
        secret_mode: runtime.secret_mode,
        worker_host: runtime.worker_host.clone(),
    });

    ExecutionPlane::with_plugin_runtime(
        runtime.definitions.workflows.clone(),
        runtime.definitions.root_config.runtime_defaults.clone(),
        registry,
        runtime.definitions.plugins.clone(),
        runtime.root_layout.plugins_dir.clone(),
        runtime.root_layout.secrets_dir.clone(),
        runtime.secret_mode,
    )
    .map_err(UserFacingError::from_contract)
}

fn build_trigger_host_policy(
    manifests: &[PluginManifest],
    plugins_root_dir: &Path,
) -> TriggerPluginHostPolicy {
    let allowlisted_plugin_ids = manifests
        .iter()
        .filter_map(|manifest| {
            manifest
                .kind()
                .ok()
                .filter(|kind| *kind == PluginKind::ExternalTrigger)
                .map(|_| manifest.plugin_id.clone())
        })
        .collect();

    TriggerPluginHostPolicy {
        allowlisted_plugin_ids,
        allowed_capabilities: BTreeSet::from([REQUIRED_TRIGGER_PLUGIN_CAPABILITY.to_owned()]),
        plugin_root_dir: plugins_root_dir.to_path_buf(),
    }
}

fn normalized_request_from_trigger(trigger_request: &TriggerRunRequest) -> NormalizedRunRequest {
    let mut request = NormalizedRunRequest::new(
        trigger_request.run_id.clone(),
        trigger_request.workflow_id.clone(),
    );

    match &trigger_request.payload {
        serde_json::Value::Object(values) => {
            request.trigger_payload_mapping.extend(
                values
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone())),
            );
        }
        other => {
            request
                .trigger_payload_mapping
                .insert(String::from("payload"), other.clone());
        }
    }

    request
}

fn load_replayable_trigger_requests(
    runtime: &mut RuntimeContext,
    limit: usize,
) -> Result<Vec<TriggerRunRequest>, UserFacingError> {
    runtime
        .state_store
        .list_replayable_trigger_records(limit)
        .map_err(|error| map_runtime_state_error("list replayable trigger records", error))
        .map(|records| {
            records
                .into_iter()
                .map(replay_trigger_request_from_record)
                .collect()
        })
}

fn replay_trigger_request_from_record(record: TriggerEventRecord) -> TriggerRunRequest {
    TriggerRunRequest {
        run_id: record.run_id,
        workflow_id: record.workflow_id,
        trigger_id: record.trigger_id.clone(),
        event_id: record.event_id,
        source: record.source,
        accepted_at_ms: record.accepted_at_ms,
        payload: record.payload,
        trigger_record_ref: format!(
            "db://trigger_event_records/{}/{}",
            record.trigger_id, record.sequence
        ),
    }
}

fn merge_trigger_requests(
    replay_requests: Vec<TriggerRunRequest>,
    new_requests: Vec<TriggerRunRequest>,
) -> Vec<TriggerRunRequest> {
    let mut merged = Vec::with_capacity(replay_requests.len().saturating_add(new_requests.len()));
    let mut seen_run_ids = BTreeSet::new();

    for request in replay_requests.into_iter().chain(new_requests) {
        if seen_run_ids.insert(request.run_id.clone()) {
            merged.push(request);
        }
    }

    merged
}

fn write_run_status(
    state_store: &mut RuntimeStateStore,
    run_id: &str,
    workflow_id: &str,
    status: RunStatus,
    started_at_ms: i64,
    finished_at_ms: Option<i64>,
) -> Result<(), RuntimeStateError> {
    state_store.write_run_summary(&RunRecordSummary {
        schema_version: "1.0.0".to_owned(),
        run_id: run_id.to_owned(),
        workflow_id: workflow_id.to_owned(),
        status,
        started_at_ms,
        finished_at_ms,
    })
}

fn write_log_entry(
    state_store: &mut RuntimeStateStore,
    run_id: &str,
    event: &str,
    message: &str,
    occurred_at_ms: i64,
) -> Result<u64, RuntimeStateError> {
    state_store.append_workflow_log_entry(run_id, event, message, occurred_at_ms)
}

fn render_run_status(status: RunStatus) -> &'static str {
    match status {
        RunStatus::Pending => "pending",
        RunStatus::Running => "running",
        RunStatus::Succeeded => "succeeded",
        RunStatus::Failed => "failed",
    }
}

fn secret_decrypt_mode_from_env() -> SecretDecryptMode {
    match std::env::var(CHAINBOT_SECRET_DECRYPTOR_ENV)
        .ok()
        .map(|value| value.to_lowercase())
        .as_deref()
    {
        Some(SECRET_DECRYPTOR_PLAINTEXT) => SecretDecryptMode::Plaintext,
        _ => SecretDecryptMode::Gpg,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::fs;
    use std::sync::{Mutex, OnceLock};

    #[test]
    fn config_reload_requires_restart() {
        let _guard = fixture_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());

        let root = prepare_fixture_root("success", "cli-config-reload-requires-restart");
        let request = CliRequest {
            command: CliCommand::Serve,
            json_output: false,
            trigger_operation: None,
            catalog_request: None,
            observe_request: None,
            daemon_owner_id: None,
        };

        unsafe {
            std::env::set_var("CHAINBOT_CONFIG_DIR", &root);
            std::env::set_var(CHAINBOT_SECRET_DECRYPTOR_ENV, SECRET_DECRYPTOR_PLAINTEXT);
        }

        let mut runtime = request
            .load_runtime_context()
            .expect("initial runtime load should succeed");

        fs::write(
            root.join("triggers")
                .join("external-trigger-e2e")
                .join("config.toml"),
            "manifest_version = \"2.0.0\"\ntrigger_id = [\n",
        )
        .expect("mutated trigger file should be writable");

        let mut lease_supervisor = ServeLeaseSupervisor::new(
            runtime.storage_config.clone(),
            String::from("test-owner"),
            i64::from(std::process::id()),
            1_710_300_000_000,
        );
        let serve_output =
            serve_once_with_lease(&mut runtime, 1_710_300_000_000, &mut lease_supervisor)
                .expect("already loaded runtime should ignore on-disk config mutation");
        assert!(serve_output
            .stdout()
            .contains("serve completed: executed 1 accepted trigger event(s)"));

        let reload_error = request
            .load_runtime_context()
            .expect_err("restarting after config mutation should observe invalid TOML");
        assert!(matches!(reload_error, UserFacingError::Validation { .. }));
        assert!(reload_error
            .to_string()
            .contains("definition file is invalid TOML"));

        unsafe {
            std::env::remove_var("CHAINBOT_CONFIG_DIR");
            std::env::remove_var(CHAINBOT_SECRET_DECRYPTOR_ENV);
        }
    }

    #[test]
    fn serve_drains_all_requests_from_snapshot() {
        let _guard = fixture_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());

        let root = prepare_fixture_root("success", "cli-serve-drains-all-requests");
        fs::create_dir_all(root.join("triggers").join("builtin-trigger-e2e"))
            .expect("builtin trigger package directory should be creatable");
        fs::write(
            root.join("triggers")
                .join("builtin-trigger-e2e")
                .join("config.toml"),
            "manifest_version = \"2.0.0\"\ntrigger_id = \"builtin-trigger-e2e\"\nkind = \"builtin\"\nsource = \"manual\"\nworkflow_id = \"wf-e2e\"\nenabled = true\n",
        )
        .expect("builtin trigger fixture should be writable");

        let request = CliRequest {
            command: CliCommand::Serve,
            json_output: false,
            trigger_operation: None,
            catalog_request: None,
            observe_request: None,
            daemon_owner_id: None,
        };

        unsafe {
            std::env::set_var("CHAINBOT_CONFIG_DIR", &root);
            std::env::set_var(CHAINBOT_SECRET_DECRYPTOR_ENV, SECRET_DECRYPTOR_PLAINTEXT);
        }

        let mut runtime = request
            .load_runtime_context()
            .expect("runtime load should succeed with canonical builtin trigger");
        let mut lease_supervisor = ServeLeaseSupervisor::new(
            runtime.storage_config.clone(),
            String::from("test-owner"),
            i64::from(std::process::id()),
            1_710_300_100_000,
        );
        let serve_output =
            serve_once_with_lease(&mut runtime, 1_710_300_100_000, &mut lease_supervisor)
                .expect("serve should execute every accepted request from the snapshot");

        assert!(serve_output
            .stdout()
            .contains("serve completed: executed 2 accepted trigger event(s)"));
        assert!(serve_output
            .stdout()
            .contains("trigger_id=builtin-trigger-e2e"));
        assert!(serve_output
            .stdout()
            .contains("trigger_id=external-trigger-e2e"));

        let run_summaries = runtime
            .state_store
            .list_run_summaries()
            .expect("serve snapshot runs should be persisted");
        assert_eq!(run_summaries.len(), 2);

        unsafe {
            std::env::remove_var("CHAINBOT_CONFIG_DIR");
            std::env::remove_var(CHAINBOT_SECRET_DECRYPTOR_ENV);
        }
    }

    #[test]
    fn serve_lease_supervisor_renews_same_owner_lease() {
        let root = prepare_fixture_root("success", "cli-serve-lease-renewal");
        let storage_config = RuntimeStorageConfig {
            backend: crate::config::RuntimeStorageBackend::Local {
                database_path: root.join("state").join("runtime.sqlite3"),
            },
            history_retention: None,
            raw_debug_enabled: false,
            raw_debug_artifacts_dir: None,
        };
        let mut state_store = RuntimeStateStore::open(&storage_config, 1_710_300_200_000)
            .expect("runtime state store should open");
        assert!(matches!(
            state_store
                .try_acquire_serve_lease("owner-renew", 1_710_300_200_000, SERVE_LEASE_TTL_MS)
                .expect("initial lease should acquire"),
            LeaseAcquireResult::Acquired
        ));

        let mut supervisor = ServeLeaseSupervisor::new(
            storage_config,
            String::from("owner-renew"),
            i64::from(std::process::id()),
            1_710_300_200_000,
        );
        supervisor
            .maybe_renew(1_710_300_210_100)
            .expect("lease renewal should succeed for same owner");

        let snapshot = state_store
            .inspect_serve_lease(1_710_300_210_100)
            .expect("lease snapshot should load after renewal");
        assert!(matches!(snapshot.state, ServeLeaseState::Active));
        assert_eq!(snapshot.owner_id.as_deref(), Some("owner-renew"));
        assert_eq!(snapshot.expires_at_ms, Some(1_710_300_240_100));
    }

    fn prepare_fixture_root(case_name: &str, root_name: &str) -> PathBuf {
        let root = workspace_root()
            .join("target")
            .join("test-roots")
            .join(root_name);
        if root.exists() {
            fs::remove_dir_all(&root).expect("existing fixture root should be removable");
        }

        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("e2e")
            .join(case_name);
        copy_directory_recursive(&source, &root).expect("fixture root should copy");
        let root_config_path = root.join("chainbot.toml");
        let root_config = fs::read_to_string(&root_config_path)
            .expect("copied fixture root config should be readable");
        let updated_root_config = root_config
            .lines()
            .map(|line| {
                if line.trim_start().starts_with("chainbot_version = ") {
                    format!("chainbot_version = \"{}\"", CHAINBOT_VERSION)
                } else {
                    line.to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(&root_config_path, format!("{updated_root_config}\n"))
            .expect("fixture root config should be rewritten with the running version");

        make_executable(&root.join("plugins").join("bin").join("external_trigger.sh"));
        make_executable(&root.join("plugins").join("bin").join("external_node.sh"));

        root
    }

    fn workspace_root() -> PathBuf {
        let crate_root = Path::new(env!("CARGO_MANIFEST_DIR"));
        crate_root
            .parent()
            .expect("crates directory should exist")
            .parent()
            .expect("workspace root should exist")
            .to_path_buf()
    }

    fn make_executable(path: &Path) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = fs::metadata(path)
                .expect("fixture script metadata should exist")
                .permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(path, permissions).expect("fixture script should be executable");
        }
    }

    fn fixture_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }
}
