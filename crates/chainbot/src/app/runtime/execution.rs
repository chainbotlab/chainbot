//! [INPUT]
//! Validated workflow definitions, normalized run requests, builtin registry dispatch, plugin manifests, and secret runtime inputs.
//!
//! [OUTPUT]
//! Executes scheduler waves and node orchestration for builtin/plugin/subflow nodes with deterministic run reports.
//!
//! [ROLE]
//! Owns app-layer workflow execution orchestration over domain runtime contracts.

// Execution Pipeline
//
// NormalizedRunRequest
//     |
//     v
// +----------------------------------------------------------------+
// |  execute_internal  — depth guard, cycle detection, stack push  |
// +----------------------------------------------------------------+
//     |
//     v
// +----------------------------------------------------------------+
// |  execute_workflow_frame  — DAG evaluation, wave scheduling     |
// |    - evaluate_dependency_decision per node (Waiting/Ready/Skip)|
// |    - schedule_ready_nodes in waves                             |
// +----------------------------------------------------------------+
//     |
//     v
// +----------------------------------------------------------------+
// |  execute_node  — input resolution, kind dispatch               |
// |    - builtin: builtin_registry.dispatch (kind -> BuiltinNodeRequest)
// |    - plugin:   ExternalNodePluginHost::execute_node_invocation
// |    - subflow:  recursive execute_internal call (depth + 1)
// +----------------------------------------------------------------+
//     |
//     v
// WorkflowRunReport { run_id, workflow_id, status, node_states,
//                     node_outputs, node_failures, runtime_namespaces, schedule_waves }

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use serde_json::Value;

use crate::builtins::nodes::input_resolver::resolve_node_inputs;
use crate::builtins::nodes::SecretDecryptMode;
use crate::builtins::nodes::contract::{BuiltinNodeRequest, BuiltinNodeResult};
use crate::builtins::nodes::dispatch::builtin_dispatch_kind;
pub use crate::builtins::nodes::registry_store::BuiltinNodeRegistry;
use crate::domain::runtime::{
    NodeDefinition, NormalizedRunRequest, ScheduledNodeState, WorkflowRunReport,
    WorkflowRunStatus,
};
use crate::domain::workflow::{DependsMode, RuntimeVariableLayers, RuntimeVariableNamespaces, WorkflowDefinition};
use crate::errors::ContractError;
use crate::plugin::{
    ExternalNodePluginHost, ExternalNodePluginRequest, PluginActivationEnvelope,
    PluginHostSecretMode, PluginKind, PluginManifest, NODE_PLUGIN_CONTRACT_VERSION,
    NODE_PLUGIN_EXECUTE_CAPABILITY,
};
use crate::secrets::{
    redact_text, GpgSecretDecryptor, PlaintextSecretDecryptor, SecretProvider, SecretReference,
    SecretValue,
};

pub const DEFAULT_MAX_SUBFLOW_DEPTH: usize = 32;

