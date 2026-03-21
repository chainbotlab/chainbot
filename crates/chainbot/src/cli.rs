//! [INPUT]
//! Process arguments, environment-resolved ChainBot roots, and runtime services from config, state, trigger, executor, worker, and secrets modules.
//!
//! [OUTPUT]
//! Parses commands, executes help, init, status, trigger, validate, run, serve, and list-runs flows, and maps failures to stable CLI output and exit codes.
//!
//! [ROLE]
//! Owns the user-facing command boundary for the `chainbot` binary.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::builtins::nodes::script_worker::{WorkerHost, WorkerHostLimits};
use crate::builtins::{
    build_builtin_registry, build_builtin_trigger_emissions, BuiltinRuntimeContext,
    SecretDecryptMode,
};
use crate::config::{
    load_effective_root_layout, load_trigger_definitions, resolve_root_layout, set_trigger_enabled,
    RootConfigDefinition, RootDefinitionBundle, RootLayout, RootPathOverrides, TriggerToggleResult,
};
use crate::errors::UserFacingError;
use crate::executor::{ExecutionPlane, NormalizedRunRequest, WorkflowRunStatus};
use crate::plugin::{PluginKind, PluginManifest};
use crate::state::{
    sanitize_path_component, CoordinationError, CoordinationStore, FileBackedStateStore,
    FileStateError, LeaseAcquireResult, RunRecordSummary, RunStatus, ServeLeaseSnapshot,
    ServeLeaseState, StateLayout, TriggerEventRecord, SERVE_OWNER_ID_PREFIX,
};
use crate::trigger::{
    TriggerDefinition, TriggerPlane, TriggerPlaneError, TriggerPluginHostPolicy, TriggerRunRequest,
    REQUIRED_TRIGGER_PLUGIN_CAPABILITY,
};
use crate::workflow::WorkflowDefinition;

