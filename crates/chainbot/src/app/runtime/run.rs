//! [INPUT]
//! Runtime contexts, normalized run requests, workflow definitions, execution fences, and host cancellation.
//!
//! [OUTPUT]
//! Selects manual workflows, executes runs, and persists run summaries and runtime logs with lease-fenced terminal commits.
//!
//! [ROLE]
//! Owns the application run lifecycle shared by manual invocation and daemon trigger execution.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};

use super::context::{current_time_ms, map_runtime_state_error, RuntimeContext};
use super::execution::ExecutionPlane;
use super::plugin_activation::parse_plugin_activation_bindings;
use crate::builtins::{build_builtin_registry, BuiltinRuntimeContext};
use crate::domain::runtime::{NormalizedRunRequest, WorkflowRunStatus};
use crate::domain::state::{RunExecutionFence, RunRecordSummary, RunStatus};
use crate::domain::workflow::WorkflowDefinition;
use crate::errors::UserFacingError;
use crate::infrastructure::config::RootLayout;
use crate::infrastructure::state::{sanitize_path_component, RuntimeStateError, RuntimeStateStore};
use crate::plugin::HostCancellation;

// Two concurrent `run` invocations can share a millisecond, so the run id also
// carries the process id and a process-local sequence to stay collision-free.
static MANUAL_RUN_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Debug)]
pub(crate) struct SingleRunResult {
    pub(crate) run_id: String,
    pub(crate) workflow_id: String,
    pub(crate) status: RunStatus,
}

pub(crate) fn run_manual_workflow(root_layout: RootLayout) -> Result<SingleRunResult, UserFacingError> {
    let mut runtime = RuntimeContext::load(root_layout)?;
    let workflow = select_manual_run_workflow(&runtime.definitions.workflows)?;
    let mut request = NormalizedRunRequest::new(
        format!(
            "manual-{}-{}-{}-{}",
            sanitize_path_component(&workflow.workflow_id),
            current_time_ms()?,
            std::process::id(),
            MANUAL_RUN_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ),
        workflow.workflow_id.clone(),
    );
    request
        .manual_invocation_input
        .insert("manual_invocation".to_owned(), serde_json::json!(true));
    execute_single_run(&mut runtime, request, current_time_ms()?)
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

fn execute_single_run(
    runtime: &mut RuntimeContext,
    request: NormalizedRunRequest,
    started_at_ms: i64,
) -> Result<SingleRunResult, UserFacingError> {
    execute_single_run_with_fence(runtime, request, started_at_ms, None)
}

fn execute_single_run_with_fence(
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

pub(super) fn execute_single_run_with_fence_and_cancellation(
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
                        .iter()
                        .map(|(node_id, detail)| format!("{node_id}={detail}"))
                        .collect::<Vec<_>>()
                        .join("; ")
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
        parse_plugin_activation_bindings(&runtime.definitions.root_config)
            .map_err(UserFacingError::from_contract)?,
        runtime.root_layout.plugins_dir.clone(),
        runtime.root_layout.secrets_dir.clone(),
        runtime.secret_mode,
    )
    .map(|plane| plane.with_cancellation(cancellation.clone()))
    .map_err(UserFacingError::from_contract)
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

fn write_log_entry(
    state_store: &mut RuntimeStateStore,
    run_id: &str,
    event: &str,
    message: &str,
    occurred_at_ms: i64,
) -> Result<u64, RuntimeStateError> {
    state_store.append_workflow_log_entry(run_id, event, message, occurred_at_ms)
}

pub(crate) fn render_run_status(status: RunStatus) -> &'static str {
    match status {
        RunStatus::Pending => "pending",
        RunStatus::Running => "running",
        RunStatus::Succeeded => "succeeded",
        RunStatus::Failed => "failed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn manual_run_rejects_missing_or_ambiguous_workflows() {
        assert!(matches!(
            select_manual_run_workflow(&[]),
            Err(UserFacingError::Validation { .. })
        ));
        let workflow = WorkflowDefinition {
            api_version: String::from("2.0.0"),
            workflow_id: String::from("first"),
            name: String::from("first"),
            runtime: Default::default(),
            nodes: Vec::new(),
            package_root: PathBuf::new(),
        };
        let mut workflows = vec![workflow.clone()];
        assert_eq!(select_manual_run_workflow(&workflows).unwrap().workflow_id, "first");
        workflows.push(WorkflowDefinition {
            workflow_id: String::from("second"),
            ..workflow
        });
        assert!(matches!(
            select_manual_run_workflow(&workflows),
            Err(UserFacingError::Usage { .. })
        ));
    }
}
