//! [INPUT]
//! Serde-backed runtime variable references and the namespace rules used by workflow execution.
//!
//! [OUTPUT]
//! Defines runtime variable namespaces, bindings, references, and namespace collections used across workflow and runtime contracts.
//!
//! [ROLE]
//! Encodes the domain language for addressing workflow inputs, defaults, outputs, and subflow data.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Deserializer, Serialize};

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

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct VariableReference {
    pub namespace: RuntimeVariableNamespace,
    pub key: String,
}

impl<'de> Deserialize<'de> for VariableReference {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum RawVariableReference {
            Structured {
                namespace: RuntimeVariableNamespace,
                key: String,
            },
            Shorthand(String),
        }

        match RawVariableReference::deserialize(deserializer)? {
            RawVariableReference::Structured { namespace, key } => Ok(Self { namespace, key }),
            RawVariableReference::Shorthand(value) => {
                Self::from_shorthand(&value).map_err(serde::de::Error::custom)
            }
        }
    }
}

impl VariableReference {
    pub fn validate(&self) -> bool {
        !self.key.trim().is_empty()
    }

    pub fn as_string(&self) -> String {
        format!("{}:{}", self.namespace.as_str(), self.key)
    }

    fn from_shorthand(value: &str) -> Result<Self, String> {
        let (namespace_alias, key) = value.split_once('.').ok_or_else(|| {
            format!("invalid variable reference `{value}`; expected <namespace>.<key>")
        })?;

        let namespace = match namespace_alias {
            "cli" => RuntimeVariableNamespace::CliArgs,
            "manual" => RuntimeVariableNamespace::ManualInvocationInput,
            "trigger" => RuntimeVariableNamespace::TriggerPayloadMapping,
            "workflow" => RuntimeVariableNamespace::WorkflowDefaults,
            "config" => RuntimeVariableNamespace::ConfigDefaults,
            "node" => RuntimeVariableNamespace::NodeOutputs,
            "run" => RuntimeVariableNamespace::RunScoped,
            "subflow_input" => RuntimeVariableNamespace::SubflowInput,
            "subflow_output" => RuntimeVariableNamespace::SubflowOutput,
            other => {
                return Err(format!(
                    "invalid variable namespace alias `{other}` in `{value}`"
                ));
            }
        };

        if key.trim().is_empty() || key.contains('.') {
            return Err(format!(
                "invalid variable reference `{value}`; key must be a single segment"
            ));
        }

        Ok(Self {
            namespace,
            key: key.to_owned(),
        })
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
            RuntimeVariableNamespace::WorkflowDefaults => {
                self.workflow_defaults.get(&reference.key)
            }
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
