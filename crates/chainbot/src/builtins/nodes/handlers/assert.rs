//! [INPUT]
//! Builtin assert requests containing comparison or truthiness inputs and builtin node operation names.
//!
//! [OUTPUT]
//! Returns assertion pass or fail results as builtin node outputs or `ContractError` failures.
//!
//! [ROLE]
//! Implements the builtin assert node behavior.

use std::collections::BTreeMap;

use crate::builtins::nodes::contract::{BuiltinNodeHandler, BuiltinNodeRequest, BuiltinNodeResult};
use crate::errors::ContractError;

#[derive(Debug, Clone, Copy)]
pub struct AssertHandler;

impl BuiltinNodeHandler for AssertHandler {
    fn kind(&self) -> &str {
        super::super::registry::BUILTIN_ASSERT_KIND
    }

    fn handle(&self, request: &BuiltinNodeRequest) -> Result<BuiltinNodeResult, ContractError> {
        let operation = normalized_assert_operation(&request.operation, request)?;
        let value = request.inputs.get("value");
        let expected = request.inputs.get("expected");
        let passed = match operation {
            AssertOperation::Truthy => value.is_some_and(is_truthy),
            AssertOperation::Falsy => value.is_none_or(|candidate| !is_truthy(candidate)),
            AssertOperation::Exists => value.is_some(),
            AssertOperation::Equals => value
                .zip(expected)
                .is_some_and(|(left, right)| left == right),
            AssertOperation::NotEquals => value
                .zip(expected)
                .is_some_and(|(left, right)| left != right),
        };

        if !passed {
            let message = request
                .inputs
                .get("message")
                .map(|value| json_string_field(value, "message", request))
                .transpose()?
                .unwrap_or_else(|| {
                    format!("assertion failed for operation {}", operation.as_str())
                });
            let code_suffix = request
                .inputs
                .get("code")
                .map(|value| json_string_field(value, "code", request))
                .transpose()?
                .map(|code| format!(" code={code}"))
                .unwrap_or_default();
            return Err(ContractError::CliUsage {
                message: format!(
                    "workflow {} node {} {}{}",
                    request.workflow_id, request.node_id, message, code_suffix
                ),
            });
        }

        let mut outputs = BTreeMap::from([(String::from("ok"), serde_json::Value::Bool(true))]);
        if let Some(value) = value {
            outputs.insert(String::from("checked_value"), value.clone());
        }

        Ok(BuiltinNodeResult {
            outputs: outputs.clone(),
            run_scoped: outputs,
            ..BuiltinNodeResult::default()
        })
    }
}

#[derive(Debug, Clone, Copy)]
enum AssertOperation {
    Truthy,
    Falsy,
    Exists,
    Equals,
    NotEquals,
}

impl AssertOperation {
    fn as_str(self) -> &'static str {
        match self {
            Self::Truthy => "truthy",
            Self::Falsy => "falsy",
            Self::Exists => "exists",
            Self::Equals => "equals",
            Self::NotEquals => "not_equals",
        }
    }
}

fn normalized_assert_operation(
    raw_operation: &str,
    request: &BuiltinNodeRequest,
) -> Result<AssertOperation, ContractError> {
    match raw_operation.trim() {
        "" | "run" | "truthy" => Ok(AssertOperation::Truthy),
        "falsy" => Ok(AssertOperation::Falsy),
        "exists" => Ok(AssertOperation::Exists),
        "equals" => Ok(AssertOperation::Equals),
        "not_equals" => Ok(AssertOperation::NotEquals),
        other => Err(ContractError::CliUsage {
            message: format!(
                "workflow {} node {} has unsupported assert operation {}",
                request.workflow_id, request.node_id, other
            ),
        }),
    }
}

fn json_string_field(
    value: &serde_json::Value,
    field: &str,
    request: &BuiltinNodeRequest,
) -> Result<String, ContractError> {
    value
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| ContractError::CliUsage {
            message: format!(
                "workflow {} node {} expects assert input {} to be a string",
                request.workflow_id, request.node_id, field
            ),
        })
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
