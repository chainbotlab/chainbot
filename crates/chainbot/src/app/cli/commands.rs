//! [INPUT]
//! Parsed command intents, environment-resolved ChainBot roots, runtime entrypoints, and CLI read models.
//!
//! [OUTPUT]
//! Dispatches command handlers, adapts runtime outcomes to CLI output, and maps failures to stable CLI errors.
//!
//! [ROLE]
//! Owns the user-facing command boundary for the `chainbot` binary.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

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
use crate::app::runtime::{
    self, current_time_ms, map_runtime_state_error, render_run_status, run_manual_workflow, StopOutcome,
};
use crate::domain::trigger::TriggerDefinition;
use crate::errors::UserFacingError;
use crate::infrastructure::config::{
    load_effective_root_layout, load_trigger_definitions, resolve_root_layout, set_trigger_enabled,
    LocalStorageDefinition, RawDebugArtifactsDefinition, RootConfigDefinition,
    RootLayout, RootPathOverrides, RuntimeHistoryRetentionDefinition,
    StorageDefinition, StorageMode, TriggerToggleResult,
};
use crate::infrastructure::state::RuntimeStateStore;
use crate::plugin::source::{
    build_list_output, build_show_output, discover_source_repository, materialize_source,
    prepare_installable_plugin, resolve_plugin_selection, InstallTransaction,
    PluginSourceDescriptor,
};
use crate::plugin::PluginManifest;
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
        let mut state_store = RuntimeStateStore::open_read_only(&storage_config, current_time_ms()?)
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
        let mut state_store = RuntimeStateStore::open_read_only(&storage_config, current_time_ms()?)
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
        let mut store = RuntimeStateStore::open_read_only(&storage_config, current_time_ms()?)
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
        let run_result = run_manual_workflow(self.resolve_existing_root()?)?;
        Ok(CliOutput::text(format!(
            "run completed: run_id={} workflow_id={} status={}",
            run_result.run_id,
            run_result.workflow_id,
            render_run_status(run_result.status)
        )))
    }

    pub(crate) fn execute_serve(&self) -> Result<CliOutput, UserFacingError> {
        let root_layout = self.resolve_existing_root()?;
        let started = runtime::execute_serve(&root_layout)?;
        Ok(CliOutput::text(format!(
            "serve started: owner={} pid={} root={}\nuse `chainbot status --json` for health and `chainbot observe` for persisted runtime output",
            started.owner_id,
            started.pid,
            root_layout.root.display()
        )))
    }

    pub(crate) fn execute_stop(&self) -> Result<CliOutput, UserFacingError> {
        let root_layout = self.resolve_existing_root()?;
        let message = match runtime::execute_stop(&root_layout)? {
            StopOutcome::NoActiveDaemon => "stop completed: no active daemon",
            StopOutcome::StaleStateCleaned => "stop completed: stale daemon state cleaned up",
            StopOutcome::ActiveLeaseCleared => "stop completed: active lease cleared",
            StopOutcome::ShutdownRequested => "stop completed: daemon shutdown requested",
        };
        Ok(CliOutput::text(message))
    }

    pub(crate) fn execute_internal_serve_daemon(&self) -> Result<CliOutput, UserFacingError> {
        let owner_id = self.daemon_owner_id.clone().ok_or_else(|| {
            UserFacingError::state_with_code(
                "daemon_start_failed",
                "Internal daemon start is missing the daemon owner identifier.",
            )
        })?;
        runtime::execute_internal_serve_daemon(&self.resolve_existing_root()?, owner_id)?;
        Ok(CliOutput::text(String::new()))
    }

    pub(crate) fn resolve_existing_root(&self) -> Result<RootLayout, UserFacingError> {
        let bootstrap_layout = resolve_root_layout().map_err(UserFacingError::from_contract)?;
        maybe_prepare_e2e_root(&bootstrap_layout)?;
        runtime::resolve_existing_root(&bootstrap_layout)
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
