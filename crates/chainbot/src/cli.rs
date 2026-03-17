/*
[INPUT]:  Process arguments, optional root overrides, definition roots, and runtime state boundaries.
[OUTPUT]: Parsed CLI command surface, runtime execution side effects, and stable user-facing failures.
[POS]:    CLI boundary for command routing, root resolution, and user-visible execution errors.
[UPDATE]: 2026-03-16 - Add validate command with --root support.
[UPDATE]: 2026-03-16 - Expand command surface with help, list-runs, run, and serve.
[UPDATE]: 2026-03-16 - Wire bounded vertical-slice execution for run and serve with trigger, plugin, script, and secret runtime paths.
[UPDATE]: 2026-03-16 - Make restart recovery explicit and keep serve reload policy restart-only.
[UPDATE]: 2026-03-16 - Drain each accepted serve snapshot deterministically and honor builtin trigger aliases.
[UPDATE]: 2026-03-17 - Redact runtime plugin/script failure details after secret resolution before log persistence and CLI propagation.
*/

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::{resolve_root_layout, RootDefinitionBundle, RootLayout};
use crate::errors::{ContractError, UserFacingError};
use crate::executor::{
    BuiltinNodeRegistry, BuiltinNodeRequest, BuiltinNodeResult, ExecutionPlane,
    NormalizedRunRequest, WorkflowRunStatus,
};
use crate::plugin::{
    ExternalNodePluginHost, ExternalNodePluginRequest, PluginKind, PluginManifest,
    NODE_PLUGIN_CONTRACT_VERSION, NODE_PLUGIN_EXECUTE_CAPABILITY,
};
use crate::secrets::{
    redact_text, GpgSecretDecryptor, PlaintextSecretDecryptor, SecretProvider, SecretReference,
    SecretValue,
};
use crate::state::{
    sanitize_path_component, CoordinationError, CoordinationStore, FileBackedStateStore,
    FileStateError, LeaseAcquireResult, RunRecordSummary, RunStatus, StateLayout,
    SERVE_OWNER_ID_PREFIX,
};
use crate::trigger::{
    TriggerEmission, TriggerKind, TriggerPlane, TriggerPlaneError, TriggerPluginHostPolicy,
    TriggerRunRequest, REQUIRED_TRIGGER_PLUGIN_CAPABILITY,
};
use crate::worker::{
    ScriptRuntime, WorkerHost, WorkerHostLimits, WorkerProcessSpec, WorkerRequestEnvelope,
};

