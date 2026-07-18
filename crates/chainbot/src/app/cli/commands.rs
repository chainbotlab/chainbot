//! [INPUT]
//! Process arguments, environment-resolved ChainBot roots, and runtime services from config, state, trigger, executor, worker, and secrets modules.
//!
//! [OUTPUT]
//! Parses commands, executes help, plugin/source/install, init, status, observe, trigger, validate, run, serve, and list-runs flows, and maps failures to stable CLI output and exit codes.
//!
//! [ROLE]
//! Owns the user-facing command boundary for the `chainbot` binary.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use super::help::{help_text, render_version_output};
use super::{
    CatalogRequest, CliCommand, CliOutput, CliRequest, PluginCommand, PluginSourceRequest,
    TriggerOperation,
};
use crate::app::cli::view::catalog::{
    build_catalog_list, build_catalog_show, render_catalog_list, render_catalog_show,
    CatalogFilterKind, CatalogReference,
};
use crate::app::cli::view::observe::{build_observe_output, render_observe_output};
use crate::app::cli::view::plugin_source::{
    render_plugin_install_success, render_plugin_source_list, render_plugin_source_show,
};
use crate::app::cli::view::status::{build_status_output, render_status_output};
use crate::app::definitions::{
    collect_compatibility_warnings, load_root_definition_bundle, CompatibilityWarning,
};
use crate::app::runtime::daemon;
use crate::app::runtime::execution::ExecutionPlane;
use crate::app::runtime::execution::PluginActivationRuntime;
#[cfg(test)]
use crate::app::runtime::external_triggers::ExternalTriggerSupervisor;
use crate::builtins::nodes::script_worker::{WorkerHost, WorkerHostLimits};
use crate::builtins::{build_builtin_registry, BuiltinRuntimeContext, SecretDecryptMode};
use crate::domain::runtime::{NormalizedRunRequest, WorkflowRunStatus};
#[cfg(test)]
use crate::domain::state::StagedTriggerEventRecord;
use crate::domain::state::{RunExecutionFence, RunRecordSummary, RunStatus, TriggerEventRecord};
use crate::domain::trigger::{
    TriggerDefinition, TriggerPluginActivationBindings, TriggerPluginHostPolicy, TriggerRunRequest,
    REQUIRED_TRIGGER_PLUGIN_CAPABILITY,
};
use crate::domain::workflow::WorkflowDefinition;
use crate::errors::{ContractError, UserFacingError};
use crate::infrastructure::config::{
    load_effective_root_layout, load_trigger_definitions, resolve_root_layout, set_trigger_enabled,
    LocalStorageDefinition, RawDebugArtifactsDefinition, RootConfigDefinition,
    RootDefinitionBundle, RootLayout, RootPathOverrides, RuntimeHistoryRetentionDefinition,
    RuntimeStorageConfig, StorageDefinition, StorageMode, TriggerToggleResult,
};
use crate::infrastructure::state::{sanitize_path_component, RuntimeStateError, RuntimeStateStore};
use crate::ingress::IngressRuntimeError;
use crate::plugin::source::{
    build_list_output, build_show_output, discover_source_repository, materialize_source,
    prepare_installable_plugin, resolve_plugin_selection, InstallTransaction,
    PluginSourceDescriptor,
};
use crate::plugin::{HostCancellation, PluginKind, PluginManifest};
use crate::secrets::SecretReference;

const CHAINBOT_SECRET_DECRYPTOR_ENV: &str = "CHAINBOT_SECRET_DECRYPTOR";
const SECRET_DECRYPTOR_PLAINTEXT: &str = "plaintext";
const INIT_MANIFEST_VERSION: &str = "2.0.0";
const CHAINBOT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug)]
struct InitResult {
    root: PathBuf,
    created_paths: Vec<PathBuf>,
    reused_paths: Vec<PathBuf>,
}

#[derive(Debug, Serialize)]
struct ValidationOutput {
    valid: bool,
    root: String,
    warnings: Vec<CompatibilityWarning>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<ValidationErrorOutput>,
}

#[derive(Debug, Serialize)]
struct ValidationErrorOutput {
    code: &'static str,
    message: String,
}