pub struct ExecutionPlane {
    workflows: BTreeMap<String, WorkflowDefinition>,
    config_defaults: BTreeMap<String, serde_json::Value>,
    builtin_registry: BuiltinNodeRegistry,
    plugin_manifests: BTreeMap<String, PluginManifest>,
    plugin_activation: BTreeMap<String, BTreeMap<String, SecretReference>>,
    plugins_root: PathBuf,
    secrets_dir: PathBuf,
    secret_mode: SecretDecryptMode,
    max_subflow_depth: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DependencyDecision {
    Waiting,
    Ready,
    Skip,
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
            Vec::new(),
            BTreeMap::new(),
            PathBuf::new(),
            PathBuf::new(),
            SecretDecryptMode::Plaintext,
            DEFAULT_MAX_SUBFLOW_DEPTH,
        )
    }

    pub fn with_plugin_runtime(
        workflows: Vec<WorkflowDefinition>,
        config_defaults: BTreeMap<String, serde_json::Value>,
        builtin_registry: BuiltinNodeRegistry,
        plugin_manifests: Vec<PluginManifest>,
        plugin_activation: BTreeMap<String, BTreeMap<String, SecretReference>>,
        plugins_root: PathBuf,
        secrets_dir: PathBuf,
        secret_mode: SecretDecryptMode,
    ) -> Result<Self, ContractError> {
        Self::with_max_subflow_depth(
            workflows,
            config_defaults,
            builtin_registry,
            plugin_manifests,
            plugin_activation,
            plugins_root,
            secrets_dir,
            secret_mode,
            DEFAULT_MAX_SUBFLOW_DEPTH,
        )
    }

    pub fn with_max_subflow_depth(
        workflows: Vec<WorkflowDefinition>,
        config_defaults: BTreeMap<String, serde_json::Value>,
        builtin_registry: BuiltinNodeRegistry,
        plugin_manifests: Vec<PluginManifest>,
        plugin_activation: BTreeMap<String, BTreeMap<String, SecretReference>>,
        plugins_root: PathBuf,
        secrets_dir: PathBuf,
        secret_mode: SecretDecryptMode,
        max_subflow_depth: usize,
    ) -> Result<Self, ContractError> {
        let mut workflow_map = BTreeMap::new();
        for workflow in workflows {
            workflow.validate()?;
            let workflow_id = workflow.workflow_id.clone();
            if workflow_map.contains_key(&workflow_id) {
                return Err(ContractError::DuplicateWorkflowId { workflow_id });
            }
            workflow_map.insert(workflow_id, workflow);
        }

        let mut plugin_manifest_map = BTreeMap::new();
        for manifest in plugin_manifests {
            manifest.validate()?;
            plugin_manifest_map.insert(manifest.plugin_id.clone(), manifest);
        }

        Ok(Self {
            workflows: workflow_map,
            config_defaults,
            builtin_registry,
            plugin_manifests: plugin_manifest_map,
            plugin_activation,
            plugins_root,
            secrets_dir,
            secret_mode,
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
        let mut runtime_namespaces =
            runtime_layers.resolve_namespaces(request.subflow_input.clone());

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
                    || matches!(
                        current_state,
                        ScheduledNodeState::Ready | ScheduledNodeState::Running
                    )
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

        if node.kind == "plugin" {
            return self.execute_plugin_node(request, workflow, node, inputs);
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

    fn execute_plugin_node(
        &self,
        _request: &NormalizedRunRequest,
        workflow: &WorkflowDefinition,
        node: &NodeDefinition,
        inputs: BTreeMap<String, serde_json::Value>,
    ) -> Result<BuiltinNodeResult, ContractError> {
        let manifest = self.plugin_manifests.get(&node.plugin_id).ok_or_else(|| {
            ContractError::CliUsage {
                message: format!(
                    "workflow {} node {} references unknown plugin {}",
                    workflow.workflow_id, node.node_id, node.plugin_id
                ),
            }
        })?;

        if manifest.kind()? != PluginKind::ExternalNode {
            return Err(ContractError::CliUsage {
                message: format!(
                    "workflow {} node {} expected external node plugin kind for {}",
                    workflow.workflow_id, node.node_id, node.plugin_id
                ),
            });
        }

        let mut resolved = resolve_node_inputs(&self.secrets_dir, self.secret_mode, &inputs)?;
        let operation = manifest.node_operation(&node.operation)?;
        if let Some(default_confirmation) = operation.default_confirmation.as_deref()
            && !resolved.values.contains_key("confirmation_mode")
        {
            resolved.values.insert(
                String::from("confirmation_mode"),
                Value::String(default_confirmation.to_owned()),
            );
        }
        let secret_mode = match self.secret_mode {
            SecretDecryptMode::Gpg => PluginHostSecretMode::Gpg,
            SecretDecryptMode::Plaintext => PluginHostSecretMode::Plaintext,
        };
        let (activation, activation_secrets) = self.resolve_plugin_activation(&node.plugin_id)?;
        let host = ExternalNodePluginHost::with_secret_runtime(
            self.plugins_root.clone(),
            self.secrets_dir.clone(),
            secret_mode,
        );
        let response = host.execute_node_invocation(
            manifest,
            &ExternalNodePluginRequest {
                contract_version: NODE_PLUGIN_CONTRACT_VERSION.to_owned(),
                plugin_id: node.plugin_id.clone(),
                node_id: node.node_id.clone(),
                operation: node.operation.clone(),
                requested_capabilities: vec![NODE_PLUGIN_EXECUTE_CAPABILITY.to_owned()],
                input: resolved.values,
                activation,
            },
        )
        .map_err(|error| redact_plugin_contract_error(error, &activation_secrets))?;

        Ok(BuiltinNodeResult {
            outputs: response.output.clone(),
            run_scoped: response.output,
            ..BuiltinNodeResult::default()
        })
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

impl ExecutionPlane {
    fn resolve_plugin_activation(
        &self,
        plugin_id: &str,
    ) -> Result<(Option<PluginActivationEnvelope>, Vec<SecretValue>), ContractError> {
        let Some(bindings) = self.plugin_activation.get(plugin_id) else {
            return Ok((None, Vec::new()));
        };
        if bindings.is_empty() {
            return Ok((None, Vec::new()));
        }

        let mut resolved = BTreeMap::new();
        let mut secrets = Vec::with_capacity(bindings.len());
        match self.secret_mode {
            SecretDecryptMode::Gpg => {
                let provider = SecretProvider::new(self.secrets_dir.clone(), GpgSecretDecryptor::new());
                for (slot, reference) in bindings {
                    let value = provider.resolve_reference(reference)?;
                    resolved.insert(slot.clone(), value.expose().to_owned());
                    secrets.push(value);
                }
            }
            SecretDecryptMode::Plaintext => {
                let provider = SecretProvider::new(self.secrets_dir.clone(), PlaintextSecretDecryptor);
                for (slot, reference) in bindings {
                    let value = provider.resolve_reference(reference)?;
                    resolved.insert(slot.clone(), value.expose().to_owned());
                    secrets.push(value);
                }
            }
        }

        Ok((Some(PluginActivationEnvelope { secrets: resolved }), secrets))
    }
}

fn redact_plugin_contract_error(error: ContractError, secrets: &[SecretValue]) -> ContractError {
    match error {
        ContractError::NodePluginProcessFailed {
            plugin_id,
            exit_code,
            stderr,
        } => ContractError::NodePluginProcessFailed {
            plugin_id,
            exit_code,
            stderr: redact_text(&stderr, secrets),
        },
        ContractError::NodePluginReturnedFailure { plugin_id, message } => {
            ContractError::NodePluginReturnedFailure {
                plugin_id,
                message: redact_text(&message, secrets),
            }
        }
        ContractError::NodePluginProtocolContractViolation { plugin_id, detail } => {
            ContractError::NodePluginProtocolContractViolation {
                plugin_id,
                detail: redact_text(&detail, secrets),
            }
        }
        other => other,
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