const SERVE_LEASE_TTL_MS: i64 = 30_000;
const CHAINBOT_SECRET_DECRYPTOR_ENV: &str = "CHAINBOT_SECRET_DECRYPTOR";
const SECRET_DECRYPTOR_PLAINTEXT: &str = "plaintext";
const SCRIPT_RUNTIME_PYTHON: &str = "python";
const SCRIPT_RUNTIME_JAVASCRIPT: &str = "javascript";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliCommand {
    Help(HelpTopic),
    Validate,
    Run,
    Serve,
    ListRuns,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelpTopic {
    General,
    Validate,
    Run,
    Serve,
    ListRuns,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliRequest {
    pub command: CliCommand,
    pub root_override: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliOutput {
    stdout: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SecretDecryptMode {
    Gpg,
    Plaintext,
}

#[derive(Debug)]
struct RuntimeContext {
    root_layout: RootLayout,
    definitions: RootDefinitionBundle,
    state_store: FileBackedStateStore,
    secret_mode: SecretDecryptMode,
    worker_host: WorkerHost,
}

#[derive(Debug, Clone)]
struct RuntimePluginContext {
    root_layout: RootLayout,
    manifests: BTreeMap<String, PluginManifest>,
    secret_mode: SecretDecryptMode,
    worker_host: WorkerHost,
}

#[derive(Debug, Clone)]
struct ScriptNodeSpec {
    runtime: ScriptRuntime,
    script_relative_path: String,
}

#[derive(Debug, Clone)]
struct ResolvedNodeInputs {
    values: BTreeMap<String, serde_json::Value>,
    resolved_secrets: Vec<SecretValue>,
}

#[derive(Debug)]
struct SingleRunResult {
    run_id: String,
    workflow_id: String,
    status: RunStatus,
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
                root_override: None,
            }),
            "help" => Self::parse_help_args(args),
            "validate" => Self::parse_command_args(CliCommand::Validate, args),
            "run" => Self::parse_command_args(CliCommand::Run, args),
            "serve" => Self::parse_command_args(CliCommand::Serve, args),
            "list-runs" => Self::parse_command_args(CliCommand::ListRuns, args),
            other => Err(UserFacingError::usage(format!(
                "Unsupported command `{other}`. Run `chainbot --help` to see available commands."
            ))),
        }
    }

    pub fn execute(&self) -> Result<CliOutput, UserFacingError> {
        match self.command {
            CliCommand::Help(topic) => Ok(CliOutput::text(help_text(topic))),
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
            root_override: None,
        })
    }

    fn parse_command_args<I>(command: CliCommand, mut args: I) -> Result<Self, UserFacingError>
    where
        I: Iterator<Item = OsString>,
    {
        let mut root_override = None;
        while let Some(arg) = args.next() {
            let raw = arg.to_string_lossy().into_owned();
            match raw.as_str() {
                "-h" | "--help" => {
                    return Ok(Self {
                        command: CliCommand::Help(help_topic_for(command)),
                        root_override: None,
                    });
                }
                "--root" => {
                    let Some(value) = args.next() else {
                        return Err(UserFacingError::usage("`--root` requires a path value."));
                    };
                    root_override = Some(PathBuf::from(value));
                }
                _ => {
                    if let Some((flag, value)) = raw.split_once('=') {
                        if flag == "--root" {
                            root_override = Some(PathBuf::from(value));
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
            root_override,
        })
    }

    fn execute_validate(&self) -> Result<CliOutput, UserFacingError> {
        let layout = self.load_definition_root()?;
        Ok(CliOutput::text(format!(
            "validated root: {}",
            layout.root.display()
        )))
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
        let workflow = runtime.definitions.workflows.first().ok_or_else(|| {
            UserFacingError::validation("No workflows were found under <root>/workflows.")
        })?;

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
                let run_result = serve_once_with_lease(&runtime, now_ms);
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
        let layout = resolve_root_layout(self.root_override.as_deref())
            .map_err(UserFacingError::from_contract)?;
        maybe_prepare_e2e_root(&layout)?;
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
        "validate" => Ok(HelpTopic::Validate),
        "run" => Ok(HelpTopic::Run),
        "serve" => Ok(HelpTopic::Serve),
        "list-runs" => Ok(HelpTopic::ListRuns),
        other => Err(UserFacingError::usage(format!(
            "Unknown help topic `{other}`. Run `chainbot --help` to see available commands."
        ))),
    }
}

fn help_topic_for(command: CliCommand) -> HelpTopic {
    match command {
        CliCommand::Help(topic) => topic,
        CliCommand::Validate => HelpTopic::Validate,
        CliCommand::Run => HelpTopic::Run,
        CliCommand::Serve => HelpTopic::Serve,
        CliCommand::ListRuns => HelpTopic::ListRuns,
    }
}

fn command_name(command: CliCommand) -> &'static str {
    match command {
        CliCommand::Help(_) => "help",
        CliCommand::Validate => "validate",
        CliCommand::Run => "run",
        CliCommand::Serve => "serve",
        CliCommand::ListRuns => "list-runs",
    }
}

fn general_help_text() -> &'static str {
    "ChainBot command line interface\n\nUsage:\n  chainbot validate [--root <path>]\n  chainbot list-runs [--root <path>]\n  chainbot run [--root <path>]\n  chainbot serve [--root <path>]\n  chainbot help [command]\n\nCommands:\n  validate   Validate root definitions without starting execution.\n  list-runs  Print persisted run summaries as JSON.\n  run        Execute one single-shot manual run.\n  serve      Acquire a lease, recover prior runtime state, and process one trigger snapshot.\n\nOptions:\n  --root <path>  Override the default root (~/.chainbot).\n  -h, --help     Show help."
}