#[derive(Debug)]
pub(crate) struct RuntimeContext {
    pub(crate) root_layout: RootLayout,
    pub(crate) definitions: RootDefinitionBundle,
    pub(crate) storage_config: RuntimeStorageConfig,
    pub(crate) state_store: RuntimeStateStore,
    pub(crate) secret_mode: SecretDecryptMode,
    pub(crate) worker_host: WorkerHost,
}

#[derive(Debug)]
pub(crate) struct SingleRunResult {
    pub(crate) run_id: String,
    pub(crate) workflow_id: String,
    pub(crate) status: RunStatus,
}

impl CliRequest {
    pub fn dispatch(&self) -> Result<CliOutput, UserFacingError> {
        match self.command {
            CliCommand::Help(topic) => Ok(CliOutput::text(help_text(topic))),
            CliCommand::Version => Ok(CliOutput::text(render_version_output())),
            CliCommand::Init => self.execute_init(),
            CliCommand::Status => self.execute_status(),
            CliCommand::Observe => self.execute_observe(),
            CliCommand::Catalog => self.execute_catalog(),
            CliCommand::Plugin => self.execute_plugin(),
            CliCommand::Stop => self.execute_stop(),
            CliCommand::Trigger => self.execute_trigger(),
            CliCommand::Validate => self.execute_validate(self.json_output),
            CliCommand::Run => self.execute_run(),
            CliCommand::Serve => self.execute_serve(),
            CliCommand::InternalServeDaemon => self.execute_internal_serve_daemon(),
            CliCommand::ListRuns => self.execute_list_runs(),
        }
    }

    pub(crate) fn execute_init(&self) -> Result<CliOutput, UserFacingError> {
        let root_layout = resolve_root_layout().map_err(UserFacingError::from_contract)?;
        let init_result = initialize_root_layout(&root_layout)?;
        let _ =
            load_root_definition_bundle(&root_layout).map_err(UserFacingError::from_contract)?;
        Ok(CliOutput::text(render_init_output(&init_result)))
    }

