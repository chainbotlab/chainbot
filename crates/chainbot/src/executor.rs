//! [INPUT]
//! Workflow definitions, normalized run requests, state persistence, secret resolution, plugin hosts, worker hosts, and builtin node handlers.
//!
//! [OUTPUT]
//! Plans node execution waves, dispatches builtin and external work, and returns structured run results with scheduler state transitions.
//!
//! [ROLE]
//! Keeps orchestration authority in Rust for the workflow execution plane.


use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

pub use crate::builtins::nodes::contract::{
    BuiltinNodeRegistry, BuiltinNodeRequest, BuiltinNodeResult,
};
use crate::builtins::nodes::dispatch::builtin_dispatch_kind;
use crate::errors::{assert_supported_major, ContractError};
use crate::workflow::{
    DependsMode, RuntimeVariableLayers, RuntimeVariableNamespaces, SubflowContract, VariableBinding,
    WhenCondition, WorkflowDefinition,
};

pub const CURRENT_API_MAJOR: u64 = 2;
pub const DEFAULT_MAX_SUBFLOW_DEPTH: usize = 32;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeDefinition {
    #[serde(rename = "manifest_version", alias = "api_version")]
    pub api_version: String,
    #[serde(rename = "id", alias = "node_id")]
    pub node_id: String,
    pub kind: String,
    #[serde(rename = "plugin", alias = "plugin_id")]
    pub plugin_id: String,
    pub operation: String,
    #[serde(default)]
    pub depends_mode: DependsMode,
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub inputs: Vec<VariableBinding>,
    #[serde(default)]
    pub when: Option<WhenCondition>,
    #[serde(default)]
    pub subflow: Option<SubflowContract>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NormalizedRunRequest {
    pub run_id: String,
    pub workflow_id: String,
    pub cli_args: BTreeMap<String, serde_json::Value>,
    pub manual_invocation_input: BTreeMap<String, serde_json::Value>,
    pub trigger_payload_mapping: BTreeMap<String, serde_json::Value>,
    pub subflow_input: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowRunStatus {
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduledNodeState {
    Pending,
    Blocked,
    Ready,
    Running,
    Succeeded,
    Skipped,
    Failed,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowRunReport {
    pub run_id: String,
    pub workflow_id: String,
    pub status: WorkflowRunStatus,
    pub node_states: BTreeMap<String, ScheduledNodeState>,
    pub node_outputs: BTreeMap<String, BTreeMap<String, serde_json::Value>>,
    pub node_failures: BTreeMap<String, String>,
    pub runtime_namespaces: RuntimeVariableNamespaces,
    pub schedule_waves: Vec<Vec<String>>,
}

pub struct ExecutionPlane {
    workflows: BTreeMap<String, WorkflowDefinition>,
    config_defaults: BTreeMap<String, serde_json::Value>,
    builtin_registry: BuiltinNodeRegistry,
    max_subflow_depth: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DependencyDecision {
    Waiting,
    Ready,
    Skip,
}

impl NodeDefinition {
    pub fn validate(&self) -> Result<(), ContractError> {
        assert_supported_major(
            "node.manifest_version",
            &self.api_version,
            CURRENT_API_MAJOR,
        )?;

        for input in &self.inputs {
            if !input.validate() {
                return Err(ContractError::InvalidVariableReference {
                    workflow_id: "<unknown>".to_owned(),
                    node_id: self.node_id.clone(),
                    context: "node.inputs",
                    namespace: input.source.namespace.as_str().to_owned(),
                    key: input.source.key.clone(),
                });
            }
        }

        if let Some(when) = &self.when
            && !when.validate()
        {
            return Err(ContractError::InvalidVariableReference {
                workflow_id: "<unknown>".to_owned(),
                node_id: self.node_id.clone(),
                context: "node.when",
                namespace: when.source.namespace.as_str().to_owned(),
                key: when.source.key.clone(),
            });
        }

        Ok(())
    }
}

impl NormalizedRunRequest {
    pub fn new(run_id: impl Into<String>, workflow_id: impl Into<String>) -> Self {
        Self {
            run_id: run_id.into(),
            workflow_id: workflow_id.into(),
            cli_args: BTreeMap::new(),
            manual_invocation_input: BTreeMap::new(),
            trigger_payload_mapping: BTreeMap::new(),
            subflow_input: BTreeMap::new(),
        }
    }
}

impl ScheduledNodeState {
    fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Skipped | Self::Failed
        )
    }
}

impl ExecutionPlane {
    pub fn new(
        workflows: Vec<WorkflowDefinition>,
        config_defaults: BTreeMap<String, serde_json::Value>,
        builtin_registry: BuiltinNodeRegistry,
    ) -> Result<Self, ContractError> {
        Self::with_max_subflow_depth(
            workflows,
            config_defaults,
            builtin_registry,
            DEFAULT_MAX_SUBFLOW_DEPTH,
        )
    }

    pub fn with_max_subflow_depth(
        workflows: Vec<WorkflowDefinition>,
        config_defaults: BTreeMap<String, serde_json::Value>,
        builtin_registry: BuiltinNodeRegistry,
        max_subflow_depth: usize,
    ) -> Result<Self, ContractError> {
        let mut workflow_map = BTreeMap::new();
        for workflow in workflows {
            workflow.validate()?;
            let workflow_id = workflow.workflow_id.clone();
            if workflow_map.contains_key(&workflow_id) {
                return Err(ContractError::DuplicateWorkflowId {
                    workflow_id,
                });
            }
            workflow_map.insert(workflow_id, workflow);
        }

        Ok(Self {
            workflows: workflow_map,
            config_defaults,
            builtin_registry,
            max_subflow_depth,
        })
    }

    pub fn execute(
        &self,
        request: &NormalizedRunRequest,
    ) -> Result<WorkflowRunReport, ContractError> {
        self.execute_internal(request, &mut Vec::new(), 0)
    }

    fn execute_internal(
        &self,
        request: &NormalizedRunRequest,
        active_stack: &mut Vec<String>,
        depth: usize,
    ) -> Result<WorkflowRunReport, ContractError> {
        if depth > self.max_subflow_depth {
            return Err(ContractError::SubflowDepthExceeded {
                workflow_id: request.workflow_id.clone(),
                max_depth: self.max_subflow_depth,
            });
        }

        if active_stack.contains(&request.workflow_id) {
            let mut workflow_chain = active_stack.clone();
            workflow_chain.push(request.workflow_id.clone());
            return Err(ContractError::SubflowCycleDetected { workflow_chain });
        }

        active_stack.push(request.workflow_id.clone());
        let result = self.execute_workflow_frame(request, active_stack, depth);
        active_stack.pop();
        result
    }

    fn execute_workflow_frame(
        &self,
        request: &NormalizedRunRequest,
        active_stack: &mut Vec<String>,
        depth: usize,
    ) -> Result<WorkflowRunReport, ContractError> {
        let workflow = self
            .workflows
            .get(&request.workflow_id)
            .ok_or_else(|| ContractError::UnknownWorkflowDefinition {
                workflow_id: request.workflow_id.clone(),
            })?
            .clone();

        workflow.validate()?;

        let runtime_layers = RuntimeVariableLayers {
            cli_args: request.cli_args.clone(),
            manual_invocation_input: request.manual_invocation_input.clone(),
            trigger_payload_mapping: request.trigger_payload_mapping.clone(),
            workflow_defaults: workflow.runtime.workflow_defaults.clone(),
            config_defaults: self.config_defaults.clone(),
        };
        let mut runtime_namespaces = runtime_layers.resolve_namespaces(request.subflow_input.clone());

        let mut node_states = BTreeMap::new();
        let mut node_index = BTreeMap::new();
        for node in &workflow.nodes {
            node_states.insert(node.node_id.clone(), ScheduledNodeState::Pending);
            node_index.insert(node.node_id.clone(), node.clone());
        }

        let mut node_outputs = BTreeMap::new();
        let mut node_failures = BTreeMap::new();
        let mut schedule_waves = Vec::new();

        loop {
            if node_states.values().copied().all(ScheduledNodeState::is_terminal) {
                break;
            }

            let mut ready_nodes = Vec::new();
            let mut state_changed = false;

            for (node_id, node) in &node_index {
                let current_state = node_states
                    .get(node_id)
                    .copied()
                    .unwrap_or(ScheduledNodeState::Pending);
                if current_state.is_terminal()
                    || matches!(current_state, ScheduledNodeState::Ready | ScheduledNodeState::Running)
                {
                    continue;
                }

                match evaluate_dependency_decision(node, &node_states) {
                    DependencyDecision::Waiting => {
                        if current_state != ScheduledNodeState::Blocked {
                            node_states.insert(node_id.clone(), ScheduledNodeState::Blocked);
                            state_changed = true;
                        }
                    }
                    DependencyDecision::Skip => {
                        node_states.insert(node_id.clone(), ScheduledNodeState::Skipped);
                        state_changed = true;
                    }
                    DependencyDecision::Ready => {
                        if let Some(when) = &node.when
                            && !when.evaluate(&runtime_namespaces)
                        {
                            node_states.insert(node_id.clone(), ScheduledNodeState::Skipped);
                            state_changed = true;
                            continue;
                        }

                        node_states.insert(node_id.clone(), ScheduledNodeState::Ready);
                        ready_nodes.push(node_id.clone());
                        state_changed = true;
                    }
                }
            }

            if ready_nodes.is_empty() {
                if node_states.values().copied().all(ScheduledNodeState::is_terminal) {
                    break;
                }

                if !state_changed {
                    let blocked_node_ids = node_states
                        .iter()
                        .filter_map(|(node_id, state)| {
                            if state.is_terminal() {
                                None
                            } else {
                                Some(node_id.clone())
                            }
                        })
                        .collect::<Vec<String>>();
                    return Err(ContractError::SchedulerStalled {
                        workflow_id: workflow.workflow_id,
                        blocked_node_ids,
                    });
                }

                continue;
            }

            schedule_waves.push(ready_nodes.clone());

            for node_id in ready_nodes {
                node_states.insert(node_id.clone(), ScheduledNodeState::Running);
                let Some(node) = node_index.get(&node_id) else {
                    return Err(ContractError::MissingDagNodeIndex {
                        workflow_id: workflow.workflow_id.clone(),
                        node_id,
                    });
                };

                match self.execute_node(
                    request,
                    &workflow,
                    node,
                    &runtime_namespaces,
                    active_stack,
                    depth,
                ) {
                    Ok(result) => {
                        node_states.insert(node.node_id.clone(), ScheduledNodeState::Succeeded);
                        node_outputs.insert(node.node_id.clone(), result.outputs.clone());
                        for (key, value) in result.outputs {
                            runtime_namespaces.node_outputs.insert(key, value);
                        }
                        for (key, value) in result.run_scoped {
                            runtime_namespaces.run_scoped.insert(key, value);
                        }
                        for (key, value) in result.subflow_output {
                            runtime_namespaces.subflow_output.insert(key, value);
                        }
                    }
                    Err(error) => {
                        node_states.insert(node.node_id.clone(), ScheduledNodeState::Failed);
                        node_failures.insert(node.node_id.clone(), error.to_string());
                    }
                }
            }
        }

        let status = if node_states
            .values()
            .any(|state| *state == ScheduledNodeState::Failed)
        {
            WorkflowRunStatus::Failed
        } else {
            WorkflowRunStatus::Succeeded
        };

        Ok(WorkflowRunReport {
            run_id: request.run_id.clone(),
            workflow_id: request.workflow_id.clone(),
            status,
            node_states,
            node_outputs,
            node_failures,
            runtime_namespaces,
            schedule_waves,
        })
    }

    fn execute_node(
        &self,
        request: &NormalizedRunRequest,
        workflow: &WorkflowDefinition,
        node: &NodeDefinition,
        runtime_namespaces: &RuntimeVariableNamespaces,
        active_stack: &mut Vec<String>,
        depth: usize,
    ) -> Result<BuiltinNodeResult, ContractError> {
        let mut inputs = BTreeMap::new();
        for binding in &node.inputs {
            if let Some(value) = runtime_namespaces.resolve(&binding.source) {
                inputs.insert(binding.target.clone(), value.clone());
            }
        }

        if node.kind == "subflow" {
            return self.execute_subflow_node(
                request,
                workflow,
                node,
                runtime_namespaces,
                active_stack,
                depth + 1,
            );
        }

        let Some(kind) = builtin_dispatch_kind(node) else {
            return Err(ContractError::UnsupportedNodeKindForScheduler {
                workflow_id: workflow.workflow_id.clone(),
                node_id: node.node_id.clone(),
                kind: node.kind.clone(),
            });
        };

        let dispatch_request = BuiltinNodeRequest {
            run_id: request.run_id.clone(),
            workflow_id: workflow.workflow_id.clone(),
            workflow_package_root: workflow.package_root.clone(),
            node_id: node.node_id.clone(),
            operation: node.operation.clone(),
            inputs,
            runtime_namespaces: runtime_namespaces.clone(),
        };
        self.builtin_registry.dispatch(kind, &dispatch_request)
    }

    fn execute_subflow_node(
        &self,
        request: &NormalizedRunRequest,
        workflow: &WorkflowDefinition,
        node: &NodeDefinition,
        runtime_namespaces: &RuntimeVariableNamespaces,
        active_stack: &mut Vec<String>,
        depth: usize,
    ) -> Result<BuiltinNodeResult, ContractError> {
        let Some(contract) = node.subflow.as_ref() else {
            return Err(ContractError::MissingSubflowContract {
                workflow_id: workflow.workflow_id.clone(),
                node_id: node.node_id.clone(),
            });
        };

        let child_request = NormalizedRunRequest {
            run_id: format!("{}::{}", request.run_id, node.node_id),
            workflow_id: contract.workflow_id.clone(),
            cli_args: BTreeMap::new(),
            manual_invocation_input: BTreeMap::new(),
            trigger_payload_mapping: BTreeMap::new(),
            subflow_input: contract.build_child_inputs(runtime_namespaces),
        };

        let child_report = self.execute_internal(&child_request, active_stack, depth)?;
        if child_report.status == WorkflowRunStatus::Failed {
            return Err(ContractError::SubflowExecutionFailed {
                workflow_id: workflow.workflow_id.clone(),
                node_id: node.node_id.clone(),
                child_workflow_id: contract.workflow_id.clone(),
            });
        }

        Ok(BuiltinNodeResult {
            subflow_output: contract.collect_exports(&child_report.runtime_namespaces.subflow_output),
            ..BuiltinNodeResult::default()
        })
    }
}

fn evaluate_dependency_decision(
    node: &NodeDefinition,
    node_states: &BTreeMap<String, ScheduledNodeState>,
) -> DependencyDecision {
    let dependencies = normalized_dependencies(&node.depends_on);
    if dependencies.is_empty() {
        return DependencyDecision::Ready;
    }

    let mut succeeded = 0usize;
    let mut all_terminal = true;

    for dependency_id in &dependencies {
        let state = node_states
            .get(dependency_id)
            .copied()
            .unwrap_or(ScheduledNodeState::Pending);
        if state == ScheduledNodeState::Succeeded {
            succeeded += 1;
        }
        if !state.is_terminal() {
            all_terminal = false;
        }
    }

    if !all_terminal {
        return DependencyDecision::Waiting;
    }

    match node.depends_mode {
        DependsMode::All => {
            if succeeded == dependencies.len() {
                DependencyDecision::Ready
            } else {
                DependencyDecision::Skip
            }
        }
        DependsMode::Any => {
            if succeeded > 0 {
                DependencyDecision::Ready
            } else {
                DependencyDecision::Skip
            }
        }
    }
}

fn normalized_dependencies(depends_on: &[String]) -> Vec<String> {
    let mut dependencies = BTreeSet::new();
    for dependency in depends_on {
        dependencies.insert(dependency.clone());
    }
    dependencies.into_iter().collect()
}