fn help_text(topic: HelpTopic) -> &'static str {
    match topic {
        HelpTopic::General => general_help_text(),
        HelpTopic::Validate => "Usage:\n  chainbot validate [--root <path>]\n\nValidate the root definition set without starting execution.\nThe command checks the root layout plus root/workflow/trigger/plugin TOML files.",
        HelpTopic::ListRuns => "Usage:\n  chainbot list-runs [--root <path>]\n\nPrint persisted run summaries as JSON.\nThe command reads file-backed summaries from <root>/state/runs and prints [] when no runs are recorded.",
        HelpTopic::Run => "Usage:\n  chainbot run [--root <path>]\n\nExecute one single-shot manual run without a long-lived serve lease.\nRuntime artifacts are persisted under <root>/state.",
        HelpTopic::Serve => "Usage:\n  chainbot serve [--root <path>]\n\nAcquire a serve lease, recover prior runtime state once, collect one trigger snapshot, execute every accepted run in deterministic order, and release the lease. Config and trigger definitions are reloaded only when the command restarts.\nRuntime artifacts are persisted under <root>/state.",
    }
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
) -> Result<CliOutput, UserFacingError> {
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
    let builtin_events = build_builtin_trigger_emissions(&runtime.definitions, accepted_at_ms);

    let mut trigger_plane = TriggerPlane::open(
        state_layout,
        runtime.definitions.triggers.clone(),
        trigger_manifests,
        policy,
        builtin_events,
        accepted_at_ms,
    )
    .map_err(map_trigger_error)?;

    let run_requests = trigger_plane
        .collect_run_requests(accepted_at_ms)
        .map_err(map_trigger_error)?;
    if run_requests.is_empty() {
        return Ok(CliOutput::text(
            "serve completed: no accepted trigger events",
        ));
    }

    let mut completed_runs = Vec::with_capacity(run_requests.len());
    let mut failures = Vec::new();

    for trigger_request in &run_requests {
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
    let plugin_context = Arc::new(RuntimePluginContext {
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

    let mut registry = BuiltinNodeRegistry::with_defaults();

    {
        let context = Arc::clone(&plugin_context);
        registry.register("builtin.external_node", move |request| {
            execute_external_node_plugin(&context, request)
        });
    }

    {
        let context = Arc::clone(&plugin_context);
        registry.register("builtin.script", move |request| {
            execute_script_node(&context, request)
        });
    }

    ExecutionPlane::new(runtime.definitions.workflows.clone(), registry)
        .map_err(UserFacingError::from_contract)
}

fn execute_external_node_plugin(
    context: &RuntimePluginContext,
    request: &BuiltinNodeRequest,
) -> Result<BuiltinNodeResult, ContractError> {
    let plugin_id = request.operation.trim();
    if plugin_id.is_empty() {
        return Err(ContractError::CliUsage {
            message: format!(
                "workflow {} node {} missing plugin id in operation field",
                request.workflow_id, request.node_id
            ),
        });
    }

    let manifest = context
        .manifests
        .get(plugin_id)
        .ok_or_else(|| ContractError::CliUsage {
            message: format!(
                "workflow {} node {} references unknown external node plugin {}",
                request.workflow_id, request.node_id, plugin_id
            ),
        })?;

    if manifest.kind()? != PluginKind::ExternalNode {
        return Err(ContractError::CliUsage {
            message: format!(
                "workflow {} node {} expected external node plugin kind for {}",
                request.workflow_id, request.node_id, plugin_id
            ),
        });
    }

    let ResolvedNodeInputs {
        values: resolved_inputs,
        resolved_secrets,
    } = resolve_node_inputs(
        &context.root_layout.secrets_dir,
        context.secret_mode,
        &request.inputs,
    )?;
    let host = ExternalNodePluginHost::new(context.root_layout.plugins_dir.clone());
    let response = host
        .execute(
            manifest,
            &ExternalNodePluginRequest {
                contract_version: NODE_PLUGIN_CONTRACT_VERSION.to_owned(),
                plugin_id: plugin_id.to_owned(),
                node_id: request.node_id.clone(),
                operation: "execute".to_owned(),
                requested_capabilities: vec![NODE_PLUGIN_EXECUTE_CAPABILITY.to_owned()],
                input: resolved_inputs,
            },
        )
        .map_err(|source| ContractError::CliUsage {
            message: redact_text(&source.to_string(), &resolved_secrets),
        })?;

    Ok(BuiltinNodeResult {
        outputs: response.output.clone(),
        run_scoped: response.output,
        ..BuiltinNodeResult::default()
    })
}

fn execute_script_node(
    context: &RuntimePluginContext,
    request: &BuiltinNodeRequest,
) -> Result<BuiltinNodeResult, ContractError> {
    let script_spec = parse_script_node_operation(&request.operation)?;
    let interpreter_path =
        resolve_interpreter_for_runtime(script_spec.runtime).ok_or_else(|| {
            ContractError::CliUsage {
                message: format!(
                    "workflow {} node {} cannot resolve interpreter for runtime {}",
                    request.workflow_id,
                    request.node_id,
                    match script_spec.runtime {
                        ScriptRuntime::Python => SCRIPT_RUNTIME_PYTHON,
                        ScriptRuntime::JavaScript => SCRIPT_RUNTIME_JAVASCRIPT,
                    }
                ),
            }
        })?;

    let script_path = context
        .root_layout
        .root
        .join(&script_spec.script_relative_path);
    let ResolvedNodeInputs {
        values: resolved_inputs,
        resolved_secrets,
    } = resolve_node_inputs(
        &context.root_layout.secrets_dir,
        context.secret_mode,
        &request.inputs,
    )?;

    let process = WorkerProcessSpec::new(script_spec.runtime, interpreter_path, script_path);
    let response = context
        .worker_host
        .execute(
            &process,
            &WorkerRequestEnvelope {
                protocol_version: "1.0.0".to_owned(),
                request_id: format!("{}::{}", request.run_id, request.node_id),
                worker_id: request.node_id.clone(),
                workflow_id: request.workflow_id.clone(),
                payload: serde_json::to_value(&resolved_inputs)?,
            },
        )
        .map_err(|source| ContractError::CliUsage {
            message: format!(
                "script worker failed for workflow {} node {}: {}",
                request.workflow_id,
                request.node_id,
                redact_text(&source.to_string(), &resolved_secrets)
            ),
        })?;

    let outputs = match response.output {
        serde_json::Value::Object(values) => values.into_iter().collect(),
        other => BTreeMap::from([(String::from("result"), other)]),
    };

    Ok(BuiltinNodeResult {
        outputs: outputs.clone(),
        run_scoped: outputs,
        ..BuiltinNodeResult::default()
    })
}

fn parse_script_node_operation(operation: &str) -> Result<ScriptNodeSpec, ContractError> {
    let Some((runtime_raw, script_relative_path)) = operation.split_once(':') else {
        return Err(ContractError::CliUsage {
            message: format!(
                "script node operation must use <runtime>:<relative_script_path>, got {operation}"
            ),
        });
    };

    let runtime = match runtime_raw.trim() {
        SCRIPT_RUNTIME_PYTHON => ScriptRuntime::Python,
        SCRIPT_RUNTIME_JAVASCRIPT => ScriptRuntime::JavaScript,
        other => {
            return Err(ContractError::CliUsage {
                message: format!(
                    "script node runtime {other} is unsupported; use python or javascript"
                ),
            });
        }
    };

    let script_relative_path = script_relative_path.trim();
    if script_relative_path.is_empty() {
        return Err(ContractError::CliUsage {
            message: "script node operation requires a relative script path".to_owned(),
        });
    }

    Ok(ScriptNodeSpec {
        runtime,
        script_relative_path: script_relative_path.to_owned(),
    })
}

fn resolve_node_inputs(
    secrets_root: &Path,
    mode: SecretDecryptMode,
    values: &BTreeMap<String, serde_json::Value>,
) -> Result<ResolvedNodeInputs, ContractError> {
    let mut resolved_values = BTreeMap::new();
    let mut resolved_secrets = Vec::new();
    for (key, value) in values {
        let resolved_value =
            resolve_value_with_secrets(secrets_root, mode, value, &mut resolved_secrets)?;
        resolved_values.insert(key.clone(), resolved_value);
    }
    Ok(ResolvedNodeInputs {
        values: resolved_values,
        resolved_secrets,
    })
}

fn resolve_value_with_secrets(
    secrets_root: &Path,
    mode: SecretDecryptMode,
    value: &serde_json::Value,
    resolved_secrets: &mut Vec<SecretValue>,
) -> Result<serde_json::Value, ContractError> {
    match value {
        serde_json::Value::String(raw) if raw.starts_with("secret://") => {
            let reference = SecretReference::parse(raw)?;
            let secret_value = match mode {
                SecretDecryptMode::Gpg => {
                    SecretProvider::new(secrets_root.to_path_buf(), GpgSecretDecryptor::new())
                        .resolve_reference(&reference)?
                }
                SecretDecryptMode::Plaintext => {
                    SecretProvider::new(secrets_root.to_path_buf(), PlaintextSecretDecryptor)
                        .resolve_reference(&reference)?
                }
            };
            resolved_secrets.push(secret_value.clone());
            Ok(serde_json::Value::String(secret_value.expose().to_owned()))
        }
        serde_json::Value::Array(items) => {
            let mut resolved = Vec::with_capacity(items.len());
            for item in items {
                resolved.push(resolve_value_with_secrets(
                    secrets_root,
                    mode,
                    item,
                    resolved_secrets,
                )?);
            }
            Ok(serde_json::Value::Array(resolved))
        }
        serde_json::Value::Object(map) => {
            let mut resolved = serde_json::Map::with_capacity(map.len());
            for (key, item) in map {
                resolved.insert(
                    key.clone(),
                    resolve_value_with_secrets(secrets_root, mode, item, resolved_secrets)?,
                );
            }
            Ok(serde_json::Value::Object(resolved))
        }
        _ => Ok(value.clone()),
    }
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

fn build_builtin_trigger_emissions(
    definitions: &RootDefinitionBundle,
    now_ms: i64,
) -> BTreeMap<String, Vec<TriggerEmission>> {
    let default_workflow_id = definitions
        .workflows
        .first()
        .map(|workflow| workflow.workflow_id.clone())
        .unwrap_or_else(|| String::from("workflow-not-found"));

    let mut emissions = BTreeMap::new();
    for definition in &definitions.triggers {
        if !matches!(definition.kind(), Ok(TriggerKind::Builtin)) {
            continue;
        }

        emissions.insert(
            definition.trigger_id.clone(),
            vec![TriggerEmission {
                event_id: format!("builtin-event-{}", definition.trigger_id),
                workflow_id: default_workflow_id.clone(),
                occurred_at_ms: now_ms,
                source: Some(definition.source.clone()),
                payload: serde_json::json!({
                    "kind": "builtin",
                    "source": definition.source,
                    "symbol": "BTCUSDT"
                }),
                dedup_key: None,
                dedup_window_ms: None,
                cooldown_key: None,
                cooldown_ms: None,
            }],
        );
    }

    emissions
}

fn normalized_request_from_trigger(trigger_request: &TriggerRunRequest) -> NormalizedRunRequest {
    let mut request = NormalizedRunRequest::new(
        trigger_request.run_id.clone(),
        trigger_request.workflow_id.clone(),
    );

    if let serde_json::Value::Object(values) = &trigger_request.payload {
        request.trigger_payload_mapping.extend(
            values
                .iter()
                .map(|(key, value)| (key.clone(), value.clone())),
        );
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

fn resolve_interpreter_for_runtime(runtime: ScriptRuntime) -> Option<PathBuf> {
    let candidates: &[&str] = match runtime {
        ScriptRuntime::Python => &["python3", "python"],
        ScriptRuntime::JavaScript => &["node", "nodejs"],
    };
    let path_env = std::env::var_os("PATH")?;

    for candidate in candidates {
        for root in std::env::split_paths(&path_env) {
            let path = root.join(candidate);
            if path.is_file() {
                return Some(path);
            }
        }
    }

    None
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
            root_override: Some(root.clone()),
        };

        unsafe {
            std::env::set_var(CHAINBOT_SECRET_DECRYPTOR_ENV, SECRET_DECRYPTOR_PLAINTEXT);
        }

        let runtime = request
            .load_runtime_context()
            .expect("initial runtime load should succeed");

        fs::write(
            root.join("triggers").join("external_trigger.toml"),
            "api_version = \"1.0.0\"\ntrigger_id = [\n",
        )
        .expect("mutated trigger file should be writable");

        let serve_output = serve_once_with_lease(&runtime, 1_710_300_000_000)
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
            std::env::remove_var(CHAINBOT_SECRET_DECRYPTOR_ENV);
        }
    }

    #[test]
    fn serve_drains_all_requests_from_snapshot() {
        let _guard = fixture_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());

        let root = prepare_fixture_root("success", "cli-serve-drains-all-requests");
        fs::write(
            root.join("triggers").join("builtin_trigger.toml"),
            "api_version = \"1.0.0\"\ntrigger_id = \"builtin-trigger-e2e\"\nkind = \"manual\"\nsource = \"manual-source\"\nenabled = true\n",
        )
        .expect("builtin trigger fixture should be writable");

        let request = CliRequest {
            command: CliCommand::Serve,
            root_override: Some(root),
        };

        unsafe {
            std::env::set_var(CHAINBOT_SECRET_DECRYPTOR_ENV, SECRET_DECRYPTOR_PLAINTEXT);
        }

        let runtime = request
            .load_runtime_context()
            .expect("runtime load should succeed with builtin alias trigger");
        let serve_output = serve_once_with_lease(&runtime, 1_710_300_100_000)
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
            std::env::remove_var(CHAINBOT_SECRET_DECRYPTOR_ENV);
        }
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