    pub(crate) fn execute_status(&self) -> Result<CliOutput, UserFacingError> {
        let root_layout = self.resolve_existing_root()?;
        let definitions =
            load_root_definition_bundle(&root_layout).map_err(UserFacingError::from_contract)?;
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

    pub(crate) fn execute_observe(&self) -> Result<CliOutput, UserFacingError> {
        let root_layout = self.resolve_existing_root()?;
        let definitions =
            load_root_definition_bundle(&root_layout).map_err(UserFacingError::from_contract)?;
        let storage_config = definitions
            .root_config
            .resolve_runtime_storage(&root_layout.root)
            .map_err(UserFacingError::from_contract)?;
        let mut state_store = RuntimeStateStore::open(&storage_config, current_time_ms()?)
            .map_err(|error| map_runtime_state_error("open runtime state store", error))?;
        let observe_request = self.observe_request.as_ref().ok_or_else(|| {
            UserFacingError::usage("Missing observe request. Run `chainbot help observe`.")
        })?;
        let payload = build_observe_output(
            &mut state_store,
            observe_request.limit,
            observe_request.trigger_id.as_deref(),
            observe_request.run_id.as_deref(),
        )
        .map_err(|error| map_runtime_state_error("build observe output", error))?;

        if self.json_output {
            let stdout = serde_json::to_string_pretty(&payload).map_err(|source| {
                UserFacingError::state(format!("Failed to serialize observe payload: {source}"))
            })?;
            return Ok(CliOutput::text(stdout));
        }

        Ok(CliOutput::text(render_observe_output(&payload)))
    }

    pub(crate) fn execute_catalog(&self) -> Result<CliOutput, UserFacingError> {
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

    pub(crate) fn execute_plugin(&self) -> Result<CliOutput, UserFacingError> {
        let plugin_command = self.plugin_command.as_ref().ok_or_else(|| {
            UserFacingError::usage("Missing plugin command. Run `chainbot help plugin`.")
        })?;
        match plugin_command {
            PluginCommand::Source(request) => {
                let locator = match request {
                    PluginSourceRequest::List { locator }
                    | PluginSourceRequest::Show { locator, .. } => locator,
                };
                let materialized =
                    materialize_source(locator).map_err(UserFacingError::from_contract)?;
                let descriptor = PluginSourceDescriptor {
                    source_kind: locator.kind().as_str().to_owned(),
                    target: locator.target(),
                    git_ref: locator.requested_ref().map(str::to_owned),
                    resolved_ref: materialized.resolved_ref.clone(),
                };
                let repository = discover_source_repository(&materialized, descriptor)
                    .map_err(UserFacingError::from_contract)?;
                match request {
                    PluginSourceRequest::List { .. } => {
                        let payload = build_list_output(&repository);
                        if self.json_output {
                            let stdout =
                                serde_json::to_string_pretty(&payload).map_err(|source| {
                                    UserFacingError::state(format!(
                                        "Failed to serialize plugin source list payload: {source}"
                                    ))
                                })?;
                            return Ok(CliOutput::text(stdout));
                        }
                        Ok(CliOutput::text(render_plugin_source_list(&payload)))
                    }
                    PluginSourceRequest::Show { plugin_id, .. } => {
                        let plugin = resolve_plugin_selection(
                            &repository,
                            plugin_id.as_deref(),
                            "chainbot plugin source show",
                        )
                        .map_err(UserFacingError::from_contract)?;
                        let payload = build_show_output(&repository, plugin);
                        if self.json_output {
                            let stdout =
                                serde_json::to_string_pretty(&payload).map_err(|source| {
                                    UserFacingError::state(format!(
                                        "Failed to serialize plugin source show payload: {source}"
                                    ))
                                })?;
                            return Ok(CliOutput::text(stdout));
                        }
                        Ok(CliOutput::text(render_plugin_source_show(&payload)))
                    }
                }
            }
            PluginCommand::Install(request) => {
                let root_layout = self.load_definition_root()?;
                let materialized =
                    materialize_source(&request.locator).map_err(UserFacingError::from_contract)?;
                let descriptor = PluginSourceDescriptor {
                    source_kind: request.locator.kind().as_str().to_owned(),
                    target: request.locator.target(),
                    git_ref: request.locator.requested_ref().map(str::to_owned),
                    resolved_ref: materialized.resolved_ref.clone(),
                };
                let repository = discover_source_repository(&materialized, descriptor)
                    .map_err(UserFacingError::from_contract)?;
                let plugin = resolve_plugin_selection(
                    &repository,
                    request.plugin_id.as_deref(),
                    "chainbot plugin install",
                )
                .map_err(UserFacingError::from_contract)?;
                let prepared = prepare_installable_plugin(&materialized, plugin)
                    .map_err(UserFacingError::from_contract)?;
                let (transaction, result) =
                    InstallTransaction::begin(&root_layout.plugins_dir, &prepared, request.force)
                        .map_err(UserFacingError::from_contract)?;
                match load_root_definition_bundle(&root_layout) {
                    Ok(_) => {
                        transaction
                            .finalize()
                            .map_err(UserFacingError::from_contract)?;
                    }
                    Err(error) => {
                        let rollback_result = transaction.rollback();
                        if let Err(rollback_error) = rollback_result {
                            return Err(UserFacingError::state(format!(
                                "Plugin install validation failed and rollback also failed: validation error: {error}; rollback error: {rollback_error}"
                            )));
                        }
                        return Err(UserFacingError::validation(format!(
                            "Plugin install was rolled back because the root no longer validated: {error}"
                        )));
                    }
                }
                Ok(CliOutput::text(render_plugin_install_success(
                    &request.locator.display_label(),
                    &result,
                    materialized.resolved_ref.as_deref(),
                )))
            }
        }
    }

    pub(crate) fn execute_validate(
        &self,
        json_output: bool,
    ) -> Result<CliOutput, UserFacingError> {
        let layout = match self.resolve_existing_root() {
            Ok(layout) => layout,
            Err(error) if json_output => {
                return Ok(render_validation_json_error(None, error));
            }
            Err(error) => return Err(error),
        };
        let definitions = match load_root_definition_bundle(&layout) {
            Ok(definitions) => definitions,
            Err(error) if json_output => {
                return Ok(render_validation_json_error(
                    Some(layout.root.display().to_string()),
                    UserFacingError::from_contract(error),
                ));
            }
            Err(error) => return Err(UserFacingError::from_contract(error)),
        };
        let warnings =
            collect_compatibility_warnings(&definitions.workflows, &definitions.plugins);
        if json_output {
            let payload = ValidationOutput {
                valid: true,
                root: layout.root.display().to_string(),
                warnings,
                error: None,
            };
            let stdout = serde_json::to_string_pretty(&payload).map_err(|source| {
                UserFacingError::state(format!(
                    "Failed to serialize validation payload: {source}"
                ))
            })?;
            return Ok(CliOutput::text(stdout));
        }

        let mut lines = vec![format!("validated root: {}", layout.root.display())];
        lines.extend(warnings.into_iter().map(|warning| {
            format!(
                "warning[{}] {}: `{}`; use `{}`",
                warning.code,
                warning.source_location,
                warning.original_reference,
                warning.replacement
            )
        }));
        Ok(CliOutput::text(lines.join("\n")))
    }

    pub(crate) fn execute_trigger(&self) -> Result<CliOutput, UserFacingError> {
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

    pub(crate) fn execute_list_runs(&self) -> Result<CliOutput, UserFacingError> {
        let layout = self.resolve_existing_root()?;
        let definitions =
            load_root_definition_bundle(&layout).map_err(UserFacingError::from_contract)?;
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

    pub(crate) fn execute_run(&self) -> Result<CliOutput, UserFacingError> {
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

    pub(crate) fn execute_serve(&self) -> Result<CliOutput, UserFacingError> {
        daemon::execute_serve(self)
    }

    pub(crate) fn execute_stop(&self) -> Result<CliOutput, UserFacingError> {
        daemon::execute_stop(self)
    }

    pub(crate) fn execute_internal_serve_daemon(&self) -> Result<CliOutput, UserFacingError> {
        daemon::execute_internal_serve_daemon(self)
    }

    pub(crate) fn resolve_existing_root(&self) -> Result<RootLayout, UserFacingError> {
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
        let _ = load_root_definition_bundle(&layout).map_err(UserFacingError::from_contract)?;
        Ok(layout)
    }

    fn resolve_trigger_root(&self) -> Result<RootLayout, UserFacingError> {
        let bootstrap_layout = resolve_root_layout().map_err(UserFacingError::from_contract)?;
        maybe_prepare_e2e_root(&bootstrap_layout)?;
        load_trigger_definitions(&bootstrap_layout).map_err(UserFacingError::from_contract)?;
        load_effective_root_layout(&bootstrap_layout).map_err(UserFacingError::from_contract)
    }

    pub(crate) fn load_runtime_context(&self) -> Result<RuntimeContext, UserFacingError> {
        let root_layout = self.resolve_existing_root()?;
        let definitions =
            load_root_definition_bundle(&root_layout).map_err(UserFacingError::from_contract)?;
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

    pub(crate) fn load_root_definition_bundle(
        &self,
    ) -> Result<RootDefinitionBundle, UserFacingError> {
        let root_layout = self.resolve_existing_root()?;
        load_root_definition_bundle(&root_layout).map_err(UserFacingError::from_contract)
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

fn load_catalog_plugins_strict() -> Result<Vec<PluginManifest>, UserFacingError> {
    let root_layout = resolve_root_layout().map_err(UserFacingError::from_contract)?;
    let bundle =
        load_root_definition_bundle(&root_layout).map_err(UserFacingError::from_contract)?;
    Ok(bundle.plugins)
}

fn load_catalog_plugins_if_available() -> Result<Vec<PluginManifest>, UserFacingError> {
    let root_layout = resolve_root_layout().map_err(UserFacingError::from_contract)?;
    match load_root_definition_bundle(&root_layout) {
        Ok(bundle) => Ok(bundle.plugins),
        Err(crate::errors::ContractError::MissingDirectory { kind: "root", .. })
        | Err(crate::errors::ContractError::MissingFile {
            kind: "root config",
            ..
        }) => Ok(Vec::new()),
        Err(error) => Err(UserFacingError::from_contract(error)),
    }
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
        plugin_activation: BTreeMap::new(),
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

pub(crate) fn current_time_ms() -> Result<i64, UserFacingError> {
    let duration = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|_| {
        UserFacingError::state("System clock is before UNIX_EPOCH; cannot continue.")
    })?;
    Ok(duration.as_millis().min(i64::MAX as u128) as i64)
}

pub(crate) fn map_runtime_state_error(action: &str, error: RuntimeStateError) -> UserFacingError {
    UserFacingError::state(format!("Failed to {action}: {error}"))
}

pub(crate) fn map_ingress_error(error: IngressRuntimeError) -> UserFacingError {
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

pub(crate) fn execute_single_run(
    runtime: &mut RuntimeContext,
    request: NormalizedRunRequest,
    started_at_ms: i64,
) -> Result<SingleRunResult, UserFacingError> {
    execute_single_run_with_fence(runtime, request, started_at_ms, None)
}

pub(crate) fn execute_single_run_with_fence(
    runtime: &mut RuntimeContext,
    request: NormalizedRunRequest,
    started_at_ms: i64,
    fence: Option<&RunExecutionFence>,
) -> Result<SingleRunResult, UserFacingError> {
    execute_single_run_with_fence_and_cancellation(
        runtime,
        request,
        started_at_ms,
        fence,
        &HostCancellation::default(),
    )
}

pub(crate) fn execute_single_run_with_fence_and_cancellation(
    runtime: &mut RuntimeContext,
    request: NormalizedRunRequest,
    started_at_ms: i64,
    fence: Option<&RunExecutionFence>,
    cancellation: &HostCancellation,
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
        fence,
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

    let plane = build_execution_plane(runtime, cancellation)?;
    let report = plane.execute(&request);
    let finished_at_ms = current_time_ms()?;

    match report {
        Ok(report) => {
            let status = if report.status == WorkflowRunStatus::Succeeded {
                RunStatus::Succeeded
            } else {
                RunStatus::Failed
            };

            let terminal_message = format!(
                "execution finished with {} and {} node failure(s)",
                render_run_status(status),
                report.node_failures.len()
            );
            write_terminal_run(
                &mut runtime.state_store,
                &run_id,
                &workflow_id,
                status,
                started_at_ms,
                finished_at_ms,
                "run_finished",
                &terminal_message,
                fence,
            )
            .map_err(|error| map_runtime_state_error("persist finished run", error))?;

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
            write_terminal_run(
                &mut runtime.state_store,
                &run_id,
                &workflow_id,
                RunStatus::Failed,
                started_at_ms,
                finished_at_ms,
                "run_failed",
                &detail,
                fence,
            )
            .map_err(|write_error| map_runtime_state_error("persist failed run", write_error))?;

            Err(UserFacingError::unavailable(format!(
                "Run {run_id} failed: {detail}"
            )))
        }
    }
}

fn build_execution_plane(
    runtime: &RuntimeContext,
    cancellation: &HostCancellation,
) -> Result<ExecutionPlane, UserFacingError> {
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
        plugin_activation_bindings(&runtime.definitions.root_config)
            .map_err(UserFacingError::from_contract)?,
        runtime.root_layout.plugins_dir.clone(),
        runtime.root_layout.secrets_dir.clone(),
        runtime.secret_mode,
    )
    .map(|plane| plane.with_cancellation(cancellation.clone()))
    .map_err(UserFacingError::from_contract)
}

pub(crate) fn build_trigger_host_policy(
    root_config: &RootConfigDefinition,
    manifests: &[PluginManifest],
    plugins_root_dir: &Path,
    secrets_root_dir: &Path,
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
        plugin_activation: trigger_plugin_activation_bindings(root_config).unwrap_or_default(),
        secrets_root_dir: secrets_root_dir.to_path_buf(),
    }
}

fn trigger_plugin_activation_bindings(
    root_config: &RootConfigDefinition,
) -> Result<BTreeMap<String, TriggerPluginActivationBindings>, ContractError> {
    let mut bindings = BTreeMap::new();
    for (plugin_id, activation) in &root_config.plugin_activation {
        let mut slots = BTreeMap::new();
        for (slot, secret_ref) in &activation.secret_bindings {
            slots.insert(slot.clone(), SecretReference::parse(secret_ref)?);
        }
        bindings.insert(
            plugin_id.clone(),
            TriggerPluginActivationBindings {
                secret_bindings: slots,
                allowed_origins: activation.allowed_origins.clone(),
            },
        );
    }
    Ok(bindings)
}

fn plugin_activation_bindings(
    root_config: &RootConfigDefinition,
) -> Result<BTreeMap<String, PluginActivationRuntime>, ContractError> {
    let mut bindings = BTreeMap::new();
    for (plugin_id, activation) in &root_config.plugin_activation {
        let mut slots = BTreeMap::new();
        for (slot, secret_ref) in &activation.secret_bindings {
            slots.insert(slot.clone(), SecretReference::parse(secret_ref)?);
        }
        bindings.insert(
            plugin_id.clone(),
            PluginActivationRuntime {
                secret_bindings: slots,
                allowed_origins: activation.allowed_origins.clone(),
            },
        );
    }
    Ok(bindings)
}

pub(crate) fn collect_external_trigger_manifests(
    plugin_manifests: &[PluginManifest],
) -> Vec<PluginManifest> {
    plugin_manifests
        .iter()
        .filter_map(|manifest| {
            manifest
                .kind()
                .ok()
                .filter(|kind| *kind == PluginKind::ExternalTrigger)
                .map(|_| manifest.clone())
        })
        .collect()
}

pub(crate) fn normalized_request_from_trigger(
    trigger_request: &TriggerRunRequest,
) -> NormalizedRunRequest {
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

pub(crate) fn load_replayable_trigger_requests(
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

pub(crate) fn merge_trigger_requests(
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
    fence: Option<&RunExecutionFence>,
) -> Result<(), RuntimeStateError> {
    let summary = RunRecordSummary {
        schema_version: "1.0.0".to_owned(),
        run_id: run_id.to_owned(),
        workflow_id: workflow_id.to_owned(),
        status,
        started_at_ms,
        finished_at_ms,
        owner_id: fence.map(|fence| fence.owner_id.clone()),
        lease_generation: fence.map(|fence| fence.lease_generation),
    };
    if finished_at_ms.is_some() && fence.is_some() {
        if !state_store.write_fenced_terminal_run_summary(&summary, finished_at_ms.unwrap_or(started_at_ms))? {
            return Err(RuntimeStateError::LeaseFenceLost {
                run_id: run_id.to_owned(),
            });
        }
        return Ok(());
    }
    state_store.write_run_summary(&summary)
}

fn write_terminal_run(
    state_store: &mut RuntimeStateStore,
    run_id: &str,
    workflow_id: &str,
    status: RunStatus,
    started_at_ms: i64,
    finished_at_ms: i64,
    event: &str,
    message: &str,
    fence: Option<&RunExecutionFence>,
) -> Result<(), RuntimeStateError> {
    if let Some(fence) = fence {
        let summary = RunRecordSummary {
            schema_version: "1.0.0".to_owned(),
            run_id: run_id.to_owned(),
            workflow_id: workflow_id.to_owned(),
            status,
            started_at_ms,
            finished_at_ms: Some(finished_at_ms),
            owner_id: Some(fence.owner_id.clone()),
            lease_generation: Some(fence.lease_generation),
        };
        if !state_store.write_fenced_terminal_run_with_log(
            &summary,
            event,
            message,
            finished_at_ms,
        )? {
            return Err(RuntimeStateError::LeaseFenceLost {
                run_id: run_id.to_owned(),
            });
        }
        return Ok(());
    }

    write_log_entry(state_store, run_id, event, message, finished_at_ms)?;
    write_run_status(
        state_store,
        run_id,
        workflow_id,
        status,
        started_at_ms,
        Some(finished_at_ms),
        None,
    )
}

fn render_validation_json_error(
    root: Option<String>,
    error: UserFacingError,
) -> CliOutput {
    let payload = ValidationOutput {
        valid: false,
        root: root.unwrap_or_default(),
        warnings: Vec::new(),
        error: Some(ValidationErrorOutput {
            code: error.error_code(),
            message: error.to_string(),
        }),
    };
    let stdout = serde_json::to_string_pretty(&payload)
        .expect("validation error payload should serialize");
    CliOutput::text(stdout)
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

    use crate::app::runtime::daemon::{serve_once_with_lease, ServeLeaseSupervisor};
    use crate::infrastructure::config::RuntimeStorageBackend;

    const TEST_SERVE_LEASE_TTL_MS: i64 = 30_000;

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
            plugin_command: None,
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
        let mut external_trigger_supervisor = ExternalTriggerSupervisor::new("test-owner");
        let serve_output = serve_once_with_lease(
            &mut runtime,
            1_710_300_000_000,
            &mut lease_supervisor,
            &mut external_trigger_supervisor,
        )
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
            plugin_command: None,
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
        let mut external_trigger_supervisor = ExternalTriggerSupervisor::new("test-owner");
        let serve_output = serve_once_with_lease(
            &mut runtime,
            1_710_300_100_000,
            &mut lease_supervisor,
            &mut external_trigger_supervisor,
        )
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
    fn serve_bridges_pending_staged_external_events() {
        let _guard = fixture_lock()
            .lock()
            .unwrap_or_else(|error| error.into_inner());

        let root = prepare_fixture_root("success", "cli-serve-bridges-staged-external-events");
        let request = CliRequest {
            command: CliCommand::Serve,
            json_output: false,
            trigger_operation: None,
            catalog_request: None,
            plugin_command: None,
            observe_request: None,
            daemon_owner_id: None,
        };

        unsafe {
            std::env::set_var("CHAINBOT_CONFIG_DIR", &root);
            std::env::set_var(CHAINBOT_SECRET_DECRYPTOR_ENV, SECRET_DECRYPTOR_PLAINTEXT);
        }

        let mut runtime = request
            .load_runtime_context()
            .expect("runtime load should succeed for staged external bridge coverage");
        runtime
            .state_store
            .append_staged_trigger_event_record(&StagedTriggerEventRecord {
                schema_version: String::from("1.0.0"),
                staging_id: String::from("staging-external-bridge-1"),
                trigger_id: String::from("external-trigger-e2e"),
                workflow_id: String::from("wf-e2e"),
                event_id: String::from("external-trigger-e2e:event-staged"),
                source: String::from("external-trigger-plugin"),
                occurred_at_ms: 1_710_300_150_000,
                staged_at_ms: 1_710_300_150_001,
                checkpoint: Some(String::from("cp-staged-1")),
                payload: serde_json::json!({"symbol": "ETHUSDT"}),
                dedup_key: None,
                dedup_window_ms: None,
                cooldown_key: None,
                cooldown_ms: None,
                accepted_at_ms: None,
                last_error: None,
            })
            .expect("staged external trigger row should persist before serve turn");

        let mut lease_supervisor = ServeLeaseSupervisor::new(
            runtime.storage_config.clone(),
            String::from("test-owner"),
            i64::from(std::process::id()),
            1_710_300_150_000,
        );
        let mut external_trigger_supervisor = ExternalTriggerSupervisor::new("test-owner");
        let serve_output = serve_once_with_lease(
            &mut runtime,
            1_710_300_150_010,
            &mut lease_supervisor,
            &mut external_trigger_supervisor,
        )
        .expect("serve should bridge staged external rows through trigger acceptance");

        assert!(serve_output
            .stdout()
            .contains("serve completed: executed 2 accepted trigger event(s)"));
        assert!(serve_output
            .stdout()
            .contains("event_id=external-trigger-e2e:event-staged"));

        let pending = runtime
            .state_store
            .list_pending_staged_trigger_event_records("external-trigger-e2e", 10)
            .expect("pending staged rows should be queryable after serve bridge");
        assert!(pending.is_empty());

        unsafe {
            std::env::remove_var("CHAINBOT_CONFIG_DIR");
            std::env::remove_var(CHAINBOT_SECRET_DECRYPTOR_ENV);
        }
    }

    #[test]
    fn serve_lease_supervisor_renews_same_owner_lease() {
        let root = prepare_fixture_root("success", "cli-serve-lease-renewal");
        let storage_config = RuntimeStorageConfig {
            backend: RuntimeStorageBackend::Local {
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
                .try_acquire_serve_lease("owner-renew", 1_710_300_200_000, TEST_SERVE_LEASE_TTL_MS,)
                .expect("initial lease should acquire"),
            crate::domain::state::LeaseAcquireResult::Acquired { .. }
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
        assert!(matches!(
            snapshot.state,
            crate::domain::state::ServeLeaseState::Active
        ));
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
