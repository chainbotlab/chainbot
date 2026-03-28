//! [INPUT]
//! Environment arguments, command parser inputs, and command/runtime adapters.
//!
//! [OUTPUT]
//! Parses command requests, renders help/version output, dispatches top-level commands, and returns stable CLI output/errors.
//!
//! [ROLE]
//! Owns the application-layer CLI entry boundary for parser/help/dispatch behavior.

use crate::app::cli::view::catalog::{CatalogFilterKind, CatalogReference};
use crate::errors::UserFacingError;
use crate::plugin::source::PluginSourceLocator;

mod commands;
mod help;
mod parse;
pub(crate) mod view;

pub(crate) use commands::{
    build_trigger_host_policy, collect_external_trigger_manifests, current_time_ms,
    execute_single_run, load_replayable_trigger_requests, map_ingress_error,
    map_runtime_state_error, merge_trigger_requests, normalized_request_from_trigger,
    RuntimeContext,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliCommand {
    Help(HelpTopic),
    Version,
    Init,
    Status,
    Observe,
    Catalog,
    Plugin,
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
    Plugin,
    Stop,
    Trigger,
    Validate,
    Run,
    Serve,
    ListRuns,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliRequest {
    pub(crate) command: CliCommand,
    pub(crate) json_output: bool,
    pub(crate) trigger_operation: Option<TriggerOperation>,
    pub(crate) catalog_request: Option<CatalogRequest>,
    pub(crate) plugin_command: Option<PluginCommand>,
    pub(crate) observe_request: Option<ObserveRequest>,
    pub(crate) daemon_owner_id: Option<String>,
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
    pub(crate) limit: usize,
    pub(crate) trigger_id: Option<String>,
    pub(crate) run_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogRequest {
    List { filter: Option<CatalogFilterKind> },
    Show { reference: CatalogReference },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PluginCommand {
    Source(PluginSourceRequest),
    Install(PluginInstallRequest),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PluginSourceRequest {
    List { locator: PluginSourceLocator },
    Show {
        locator: PluginSourceLocator,
        plugin_id: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginInstallRequest {
    pub(crate) locator: PluginSourceLocator,
    pub(crate) plugin_id: Option<String>,
    pub(crate) force: bool,
}

impl CliOutput {
    pub fn stdout(&self) -> &str {
        &self.stdout
    }

    pub(crate) fn text(text: impl Into<String>) -> Self {
        let mut stdout = text.into();
        if !stdout.is_empty() && !stdout.ends_with('\n') {
            stdout.push('\n');
        }
        Self { stdout }
    }
}

pub fn run_from_env() -> Result<CliOutput, UserFacingError> {
    CliRequest::from_env()?.dispatch()
}
