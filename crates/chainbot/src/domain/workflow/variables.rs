//! [INPUT]
//! Serde-backed runtime variable references and the namespace rules used by workflow execution.
//!
//! [OUTPUT]
//! Defines runtime variable namespaces, bindings, references, and namespace collections used across workflow and runtime contracts.
//!
//! [ROLE]
//! Encodes the domain language for addressing workflow inputs, defaults, outputs, and subflow data.

use std::collections::{BTreeMap, BTreeSet};

use serde::{ser::SerializeStruct, Deserialize, Deserializer, Serialize, Serializer};

use crate::errors::ContractError;

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

#[derive(Debug, Clone, PartialEq)]
pub struct VariableReference {
    pub namespace: RuntimeVariableNamespace,
    pub key: String,
}

impl Serialize for VariableReference {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if let Some((producer, key)) = self.node_output_address() {
            let mut reference = serializer.serialize_struct("VariableReference", 3)?;
            reference.serialize_field("namespace", &self.namespace)?;
            reference.serialize_field("producer", producer)?;
            reference.serialize_field("key", key)?;
            reference.end()
        } else {
            let mut reference = serializer.serialize_struct("VariableReference", 2)?;
            reference.serialize_field("namespace", &self.namespace)?;
            reference.serialize_field("key", &self.key)?;
            reference.end()
        }
    }
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
                #[serde(default)]
                producer: Option<String>,
                key: String,
            },
            Shorthand(String),
        }

        match RawVariableReference::deserialize(deserializer)? {
            RawVariableReference::Structured {
                namespace,
                producer,
                key,
            } => {
                if producer.is_some() && namespace != RuntimeVariableNamespace::NodeOutputs {
                    return Err(serde::de::Error::custom(
                        "producer is only valid for namespace=node_outputs",
                    ));
                }
                let key = match producer {
                    Some(producer)
                        if !producer.trim().is_empty()
                            && !key.trim().is_empty()
                            && !producer.contains('.')
                            && !key.contains('.') =>
                    {
                        format!("{producer}.{key}")
                    }
                    Some(_) => {
                        return Err(serde::de::Error::custom(
                            "producer and key must be non-empty single segments",
                        ));
                    }
                    None => key,
                };
                let reference = Self { namespace, key };
                if !reference.validate() {
                    return Err(serde::de::Error::custom(
                        "variable key must be one segment, or producer.key for node_outputs",
                    ));
                }
                Ok(reference)
            }
            RawVariableReference::Shorthand(value) => {
                Self::from_shorthand(&value).map_err(serde::de::Error::custom)
            }
        }
    }
}

impl VariableReference {
    pub fn validate(&self) -> bool {
        if self.key.trim().is_empty() || self.key.split('.').any(str::is_empty) {
            return false;
        }
        let segment_count = self.key.split('.').count();
        match self.namespace {
            RuntimeVariableNamespace::NodeOutputs => segment_count <= 2,
            _ => segment_count == 1,
        }
    }

    pub fn as_string(&self) -> String {
        format!("{}:{}", self.namespace.as_str(), self.key)
    }

    pub fn node_output_address(&self) -> Option<(&str, &str)> {
        (self.namespace == RuntimeVariableNamespace::NodeOutputs)
            .then(|| self.key.split_once('.'))
            .flatten()
    }

    pub fn is_legacy_node_output(&self) -> bool {
        self.namespace == RuntimeVariableNamespace::NodeOutputs
            && self.node_output_address().is_none()
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

        if key.trim().is_empty()
            || key.split('.').any(str::is_empty)
            || (namespace == RuntimeVariableNamespace::NodeOutputs && key.split('.').count() > 2)
            || (namespace != RuntimeVariableNamespace::NodeOutputs && key.contains('.'))
        {
            return Err(format!(
                "invalid variable reference `{value}`; expected node.<key> or node.<producer>.<key>, and single-segment keys elsewhere"
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

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
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
    pub node_outputs_by_producer: BTreeMap<String, BTreeMap<String, serde_json::Value>>,
    #[serde(default)]
    pub run_scoped: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub subflow_input: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub subflow_output: BTreeMap<String, serde_json::Value>,
}

impl RuntimeVariableNamespaces {
    pub fn try_resolve(
        &self,
        reference: &VariableReference,
    ) -> Result<Option<&serde_json::Value>, ContractError> {
        match reference.namespace {
            RuntimeVariableNamespace::CliArgs => Ok(self.cli_args.get(&reference.key)),
            RuntimeVariableNamespace::ManualInvocationInput => {
                Ok(self.manual_invocation_input.get(&reference.key))
            }
            RuntimeVariableNamespace::TriggerPayloadMapping => {
                Ok(self.trigger_payload_mapping.get(&reference.key))
            }
            RuntimeVariableNamespace::WorkflowDefaults => {
                Ok(self.workflow_defaults.get(&reference.key))
            }
            RuntimeVariableNamespace::ConfigDefaults => Ok(self.config_defaults.get(&reference.key)),
            RuntimeVariableNamespace::NodeOutputs => {
                if let Some((producer, key)) = reference.node_output_address() {
                    return Ok(self
                        .node_outputs_by_producer
                        .get(producer)
                        .and_then(|outputs| outputs.get(key)));
                }
                let producers = self
                    .node_outputs_by_producer
                    .iter()
                    .filter(|(_, outputs)| outputs.contains_key(&reference.key))
                    .map(|(producer, _)| producer.clone())
                    .collect::<Vec<_>>();
                if producers.len() > 1 {
                    return Err(ContractError::AmbiguousLegacyNodeOutput {
                        key: reference.key.clone(),
                        producer_ids: producers,
                    });
                }
                if let Some(producer) = producers.first() {
                    return Ok(self
                        .node_outputs_by_producer
                        .get(producer)
                        .and_then(|outputs| outputs.get(&reference.key)));
                }
                Ok(self.node_outputs.get(&reference.key))
            }
            RuntimeVariableNamespace::RunScoped => Ok(self.run_scoped.get(&reference.key)),
            RuntimeVariableNamespace::SubflowInput => Ok(self.subflow_input.get(&reference.key)),
            RuntimeVariableNamespace::SubflowOutput => Ok(self.subflow_output.get(&reference.key)),
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
            node_outputs_by_producer: BTreeMap::new(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addressed_node_reference_serializes_with_canonical_producer_field() {
        let reference: VariableReference =
            serde_json::from_str(r#""node.fetch.price""#).expect("shorthand should parse");

        let serialized = serde_json::to_value(&reference).expect("reference should serialize");
        assert_eq!(
            serialized,
            serde_json::json!({
                "namespace": "node_outputs",
                "producer": "fetch",
                "key": "price"
            })
        );
        assert_eq!(
            serde_json::from_value::<VariableReference>(serialized)
                .expect("canonical structured reference should parse"),
            reference
        );
    }

    #[test]
    fn node_reference_rejects_more_than_producer_and_key_segments() {
        assert!(serde_json::from_str::<VariableReference>(r#""node.a.b.c""#).is_err());
        assert!(serde_json::from_value::<VariableReference>(serde_json::json!({
            "namespace": "node_outputs",
            "producer": "a.b",
            "key": "c"
        }))
        .is_err());
    }
}
