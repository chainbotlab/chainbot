//! [INPUT]
//! Runtime history store reads for recent run summaries, workflow logs, trigger events, and archive counters.
//!
//! [OUTPUT]
//! Observe read-model payloads plus stable human-readable rendering for runtime-history inspection.
//!
//! [ROLE]
//! Owns observe command read-model construction and rendering outside CLI parsing/orchestration.

use crate::domain::state::{RunRecordSummary, TriggerEventRecord, WorkflowRuntimeLogEntry};
use crate::infrastructure::state::{
    RuntimeHistoryArchiveCounts, RuntimeStateError, RuntimeStateStore,
};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub(crate) struct ObserveOutput {
    pub summary: ObserveSummaryView,
    pub runs: Vec<RunRecordSummary>,
    pub workflow_logs: Vec<WorkflowRuntimeLogEntry>,
    pub trigger_events: Vec<TriggerEventRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub(crate) struct ObserveSummaryView {
    pub requested_limit: usize,
    pub archived: RuntimeHistoryArchiveCounts,
}

pub(crate) fn build_observe_output(
    state_store: &mut RuntimeStateStore,
    limit: usize,
    trigger_id: Option<&str>,
    run_id: Option<&str>,
) -> Result<ObserveOutput, RuntimeStateError> {
    Ok(ObserveOutput {
        summary: ObserveSummaryView {
            requested_limit: limit,
            archived: state_store.archived_history_counts()?,
        },
        runs: state_store.list_recent_run_summaries(limit)?,
        workflow_logs: state_store.list_recent_workflow_log_entries(limit, run_id)?,
        trigger_events: state_store.list_recent_trigger_records(limit, trigger_id)?,
    })
}

pub(crate) fn render_observe_output(output: &ObserveOutput) -> String {
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

fn render_run_status(status: crate::domain::state::RunStatus) -> &'static str {
    match status {
        crate::domain::state::RunStatus::Pending => "pending",
        crate::domain::state::RunStatus::Running => "running",
        crate::domain::state::RunStatus::Succeeded => "succeeded",
        crate::domain::state::RunStatus::Failed => "failed",
    }
}
