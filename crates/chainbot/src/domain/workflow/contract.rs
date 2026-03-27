//! [INPUT]
//! Runtime node contracts, workflow variable namespaces, subflow contracts, graph helpers, and manifest-version validation.
//!
//! [OUTPUT]
//! Defines validated workflow manifest types, dependency modes, and graph-derived workflow invariants.
//!
//! [ROLE]
//! Provides the backend-agnostic domain contract for workflow definitions.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use petgraph::algo::tarjan_scc;
use petgraph::graph::DiGraph;
use serde::{Deserialize, Deserializer, Serialize};

use crate::domain::runtime::NodeDefinition;
use crate::errors::{assert_required_major, ContractError};

use super::subflow::{SubflowContract, SubflowExport, SubflowImport};
use super::variables::{RuntimeVariableLayers, VariableBinding, VariableReference};
use super::when::WhenCondition;

pub const CURRENT_API_MAJOR: u64 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DependsMode {
    #[default]
    All,
    Any,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkflowHeader {
    #[serde(rename = "manifest_version")]
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

#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSubflowCallDefinition {
    workflow: String,
    #[serde(default, rename = "with")]
    imports: BTreeMap<String, VariableReference>,
    #[serde(default, rename = "returns")]
    exports: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawNodeDefinition {
    #[serde(rename = "manifest_version")]
    pub api_version: String,
    #[serde(rename = "id", alias = "node_id")]
    pub node_id: String,
    pub kind: String,
    #[serde(rename = "plugin", alias = "plugin_id")]
    #[serde(default)]
    pub plugin_id: Option<String>,
    #[serde(default)]
    pub operation: Option<String>,
    #[serde(default)]
    pub depends_mode: DependsMode,
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub inputs: Vec<VariableBinding>,
    #[serde(default)]
    pub when: Option<WhenCondition>,
    #[serde(default)]
    pub call: Option<RawSubflowCallDefinition>,
}

impl RawNodeDefinition {
    fn lower(self) -> Result<NodeDefinition, String> {
        let Self {
            api_version,
            node_id,
            kind,
            plugin_id,
            operation,
            depends_mode,
            depends_on,
            inputs,
            when,
            call,
        } = self;

        let lowered_subflow = call.map(|call| call.lower(&node_id, &kind)).transpose()?;

        if kind != "subflow" && lowered_subflow.is_some() {
            return Err(format!(
                "node {node_id}: only kind `subflow` may define `call`"
            ));
        }

        if kind == "subflow" && lowered_subflow.is_none() {
            return Err(format!(
                "node {node_id}: kind `subflow` requires a `call.workflow` definition"
            ));
        }

        let plugin_id = match (kind.as_str(), plugin_id) {
            ("subflow", Some(value)) => {
                if value != "builtin-subflow" {
                    return Err(format!(
                        "node {node_id}: subflow nodes must use plugin `builtin-subflow`"
                    ));
                }
                value
            }
            ("subflow", None) => String::from("builtin-subflow"),
            (_, Some(value)) => value,
            (_, None) => {
                return Err(format!(
                    "node {node_id}: missing required field `plugin`"
                ));
            }
        };

        let operation = match (kind.as_str(), operation) {
            ("subflow", Some(value)) => {
                if value != "run" {
                    return Err(format!(
                        "node {node_id}: subflow nodes must use operation `run`"
                    ));
                }
                value
            }
            ("subflow", None) => String::from("run"),
            (_, Some(value)) => value,
            (_, None) => {
                return Err(format!(
                    "node {node_id}: missing required field `operation`"
                ));
            }
        };

        Ok(NodeDefinition {
            api_version,
            node_id,
            kind,
            plugin_id,
            operation,
            depends_mode,
            depends_on,
            inputs,
            when,
            subflow: lowered_subflow,
        })
    }
}

impl RawSubflowCallDefinition {
    fn lower(self, node_id: &str, kind: &str) -> Result<SubflowContract, String> {
        if kind != "subflow" {
            return Err(format!(
                "node {node_id}: only kind `subflow` may define `call`"
            ));
        }

        if self.workflow.trim().is_empty() {
            return Err(format!(
                "node {node_id}: subflow call.workflow is required"
            ));
        }

        let mut imports = Vec::new();
        for (child_key, source) in self.imports {
            imports.push(SubflowImport { child_key, source });
        }

        let mut exports = Vec::new();
        for (child_key, parent_key) in self.exports {
            exports.push(SubflowExport {
                child_key,
                parent_key,
            });
        }

        Ok(SubflowContract {
            workflow_id: self.workflow,
            imports,
            exports,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawWorkflowDefinition {
    #[serde(default)]
    workflow: Option<WorkflowHeader>,
    #[serde(rename = "manifest_version")]
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
    nodes: Vec<RawNodeDefinition>,
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

        let nodes = raw
            .nodes
            .into_iter()
            .map(|node| node.lower().map_err(serde::de::Error::custom))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Self {
            api_version,
            workflow_id,
            name,
            runtime,
            nodes,
            package_root: PathBuf::new(),
        })
    }
}

impl WorkflowDefinition {
    pub fn validate(&self) -> Result<(), ContractError> {
        assert_required_major(
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
        if node.kind == "subflow" && !node.inputs.is_empty() {
            return Err(ContractError::UnexpectedSubflowNodeInputs {
                workflow_id: self.workflow_id.clone(),
                node_id: node.node_id.clone(),
            });
        }

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

        if let Some(when) = &node.when
            && !when.validate()
        {
            return Err(ContractError::InvalidVariableReference {
                workflow_id: self.workflow_id.clone(),
                node_id: node.node_id.clone(),
                context: "node.when",
                namespace: when.source.namespace.as_str().to_owned(),
                key: when.source.key.clone(),
            });
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
