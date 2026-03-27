//! [INPUT]
//! Root layout metadata, loaded workflow/trigger/plugin definitions, and runtime snapshots from run and daemon state.
//!
//! [OUTPUT]
//! Builds status read-model payloads and renders stable human-readable status output.
//!
//! [ROLE]
//! Owns status command read-model shape and rendering outside command dispatch code.

use std::collections::BTreeMap;

use crate::app::cli::view::catalog::{build_status_plugin_summary, StatusPluginSummaryView};
use crate::domain::state::{RunRecordSummary, RunStatus, ServeLeaseState, TriggerSnapshotRecord};
use crate::domain::trigger::TriggerDefinition;
use crate::domain::workflow::WorkflowDefinition;
use crate::infrastructure::config::RootLayout;
use crate::infrastructure::state::RuntimeDaemonStatus;
use crate::plugin::PluginManifest;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub(crate) struct StatusOutput {
    pub root: StatusRootView,
    pub serve: StatusServeView,
    pub workflows: Vec<StatusWorkflowView>,
    pub triggers: Vec<StatusTriggerView>,
    pub plugins: StatusPluginSummaryView,
    pub summary: StatusSummaryView,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub(crate) struct StatusRootView {
    pub path: String,
    pub profile: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub(crate) struct StatusServeView {
    pub state: ServeLeaseState,
    pub owner: Option<String>,
    pub pid: Option<i64>,
    pub started_at_ms: Option<i64>,
    pub last_heartbeat_at_ms: Option<i64>,
    pub lease_expires_at_ms: Option<i64>,
    pub last_reload_at_ms: Option<i64>,
    pub stop_requested_at_ms: Option<i64>,
    pub last_error_code: Option<String>,
    pub last_error_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub(crate) struct StatusWorkflowView {
    pub workflow_id: String,
    pub last_run_status: Option<RunStatus>,
    pub last_run_id: Option<String>,
    pub last_started_at_ms: Option<i64>,
    pub last_finished_at_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub(crate) struct StatusTriggerView {
    pub trigger_id: String,
    pub enabled: bool,
    pub workflow_id: String,
    pub last_event_id: Option<String>,
    pub last_accepted_at_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub(crate) struct StatusSummaryView {
    pub workflow_count: usize,
    pub trigger_count: usize,
    pub run_count: usize,
    pub running_run_count: usize,
}

pub(crate) fn build_status_output(
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

pub(crate) fn render_status_output(status: &StatusOutput) -> String {
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

fn render_run_status(status: RunStatus) -> &'static str {
    match status {
        RunStatus::Pending => "pending",
        RunStatus::Running => "running",
        RunStatus::Succeeded => "succeeded",
        RunStatus::Failed => "failed",
    }
}
