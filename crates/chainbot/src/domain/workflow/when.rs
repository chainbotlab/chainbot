//! [INPUT]
//! Runtime variable namespaces, variable references, and serde-backed conditional operator decoding.
//!
//! [OUTPUT]
//! Defines workflow `when` conditions and validation logic for conditional node execution.
//!
//! [ROLE]
//! Models the domain contract for workflow branching predicates.

use serde::{Deserialize, Serialize};

use crate::errors::ContractError;

use super::variables::{RuntimeVariableNamespaces, VariableReference};

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
        self.try_evaluate(namespaces).unwrap_or(false)
    }

    pub fn try_evaluate(
        &self,
        namespaces: &RuntimeVariableNamespaces,
    ) -> Result<bool, ContractError> {
        let value = namespaces.try_resolve(&self.source)?;
        Ok(match self.operator {
            WhenOperator::Exists => value.is_some(),
            WhenOperator::Equals => value
                .zip(self.expected.as_ref())
                .is_some_and(|(left, right)| left == right),
            WhenOperator::NotEquals => value
                .zip(self.expected.as_ref())
                .is_some_and(|(left, right)| left != right),
            WhenOperator::Truthy => value.is_some_and(is_truthy),
            WhenOperator::Falsy => value.is_none_or(|candidate| !is_truthy(candidate)),
        })
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