const SERVE_LEASE_TTL_MS: i64 = 30_000;
const SERVE_LEASE_RENEW_INTERVAL_MS: i64 = 10_000;
const CHAINBOT_SECRET_DECRYPTOR_ENV: &str = "CHAINBOT_SECRET_DECRYPTOR";
const SECRET_DECRYPTOR_PLAINTEXT: &str = "plaintext";
const INIT_MANIFEST_VERSION: &str = "2.0.0";
const CHAINBOT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliCommand {
    Help(HelpTopic),
    Version,
    Init,
    Status,
    Trigger,
    Validate,
    Run,
    Serve,
    ListRuns,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelpTopic {
    General,
    Version,
    Init,
    Status,
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
    state_store: FileBackedStateStore,
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
    state_layout: StateLayout,
    owner_id: String,
    lease_ttl_ms: i64,
    renew_interval_ms: i64,
    next_renew_at_ms: i64,
}

impl ServeLeaseSupervisor {
    fn new(state_layout: StateLayout, owner_id: String, acquired_at_ms: i64) -> Self {
        Self {
            state_layout,
            owner_id,
            lease_ttl_ms: SERVE_LEASE_TTL_MS,
            renew_interval_ms: SERVE_LEASE_RENEW_INTERVAL_MS,
            next_renew_at_ms: acquired_at_ms.saturating_add(SERVE_LEASE_RENEW_INTERVAL_MS),
        }
    }

    fn maybe_renew(&mut self, now_ms: i64) -> Result<(), UserFacingError> {
        if now_ms < self.next_renew_at_ms {
            return Ok(());
        }

        let mut coordination = CoordinationStore::open(&self.state_layout, now_ms)
            .map_err(|error| map_coordination_error("open serve coordination store", error))?;
        match coordination
            .try_acquire_serve_lease(&self.owner_id, now_ms, self.lease_ttl_ms)
            .map_err(|error| map_coordination_error("renew serve lease", error))?
        {
            LeaseAcquireResult::Acquired | LeaseAcquireResult::Renewed => {
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
    expires_at_ms: Option<i64>,
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
            }),
            "-V" | "--version" => Ok(Self {
                command: CliCommand::Version,
                json_output: false,
                trigger_operation: None,
            }),
            "help" => Self::parse_help_args(args),
            "version" => Self::parse_command_args(CliCommand::Version, args),
            "init" => Self::parse_command_args(CliCommand::Init, args),
            "status" => Self::parse_command_args(CliCommand::Status, args),
            "trigger" => Self::parse_trigger_args(args),
            "validate" => Self::parse_command_args(CliCommand::Validate, args),
            "run" => Self::parse_command_args(CliCommand::Run, args),
            "serve" => Self::parse_command_args(CliCommand::Serve, args),
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
            CliCommand::Trigger => self.execute_trigger(),
            CliCommand::Validate => self.execute_validate(),
            CliCommand::Run => self.execute_run(),
            CliCommand::Serve => self.execute_serve(),
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
                "Unexpected argument for help: {}",
                extra.to_string_lossy()
            )));
        }

        Ok(Self {
            command: CliCommand::Help(topic),
            json_output: false,
            trigger_operation: None,
        })
    }

    fn parse_command_args<I>(command: CliCommand, mut args: I) -> Result<Self, UserFacingError>
    where
        I: Iterator<Item = OsString>,
    {
        let mut json_output = false;
        while let Some(arg) = args.next() {
            let raw = arg.to_string_lossy().into_owned();
            match raw.as_str() {
                "-h" | "--help" => {
                    return Ok(Self {
                        command: CliCommand::Help(help_topic_for(command)),
                        json_output: false,
                        trigger_operation: None,
                    });
                }
                "--json" if matches!(command, CliCommand::Status) => {
                    json_output = true;
                }
                _ => {
                    if let Some((flag, value)) = raw.split_once('=') {
                        if flag == "--json" && matches!(command, CliCommand::Status) {
                            json_output = parse_bool_flag_value(value)?;
                            continue;
                        }
                    }

                    return Err(UserFacingError::usage(format!(
                        "Unexpected argument for {}: {raw}",
                        command_name(command)
                    )));
                }
            }
        }

        Ok(Self {
            command,
            json_output,
            trigger_operation: None,
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
            });
        }

        if action == "list" {
            let mut json_output = false;
            while let Some(arg) = args.next() {
                let raw = arg.to_string_lossy().into_owned();
                match raw.as_str() {
                    "--json" => {
                        json_output = true;
                    }
                    _ => {
                        if let Some((flag, value)) = raw.split_once('=') {
                            if flag == "--json" {
                                json_output = parse_bool_flag_value(value)?;
                                continue;
                            }
                        }

                        return Err(UserFacingError::usage(format!(
                            "Unexpected argument for trigger list: {raw}"
                        )));
                    }
                }
            }

            return Ok(Self {
                command: CliCommand::Trigger,
                json_output,
                trigger_operation: Some(TriggerOperation::List),
            });
        }

        let enabled = match action.as_str() {
            "enable" => true,
            "disable" => false,
            other => {
                return Err(UserFacingError::usage(format!(
                    "Unsupported trigger action `{other}`. Use `list`, `enable`, or `disable`."
                )));
            }
        };

        let mut trigger_id = None;
        while let Some(arg) = args.next() {
            let raw = arg.to_string_lossy().into_owned();
            match raw.as_str() {
                "-h" | "--help" => {
                    return Ok(Self {
                        command: CliCommand::Help(HelpTopic::Trigger),
                        json_output: false,
                        trigger_operation: None,
                    });
                }
                _ => {
                    if trigger_id.is_none() {
                        trigger_id = Some(raw);
                        continue;
                    }

                    return Err(UserFacingError::usage(format!(
                        "Unexpected argument for trigger {}: {raw}",
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
        let state_layout = StateLayout::from_root_layout(&root_layout);
        let state_store = FileBackedStateStore::new(state_layout.clone());
        let observed_at_ms = current_time_ms()?;
        let run_summaries = state_store
            .list_committed_run_summaries()
            .map_err(|error| map_file_state_error("list committed run summaries", error))?;
        let trigger_records = state_store
            .load_committed_trigger_records()
            .map_err(|error| map_file_state_error("load committed trigger records", error))?;
        let serve_lease =
            CoordinationStore::inspect_existing_serve_lease(&state_layout, observed_at_ms)
                .map_err(|error| map_coordination_error("inspect existing serve lease", error))?;

        let payload = build_status_output(
            &root_layout,
            definitions.root_config.profile,
            &definitions.workflows,
            &definitions.triggers,
            &run_summaries,
            &trigger_records,
            serve_lease,
        );

        if self.json_output {
            let stdout = serde_json::to_string_pretty(&payload).map_err(|source| {
                UserFacingError::state(format!("Failed to serialize status payload: {source}"))
            })?;
            return Ok(CliOutput::text(stdout));
        }

        Ok(CliOutput::text(render_status_output(&payload)))
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
        let store = FileBackedStateStore::new(StateLayout::from_root_layout(&layout));
        store
            .initialize()
            .map_err(|error| map_file_state_error("initialize runtime state", error))?;
        let recovered_at_ms = current_time_ms()?;
        let _ = store
            .recover_runtime_state(recovered_at_ms)
            .map_err(|error| map_file_state_error("recover runtime state", error))?;
        let summaries = store
            .list_run_summaries()
            .map_err(|error| map_file_state_error("list persisted run summaries", error))?;
        let payload = serde_json::to_string_pretty(&summaries).map_err(|source| {
            UserFacingError::state(format!(
                "Failed to serialize run summaries for output: {source}"
            ))
        })?;
        Ok(CliOutput::text(payload))
    }

    fn execute_run(&self) -> Result<CliOutput, UserFacingError> {
        let runtime = self.load_runtime_context()?;
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

        let run_result = execute_single_run(&runtime, request, current_time_ms()?)?;
        Ok(CliOutput::text(format!(
            "run completed: run_id={} workflow_id={} status={}",
            run_result.run_id,
            run_result.workflow_id,
            render_run_status(run_result.status)
        )))
    }

    fn execute_serve(&self) -> Result<CliOutput, UserFacingError> {
        let runtime = self.load_runtime_context()?;
        let state_layout = StateLayout::from_root_layout(&runtime.root_layout);
        let now_ms = current_time_ms()?;
        let owner_id = format!("{SERVE_OWNER_ID_PREFIX}{}", std::process::id());
        let mut coordination = CoordinationStore::open(&state_layout, now_ms)
            .map_err(|error| map_coordination_error("open serve coordination store", error))?;

        match coordination
            .try_acquire_serve_lease(&owner_id, now_ms, SERVE_LEASE_TTL_MS)
            .map_err(|error| map_coordination_error("acquire serve lease", error))?
        {
            LeaseAcquireResult::Acquired | LeaseAcquireResult::Renewed => {
                let mut lease_supervisor = ServeLeaseSupervisor::new(
                    state_layout.clone(),
                    owner_id.clone(),
                    now_ms,
                );
                let run_result = serve_once_with_lease(&runtime, now_ms, &mut lease_supervisor);
                let released = coordination
                    .release_serve_lease(&owner_id)
                    .map_err(|error| map_coordination_error("release serve lease", error))?;
                if !released {
                    return Err(UserFacingError::state(
                        "Failed to release serve lease for current owner.",
                    ));
                }
                run_result
            }
            LeaseAcquireResult::Rejected {
                current_owner,
                expires_at_ms,
            } => Err(UserFacingError::conflict(format!(
                "Serve is already active for this root (owner: {current_owner}, expires_at_ms: {expires_at_ms})."
            ))),
        }
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

        let state_store = FileBackedStateStore::new(StateLayout::from_root_layout(&root_layout));
        let recovered_at_ms = current_time_ms()?;
        state_store
            .initialize()
            .map_err(|error| map_file_state_error("initialize runtime state", error))?;
        let _ = state_store
            .recover_runtime_state(recovered_at_ms)
            .map_err(|error| map_file_state_error("recover runtime state", error))?;

        Ok(RuntimeContext {
            root_layout,
            definitions,
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
        "trigger" => Ok(HelpTopic::Trigger),
        "validate" => Ok(HelpTopic::Validate),
        "run" => Ok(HelpTopic::Run),
        "serve" => Ok(HelpTopic::Serve),
        "list-runs" => Ok(HelpTopic::ListRuns),
        other => Err(UserFacingError::usage(format!(
            "{}",
            unsupported_command_message(other)
        ))),
    }
}

fn help_topic_for(command: CliCommand) -> HelpTopic {
    match command {
        CliCommand::Help(topic) => topic,
        CliCommand::Version => HelpTopic::Version,
        CliCommand::Init => HelpTopic::Init,
        CliCommand::Status => HelpTopic::Status,
        CliCommand::Trigger => HelpTopic::Trigger,
        CliCommand::Validate => HelpTopic::Validate,
        CliCommand::Run => HelpTopic::Run,
        CliCommand::Serve => HelpTopic::Serve,
        CliCommand::ListRuns => HelpTopic::ListRuns,
    }
}

fn command_name(command: CliCommand) -> &'static str {
    match command {
        CliCommand::Help(_) => "help",
        CliCommand::Version => "version",
        CliCommand::Init => "init",
        CliCommand::Status => "status",
        CliCommand::Trigger => "trigger",
        CliCommand::Validate => "validate",
        CliCommand::Run => "run",
        CliCommand::Serve => "serve",
        CliCommand::ListRuns => "list-runs",
    }
}

fn general_help_text() -> String {
    format!(
        "ChainBot command skills\n\nVersion:\n  {CHAINBOT_VERSION}\n\nUsage:\n  chainbot help [command]\n  chainbot version\n  chainbot init\n  chainbot status [--json]\n  chainbot trigger list [--json]\n  chainbot trigger <enable|disable> <trigger-id>\n  chainbot validate\n  chainbot list-runs\n  chainbot run\n  chainbot serve\n\nRoot resolution:\n  - use CHAINBOT_CONFIG_DIR when it is set to a non-empty path\n  - otherwise fall back to ~/.chainbot\n\nCommands:\n  version    Print the running ChainBot version.\n  init       Bootstrap a minimal ChainBot root.\n  status     Inspect runtime state without executing workflows.\n  trigger    Inspect or persist trigger package state.\n  validate   Validate config and package contracts.\n  list-runs  Print persisted run summaries as JSON.\n  run        Execute one single-shot manual run.\n  serve      Drain one trigger snapshot under a serve lease.\n\nUse `chainbot help <command>` for command-specific guidance."
    )
}

fn help_text(topic: HelpTopic) -> String {
    match topic {
        HelpTopic::General => general_help_text(),
        HelpTopic::Version => format!(
            "version - Print the running ChainBot version\n\nUse when:\n  - you need to confirm the installed CLI release\n  - you want to compare the binary version against root config metadata\n\nPrints:\n  - chainbot {CHAINBOT_VERSION}\n\nExamples:\n  chainbot version\n  chainbot --version\n\nSee also:\n  init, validate"
        ),
        HelpTopic::Init => String::from("init - Bootstrap a minimal ChainBot root\n\nUse when:\n  - you need a new ChainBot root that validates immediately\n  - you want the default single-file root config without manual setup\n  - you are preparing a fresh local root for workflows and triggers\n\nWrites:\n  - resolved root directory\n  - chainbot.toml when no root config exists yet\n  - default package directories under the root\n\nDoes not execute:\n  - workflow runs\n  - trigger snapshots\n\nExamples:\n  chainbot init\n  CHAINBOT_CONFIG_DIR=/tmp/demo-root chainbot init\n\nCompatibility:\n  - reuses existing chainbot.toml when present\n  - falls back to config/root.toml for pre-migration roots\n\nSee also:\n  version, validate, status, trigger"),
        HelpTopic::Status => String::from("status - Inspect runtime state without executing workflows\n\nUse when:\n  - you want to know whether serve is active\n  - you want the latest workflow run result\n  - you want trigger activity without opening state files\n\nReads:\n  - configured root config\n  - configured workflow packages\n  - configured trigger packages\n  - configured state runs directory\n  - configured trigger record directory\n  - configured coordination store\n\nDoes not execute:\n  - workflow runs\n  - trigger snapshots\n\nExamples:\n  chainbot status\n  CHAINBOT_CONFIG_DIR=/tmp/demo-root chainbot status\n  chainbot status --json\n\nSee also:\n  validate, list-runs, serve"),
        HelpTopic::Trigger => String::from("trigger - Inspect or persist trigger package state\n\nUse when:\n  - you need to inspect configured triggers without opening TOML manually\n  - you need to stop a trigger without editing TOML manually\n  - you want to re-enable a trigger after maintenance or debugging\n\nReads:\n  - configured root config\n  - configured trigger packages\n\nWrites:\n  - target trigger package config.toml for enable or disable actions\n\nDoes not execute:\n  - workflow runs\n  - trigger snapshots\n\nExamples:\n  chainbot trigger list\n  chainbot trigger list --json\n  chainbot trigger enable tr-market\n  CHAINBOT_CONFIG_DIR=/tmp/demo-root chainbot trigger disable tr-market\n\nSee also:\n  init, status, validate, serve"),
        HelpTopic::Validate => String::from("validate - Validate config and package contracts\n\nUse when:\n  - you want to confirm a root is structurally valid\n  - you changed config and want a fast contract check\n\nReads:\n  - configured root config\n  - configured workflow packages\n  - configured trigger packages\n  - configured plugin packages\n\nDoes not execute:\n  - workflow runs\n  - trigger snapshots\n\nExamples:\n  chainbot validate\n  CHAINBOT_CONFIG_DIR=/tmp/demo-root chainbot validate\n\nSee also:\n  status, run, serve"),
        HelpTopic::ListRuns => String::from("list-runs - Print persisted run summaries as JSON\n\nUse when:\n  - you need machine-readable workflow run summaries\n  - you want raw persisted run status output without higher-level aggregation\n\nReads:\n  - configured state runs directory\n\nDoes not execute:\n  - workflow runs\n  - trigger snapshots\n\nExamples:\n  chainbot list-runs\n  CHAINBOT_CONFIG_DIR=/tmp/demo-root chainbot list-runs\n\nSee also:\n  status, serve"),
        HelpTopic::Run => String::from("run - Execute one manual workflow run\n\nUse when:\n  - you want a single manual execution without a serve lease\n  - your root contains exactly one workflow package\n\nReads:\n  - configured root config\n  - configured workflow packages\n  - configured plugin packages\n  - configured secrets directory\n\nWrites:\n  - configured state runs directory\n  - configured workflow log directory\n\nExamples:\n  chainbot run\n  CHAINBOT_CONFIG_DIR=/tmp/demo-root chainbot run\n\nSee also:\n  status, validate, serve"),
        HelpTopic::Serve => String::from("serve - Drain one trigger snapshot under a serve lease\n\nUse when:\n  - you want to evaluate configured triggers once\n  - you need runtime recovery plus duplicate-suppression coordination\n\nReads:\n  - configured root config\n  - configured workflow packages\n  - configured trigger packages\n  - configured plugin packages\n  - configured secrets directory\n\nWrites:\n  - configured state runs directory\n  - configured workflow log directory\n  - configured trigger record directory\n  - configured coordination store\n\nExamples:\n  chainbot serve\n  CHAINBOT_CONFIG_DIR=/tmp/demo-root chainbot serve\n\nSee also:\n  status, validate, list-runs"),
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
    run_summaries: &[RunRecordSummary],
    trigger_records: &[TriggerEventRecord],
    serve_lease: ServeLeaseSnapshot,
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

    let mut latest_trigger_events = BTreeMap::<String, TriggerEventRecord>::new();
    for record in trigger_records {
        match latest_trigger_events.get(&record.trigger_id) {
            Some(current) if !trigger_record_is_newer(record, current) => {}
            _ => {
                latest_trigger_events.insert(record.trigger_id.clone(), record.clone());
            }
        }
    }

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
                last_event_id: latest_record.map(|record| record.event_id.clone()),
                last_accepted_at_ms: latest_record.map(|record| record.accepted_at_ms),
            }
        })
        .collect::<Vec<_>>();

    StatusOutput {
        root: StatusRootView {
            path: root_layout.root.display().to_string(),
            profile,
        },
        serve: StatusServeView {
            state: serve_lease.state,
            owner: serve_lease.owner_id,
            expires_at_ms: serve_lease.expires_at_ms,
        },
        workflows: workflow_views,
        triggers: trigger_views,
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
    if let Some(expires_at_ms) = status.serve.expires_at_ms {
        lines.push(format!("  serve_expires_at_ms: {expires_at_ms}"));
    }

    lines.push(String::new());
    lines.push(String::from("Legacy Layout"));
    lines.push(String::new());
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

fn trigger_record_is_newer(candidate: &TriggerEventRecord, current: &TriggerEventRecord) -> bool {
    (
        candidate.accepted_at_ms,
        candidate.sequence,
        &candidate.event_id,
    ) > (current.accepted_at_ms, current.sequence, &current.event_id)
}

fn render_serve_lease_state(state: ServeLeaseState) -> &'static str {
    match state {
        ServeLeaseState::Idle => "idle",
        ServeLeaseState::Active => "active",
        ServeLeaseState::Stale => "stale",
    }
}

fn parse_bool_flag_value(value: &str) -> Result<bool, UserFacingError> {
    match value {
        "true" | "1" => Ok(true),
        "false" | "0" => Ok(false),
        other => Err(UserFacingError::usage(format!(
            "Unsupported boolean flag value `{other}`. Use true, false, 1, or 0."
        ))),
    }
}

fn unsupported_command_message(value: &str) -> String {
    match suggest_command(value) {
        Some(suggestion) => format!(
            "Unsupported command `{value}`. Did you mean `{suggestion}`? Run `chainbot help` to see available command skills."
        ),
        None => format!(
            "Unsupported command `{value}`. Run `chainbot help` to see available command skills."
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
            let root_config = toml::from_str::<RootConfigDefinition>(&contents).map_err(|source| {
                UserFacingError::validation(format!(
                    "Existing init root config is invalid TOML at {}: {source}",
                    path.display()
                ))
            })?;
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

fn map_file_state_error(action: &str, error: FileStateError) -> UserFacingError {
    UserFacingError::state(format!("Failed to {action}: {error}"))
}

fn map_coordination_error(action: &str, error: CoordinationError) -> UserFacingError {
    UserFacingError::state(format!("Failed to {action}: {error}"))
}

fn map_trigger_error(error: TriggerPlaneError) -> UserFacingError {
    match error {
        TriggerPlaneError::Contract(source) => UserFacingError::from_contract(source),
        TriggerPlaneError::FileState(source) => {
            map_file_state_error("persist trigger-plane state", source)
        }
        TriggerPlaneError::Coordination(source) => {
            map_coordination_error("evaluate trigger-plane coordination", source)
        }
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

fn serve_once_with_lease(
    runtime: &RuntimeContext,
    accepted_at_ms: i64,
    lease_supervisor: &mut ServeLeaseSupervisor,
) -> Result<CliOutput, UserFacingError> {
    lease_supervisor.maybe_renew(accepted_at_ms)?;
    let state_layout = StateLayout::from_root_layout(&runtime.root_layout);
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

    let mut trigger_plane = TriggerPlane::open(
        state_layout,
        runtime.definitions.triggers.clone(),
        trigger_manifests,
        policy,
        builtin_events,
        accepted_at_ms,
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
    if run_requests.is_empty() {
        return Ok(CliOutput::text(
            "serve completed: no accepted trigger events",
        ));
    }

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
    runtime: &RuntimeContext,
    request: NormalizedRunRequest,
    started_at_ms: i64,
) -> Result<SingleRunResult, UserFacingError> {
    let run_id = request.run_id.clone();
    let workflow_id = request.workflow_id.clone();

    write_run_status(
        &runtime.state_store,
        &run_id,
        &workflow_id,
        RunStatus::Running,
        started_at_ms,
        None,
    )
    .map_err(|error| map_file_state_error("persist running run summary", error))?;

    write_log_entry(
        &runtime.state_store,
        &run_id,
        "run_started",
        "run accepted for execution",
        started_at_ms,
    )
    .map_err(|error| map_file_state_error("write run_started log", error))?;

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
                &runtime.state_store,
                &run_id,
                "run_finished",
                &format!(
                    "execution finished with {} and {} node failure(s)",
                    render_run_status(status),
                    report.node_failures.len()
                ),
                finished_at_ms,
            )
            .map_err(|error| map_file_state_error("write run_finished log", error))?;

            write_run_status(
                &runtime.state_store,
                &run_id,
                &workflow_id,
                status,
                started_at_ms,
                Some(finished_at_ms),
            )
            .map_err(|error| map_file_state_error("persist finished run summary", error))?;

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
                &runtime.state_store,
                &run_id,
                "run_failed",
                &detail,
                finished_at_ms,
            )
            .map_err(|write_error| map_file_state_error("write run_failed log", write_error))?;

            write_run_status(
                &runtime.state_store,
                &run_id,
                &workflow_id,
                RunStatus::Failed,
                started_at_ms,
                Some(finished_at_ms),
            )
            .map_err(|write_error| {
                map_file_state_error("persist failed run summary", write_error)
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
        manifests: runtime
            .definitions
            .plugins
            .iter()
            .cloned()
            .map(|manifest| (manifest.plugin_id.clone(), manifest))
            .collect(),
        secret_mode: runtime.secret_mode,
        worker_host: runtime.worker_host.clone(),
    });

    ExecutionPlane::new(
        runtime.definitions.workflows.clone(),
        runtime.definitions.root_config.runtime_defaults.clone(),
        registry,
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

fn write_run_status(
    state_store: &FileBackedStateStore,
    run_id: &str,
    workflow_id: &str,
    status: RunStatus,
    started_at_ms: i64,
    finished_at_ms: Option<i64>,
) -> Result<PathBuf, FileStateError> {
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
    state_store: &FileBackedStateStore,
    run_id: &str,
    event: &str,
    message: &str,
    occurred_at_ms: i64,
) -> Result<PathBuf, FileStateError> {
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
        };

        unsafe {
            std::env::set_var("CHAINBOT_CONFIG_DIR", &root);
            std::env::set_var(CHAINBOT_SECRET_DECRYPTOR_ENV, SECRET_DECRYPTOR_PLAINTEXT);
        }

        let runtime = request
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
            StateLayout::from_root_layout(&runtime.root_layout),
            String::from("test-owner"),
            1_710_300_000_000,
        );
        let serve_output =
            serve_once_with_lease(&runtime, 1_710_300_000_000, &mut lease_supervisor)
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
            .contains("Definition file is invalid TOML"));

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
        };

        unsafe {
            std::env::set_var("CHAINBOT_CONFIG_DIR", &root);
            std::env::set_var(CHAINBOT_SECRET_DECRYPTOR_ENV, SECRET_DECRYPTOR_PLAINTEXT);
        }

        let runtime = request
            .load_runtime_context()
            .expect("runtime load should succeed with canonical builtin trigger");
        let mut lease_supervisor = ServeLeaseSupervisor::new(
            StateLayout::from_root_layout(&runtime.root_layout),
            String::from("test-owner"),
            1_710_300_100_000,
        );
        let serve_output =
            serve_once_with_lease(&runtime, 1_710_300_100_000, &mut lease_supervisor)
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
        let state_layout = StateLayout::from_root_layout(&RootLayout::from_root(root));
        let mut coordination = CoordinationStore::open(&state_layout, 1_710_300_200_000)
            .expect("coordination store should open");
        assert!(matches!(
            coordination
                .try_acquire_serve_lease("owner-renew", 1_710_300_200_000, SERVE_LEASE_TTL_MS)
                .expect("initial lease should acquire"),
            LeaseAcquireResult::Acquired
        ));

        let mut supervisor = ServeLeaseSupervisor::new(
            state_layout.clone(),
            String::from("owner-renew"),
            1_710_300_200_000,
        );
        supervisor
            .maybe_renew(1_710_300_210_100)
            .expect("lease renewal should succeed for same owner");

        let snapshot =
            CoordinationStore::inspect_existing_serve_lease(&state_layout, 1_710_300_210_100)
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
