/*
[INPUT]:  Workflow package manifests, node dependency declarations, and namespace/subflow mappings.
[OUTPUT]: Typed DAG validation, namespace-safe runtime variable contracts, deterministic precedence resolution, and when-condition evaluation helpers.
[POS]:    Workflow semantic boundary that freezes package-based graph and variable contracts while exposing scheduler-safe evaluators.
[UPDATE]: 2026-03-16 - Add versioned workflow definition contract.
[UPDATE]: 2026-03-16 - Add DAG validation, typed depends_mode/when, runtime variable namespaces, and subflow import/export contracts.
[UPDATE]: 2026-03-16 - Add deterministic when evaluation and runtime namespace materialization helpers for execution-plane scheduling.
[UPDATE]: 2026-03-18 - Parse v2.1 workflow package manifests with nested headers and package-root-relative resources.
*/

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use petgraph::algo::tarjan_scc;
use petgraph::graph::DiGraph;
use serde::{Deserialize, Deserializer, Serialize};

use crate::errors::{assert_supported_major, ContractError};
use crate::executor::NodeDefinition;
pub const CURRENT_API_MAJOR: u64 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DependsMode {
    #[default]
    All,
    Any,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeVariableNamespace {
    CliArgs,
    ManualInvocationInput,
    TriggerPayloadMapping,
    WorkflowDefaults,
    ConfigDefaults,
    NodeOutputs,
    RunScoped,
    SubflowInput,
    SubflowOutput,
}

impl RuntimeVariableNamespace {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::CliArgs => "cli_args",
            Self::ManualInvocationInput => "manual_invocation_input",
            Self::TriggerPayloadMapping => "trigger_payload_mapping",
            Self::WorkflowDefaults => "workflow_defaults",
            Self::ConfigDefaults => "config_defaults",
            Self::NodeOutputs => "node_outputs",
            Self::RunScoped => "run_scoped",
            Self::SubflowInput => "subflow_input",
            Self::SubflowOutput => "subflow_output",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VariableReference {
    pub namespace: RuntimeVariableNamespace,
    pub key: String,
}

impl VariableReference {
    pub fn validate(&self) -> bool {
        !self.key.trim().is_empty()
    }

    pub fn as_string(&self) -> String {
        format!("{}:{}", self.namespace.as_str(), self.key)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VariableBinding {
    pub target: String,
    pub source: VariableReference,
}

impl VariableBinding {
    pub fn validate(&self) -> bool {
        !self.target.trim().is_empty() && self.source.validate()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WhenOperator {
    Exists,
    Equals,
    NotEquals,
    #[default]
    Truthy,
    Falsy,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WhenCondition {
    pub source: VariableReference,
    #[serde(default)]
    pub operator: WhenOperator,
    #[serde(default)]
    pub expected: Option<serde_json::Value>,
}

impl WhenCondition {
    pub fn validate(&self) -> bool {
        if !self.source.validate() {
            return false;
        }

        match self.operator {
            WhenOperator::Equals | WhenOperator::NotEquals => self.expected.is_some(),
            WhenOperator::Exists | WhenOperator::Truthy | WhenOperator::Falsy => {
                self.expected.is_none()
            }
        }
    }

    pub fn evaluate(&self, namespaces: &RuntimeVariableNamespaces) -> bool {
        let value = namespaces.resolve(&self.source);
        match self.operator {
            WhenOperator::Exists => value.is_some(),
            WhenOperator::Equals => value.zip(self.expected.as_ref()).is_some_and(|(left, right)| left == right),
            WhenOperator::NotEquals => {
                value.zip(self.expected.as_ref()).is_some_and(|(left, right)| left != right)
            }
            WhenOperator::Truthy => value.is_some_and(is_truthy),
            WhenOperator::Falsy => value.is_none_or(|candidate| !is_truthy(candidate)),
        }
    }
}

fn is_truthy(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Null => false,
        serde_json::Value::Bool(flag) => *flag,
        serde_json::Value::Number(number) => {
            if let Some(integer) = number.as_i64() {
                integer != 0
            } else if let Some(unsigned) = number.as_u64() {
                unsigned != 0
            } else {
                number
                    .as_f64()
                    .is_some_and(|float| float != 0.0 && !float.is_nan())
            }
        }
        serde_json::Value::String(text) => !text.is_empty(),
        serde_json::Value::Array(entries) => !entries.is_empty(),
        serde_json::Value::Object(entries) => !entries.is_empty(),
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubflowImport {
    pub child_key: String,
    pub source: VariableReference,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubflowExport {
    pub child_key: String,
    pub parent_key: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubflowContract {
    pub workflow_id: String,
    #[serde(default)]
    pub imports: Vec<SubflowImport>,
    #[serde(default)]
    pub exports: Vec<SubflowExport>,
}

impl SubflowContract {
    pub fn validate(
        &self,
        workflow_id: &str,
        node_id: &str,
        is_subflow_node: bool,
    ) -> Result<(), ContractError> {
        if !is_subflow_node {
            return Err(ContractError::UnexpectedSubflowContract {
                workflow_id: workflow_id.to_owned(),
                node_id: node_id.to_owned(),
            });
        }

        if self.workflow_id.trim().is_empty() {
            return Err(ContractError::InvalidSubflowContract {
                workflow_id: workflow_id.to_owned(),
                node_id: node_id.to_owned(),
                detail: "subflow workflow_id cannot be empty".to_owned(),
            });
        }

        let mut child_import_keys = BTreeSet::new();
        for import in &self.imports {
            if import.child_key.trim().is_empty() {
                return Err(ContractError::InvalidSubflowContract {
                    workflow_id: workflow_id.to_owned(),
                    node_id: node_id.to_owned(),
                    detail: "subflow import child_key cannot be empty".to_owned(),
                });
            }

            if !import.source.validate() {
                return Err(ContractError::InvalidVariableReference {
                    workflow_id: workflow_id.to_owned(),
                    node_id: node_id.to_owned(),
                    context: "subflow.imports",
                    namespace: import.source.namespace.as_str().to_owned(),
                    key: import.source.key.clone(),
                });
            }

            match import.source.namespace {
                RuntimeVariableNamespace::SubflowInput | RuntimeVariableNamespace::SubflowOutput => {
                    return Err(ContractError::InvalidVariableReference {
                        workflow_id: workflow_id.to_owned(),
                        node_id: node_id.to_owned(),
                        context: "subflow.imports",
                        namespace: import.source.namespace.as_str().to_owned(),
                        key: import.source.key.clone(),
                    });
                }
                RuntimeVariableNamespace::CliArgs
                | RuntimeVariableNamespace::ManualInvocationInput
                | RuntimeVariableNamespace::TriggerPayloadMapping
                | RuntimeVariableNamespace::WorkflowDefaults
                | RuntimeVariableNamespace::ConfigDefaults
                | RuntimeVariableNamespace::NodeOutputs
                | RuntimeVariableNamespace::RunScoped => {}
            }

            if !child_import_keys.insert(import.child_key.clone()) {
                return Err(ContractError::InvalidSubflowContract {
                    workflow_id: workflow_id.to_owned(),
                    node_id: node_id.to_owned(),
                    detail: format!(
                        "subflow import child_key {} is duplicated",
                        import.child_key
                    ),
                });
            }
        }

        let mut parent_export_keys = BTreeSet::new();
        for export in &self.exports {
            if export.child_key.trim().is_empty() || export.parent_key.trim().is_empty() {
                return Err(ContractError::InvalidSubflowContract {
                    workflow_id: workflow_id.to_owned(),
                    node_id: node_id.to_owned(),
                    detail: "subflow exports cannot contain empty child_key or parent_key"
                        .to_owned(),
                });
            }

            if !parent_export_keys.insert(export.parent_key.clone()) {
                return Err(ContractError::InvalidSubflowContract {
                    workflow_id: workflow_id.to_owned(),
                    node_id: node_id.to_owned(),
                    detail: format!(
                        "subflow export parent_key {} is duplicated",
                        export.parent_key
                    ),
                });
            }
        }

        Ok(())
    }

    pub fn build_child_inputs(
        &self,
        namespaces: &RuntimeVariableNamespaces,
    ) -> BTreeMap<String, serde_json::Value> {
        let mut child_inputs = BTreeMap::new();
        for import in &self.imports {
            if let Some(value) = namespaces.resolve(&import.source) {
                child_inputs.insert(import.child_key.clone(), value.clone());
            }
        }
        child_inputs
    }

    pub fn collect_exports(
        &self,
        child_outputs: &BTreeMap<String, serde_json::Value>,
    ) -> BTreeMap<String, serde_json::Value> {
        let mut exports = BTreeMap::new();
        for export in &self.exports {
            if let Some(value) = child_outputs.get(&export.child_key) {
                exports.insert(export.parent_key.clone(), value.clone());
            }
        }
        exports
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeVariableNamespaces {
    #[serde(default)]
    pub cli_args: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub manual_invocation_input: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub trigger_payload_mapping: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub workflow_defaults: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub config_defaults: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub node_outputs: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub run_scoped: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub subflow_input: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub subflow_output: BTreeMap<String, serde_json::Value>,
}

impl RuntimeVariableNamespaces {
    pub fn resolve(&self, reference: &VariableReference) -> Option<&serde_json::Value> {
        match reference.namespace {
            RuntimeVariableNamespace::CliArgs => self.cli_args.get(&reference.key),
            RuntimeVariableNamespace::ManualInvocationInput => {
                self.manual_invocation_input.get(&reference.key)
            }
            RuntimeVariableNamespace::TriggerPayloadMapping => {
                self.trigger_payload_mapping.get(&reference.key)
            }
            RuntimeVariableNamespace::WorkflowDefaults => self.workflow_defaults.get(&reference.key),
            RuntimeVariableNamespace::ConfigDefaults => self.config_defaults.get(&reference.key),
            RuntimeVariableNamespace::NodeOutputs => self.node_outputs.get(&reference.key),
            RuntimeVariableNamespace::RunScoped => self.run_scoped.get(&reference.key),
            RuntimeVariableNamespace::SubflowInput => self.subflow_input.get(&reference.key),
            RuntimeVariableNamespace::SubflowOutput => self.subflow_output.get(&reference.key),
        }
    }
}

impl Default for RuntimeVariableNamespaces {
    fn default() -> Self {
        Self {
            cli_args: BTreeMap::new(),
            manual_invocation_input: BTreeMap::new(),
            trigger_payload_mapping: BTreeMap::new(),
            workflow_defaults: BTreeMap::new(),
            config_defaults: BTreeMap::new(),
            node_outputs: BTreeMap::new(),
            run_scoped: BTreeMap::new(),
            subflow_input: BTreeMap::new(),
            subflow_output: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RuntimeVariableSource {
    CliArgs,
    ManualInvocationInput,
    TriggerPayloadMapping,
    WorkflowDefaults,
    ConfigDefaults,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedRuntimeVariable {
    pub value: serde_json::Value,
    pub source: RuntimeVariableSource,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct RuntimeVariableLayers {
    #[serde(default)]
    pub cli_args: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub manual_invocation_input: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub trigger_payload_mapping: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub workflow_defaults: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub config_defaults: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkflowHeader {
    #[serde(rename = "manifest_version", alias = "api_version")]
    manifest_version: String,
    #[serde(rename = "id", alias = "workflow_id")]
    id: String,
    name: String,
}

#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawWorkflowRuntime {
    #[serde(default)]
    defaults: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    workflow_defaults: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    config_defaults: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawWorkflowDefinition {
    #[serde(default)]
    workflow: Option<WorkflowHeader>,
    #[serde(rename = "manifest_version", alias = "api_version")]
    #[serde(default)]
    api_version: Option<String>,
    #[serde(rename = "id", alias = "workflow_id")]
    #[serde(default)]
    workflow_id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    runtime: RawWorkflowRuntime,
    #[serde(default)]
    nodes: Vec<NodeDefinition>,
}

impl RuntimeVariableLayers {
    pub fn resolve(&self) -> BTreeMap<String, ResolvedRuntimeVariable> {
        let mut keys = BTreeSet::new();
        keys.extend(self.cli_args.keys().cloned());
        keys.extend(self.manual_invocation_input.keys().cloned());
        keys.extend(self.trigger_payload_mapping.keys().cloned());
        keys.extend(self.workflow_defaults.keys().cloned());
        keys.extend(self.config_defaults.keys().cloned());

        let mut resolved = BTreeMap::new();
        for key in keys {
            let value = self
                .cli_args
                .get(&key)
                .map(|value| (value.clone(), RuntimeVariableSource::CliArgs))
                .or_else(|| {
                    self.manual_invocation_input
                        .get(&key)
                        .map(|value| (value.clone(), RuntimeVariableSource::ManualInvocationInput))
                })
                .or_else(|| {
                    self.trigger_payload_mapping
                        .get(&key)
                        .map(|value| (value.clone(), RuntimeVariableSource::TriggerPayloadMapping))
                })
                .or_else(|| {
                    self.workflow_defaults
                        .get(&key)
                        .map(|value| (value.clone(), RuntimeVariableSource::WorkflowDefaults))
                })
                .or_else(|| {
                    self.config_defaults
                        .get(&key)
                        .map(|value| (value.clone(), RuntimeVariableSource::ConfigDefaults))
                });

            if let Some((value, source)) = value {
                resolved.insert(key, ResolvedRuntimeVariable { value, source });
            }
        }

        resolved
    }

    pub fn resolve_namespaces(
        &self,
        subflow_input: BTreeMap<String, serde_json::Value>,
    ) -> RuntimeVariableNamespaces {
        let mut namespaces = RuntimeVariableNamespaces {
            cli_args: self.cli_args.clone(),
            manual_invocation_input: self.manual_invocation_input.clone(),
            trigger_payload_mapping: self.trigger_payload_mapping.clone(),
            workflow_defaults: self.workflow_defaults.clone(),
            config_defaults: self.config_defaults.clone(),
            node_outputs: BTreeMap::new(),
            run_scoped: BTreeMap::new(),
            subflow_input,
            subflow_output: BTreeMap::new(),
        };

        for (key, resolved) in self.resolve() {
            namespaces.run_scoped.insert(key, resolved.value);
        }

        namespaces
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WorkflowDefinition {
    pub api_version: String,
    pub workflow_id: String,
    pub name: String,
    #[serde(default)]
    pub runtime: RuntimeVariableLayers,
    pub nodes: Vec<NodeDefinition>,
    #[serde(skip)]
    pub package_root: PathBuf,
}

impl<'de> Deserialize<'de> for WorkflowDefinition {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawWorkflowDefinition::deserialize(deserializer)?;
        let api_version = raw
            .workflow
            .as_ref()
            .map(|header| header.manifest_version.clone())
            .or(raw.api_version)
            .ok_or_else(|| serde::de::Error::missing_field("workflow.manifest_version"))?;
        let workflow_id = raw
            .workflow
            .as_ref()
            .map(|header| header.id.clone())
            .or(raw.workflow_id)
            .ok_or_else(|| serde::de::Error::missing_field("workflow.id"))?;
        let name = raw
            .workflow
            .as_ref()
            .map(|header| header.name.clone())
            .or(raw.name)
            .ok_or_else(|| serde::de::Error::missing_field("workflow.name"))?;

        let mut runtime = RuntimeVariableLayers::default();
        runtime.workflow_defaults = if raw.runtime.defaults.is_empty() {
            raw.runtime.workflow_defaults
        } else {
            raw.runtime.defaults
        };
        runtime.config_defaults = raw.runtime.config_defaults;

        Ok(Self {
            api_version,
            workflow_id,
            name,
            runtime,
            nodes: raw.nodes,
            package_root: PathBuf::new(),
        })
    }
}

impl WorkflowDefinition {
    pub fn validate(&self) -> Result<(), ContractError> {
        assert_supported_major(
            "workflow.manifest_version",
            &self.api_version,
            CURRENT_API_MAJOR,
        )?;

        for node in &self.nodes {
            self.validate_node_contract(node)?;
            node.validate()?;
        }

        self.validate_graph_contract()
    }

    fn validate_node_contract(&self, node: &NodeDefinition) -> Result<(), ContractError> {
        for input in &node.inputs {
            if !input.validate() {
                return Err(ContractError::InvalidVariableReference {
                    workflow_id: self.workflow_id.clone(),
                    node_id: node.node_id.clone(),
                    context: "node.inputs",
                    namespace: input.source.namespace.as_str().to_owned(),
                    key: input.source.key.clone(),
                });
            }
        }

        if let Some(when) = &node.when {
            if !when.validate() {
                return Err(ContractError::InvalidVariableReference {
                    workflow_id: self.workflow_id.clone(),
                    node_id: node.node_id.clone(),
                    context: "node.when",
                    namespace: when.source.namespace.as_str().to_owned(),
                    key: when.source.key.clone(),
                });
            }
        }

        match (&node.subflow, node.kind.as_str()) {
            (Some(contract), "subflow") => {
                contract.validate(&self.workflow_id, &node.node_id, true)?;
            }
            (Some(_), _) => {
                return Err(ContractError::UnexpectedSubflowContract {
                    workflow_id: self.workflow_id.clone(),
                    node_id: node.node_id.clone(),
                });
            }
            (None, "subflow") => {
                return Err(ContractError::MissingSubflowContract {
                    workflow_id: self.workflow_id.clone(),
                    node_id: node.node_id.clone(),
                });
            }
            (None, _) => {}
        }

        Ok(())
    }

    fn validate_graph_contract(&self) -> Result<(), ContractError> {
        let mut node_ids = BTreeSet::new();
        for node in &self.nodes {
            if !node_ids.insert(node.node_id.clone()) {
                return Err(ContractError::DuplicateNodeId {
                    workflow_id: self.workflow_id.clone(),
                    node_id: node.node_id.clone(),
                });
            }
        }

        let mut graph = DiGraph::<String, ()>::new();
        let mut indices = BTreeMap::new();
        for node in &self.nodes {
            let index = graph.add_node(node.node_id.clone());
            indices.insert(node.node_id.clone(), index);
        }

        for node in &self.nodes {
            let Some(to) = indices.get(&node.node_id) else {
                return Err(ContractError::MissingDagNodeIndex {
                    workflow_id: self.workflow_id.clone(),
                    node_id: node.node_id.clone(),
                });
            };

            let mut dependencies = node.depends_on.clone();
            dependencies.sort();
            dependencies.dedup();

            for dependency in dependencies {
                if dependency == node.node_id {
                    return Err(ContractError::SelfDependency {
                        workflow_id: self.workflow_id.clone(),
                        node_id: node.node_id.clone(),
                    });
                }

                let Some(from) = indices.get(&dependency) else {
                    return Err(ContractError::UnknownNodeDependency {
                        workflow_id: self.workflow_id.clone(),
                        node_id: node.node_id.clone(),
                        dependency_id: dependency,
                    });
                };
                graph.add_edge(*from, *to, ());
            }
        }

        let mut cycle_nodes = BTreeSet::new();
        for component in tarjan_scc(&graph) {
            if component.len() > 1 {
                for index in component {
                    cycle_nodes.insert(graph[index].clone());
                }
            } else if let Some(index) = component.first().copied()
                && graph.contains_edge(index, index)
            {
                cycle_nodes.insert(graph[index].clone());
            }
        }

        if !cycle_nodes.is_empty() {
            return Err(ContractError::DagCycleDetected {
                workflow_id: self.workflow_id.clone(),
                node_ids: cycle_nodes.into_iter().collect(),
            });
        }

        Ok(())
    }
}
