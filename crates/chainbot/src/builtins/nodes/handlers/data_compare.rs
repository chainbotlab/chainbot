use std::cmp::Ordering;
use std::collections::BTreeMap;

use crate::builtins::nodes::contract::{BuiltinNodeHandler, BuiltinNodeRequest, BuiltinNodeResult};
use crate::errors::ContractError;

#[derive(Debug, Clone, Copy)]
pub struct DataCompareHandler;

impl BuiltinNodeHandler for DataCompareHandler {
    fn kind(&self) -> &str {
        super::super::registry::BUILTIN_DATA_COMPARE_KIND
    }

    fn handle(&self, request: &BuiltinNodeRequest) -> Result<BuiltinNodeResult, ContractError> {
        let operation = CompareOperation::parse(&request.operation, request)?;
        let left = request
            .inputs
            .get("left")
            .ok_or_else(|| ContractError::CliUsage {
                message: format!(
                    "workflow {} node {} requires data.compare input left",
                    request.workflow_id, request.node_id
                ),
            })?;
        let right = request
            .inputs
            .get("right")
            .ok_or_else(|| ContractError::CliUsage {
                message: format!(
                    "workflow {} node {} requires data.compare input right",
                    request.workflow_id, request.node_id
                ),
            })?;

        let result = evaluate_compare(operation, left, right, request)?;
        let outputs = BTreeMap::from([(String::from("result"), serde_json::Value::Bool(result))]);
        Ok(BuiltinNodeResult {
            outputs: outputs.clone(),
            run_scoped: outputs,
            ..BuiltinNodeResult::default()
        })
    }
}

#[derive(Debug, Clone, Copy)]
enum CompareOperation {
    Equals,
    NotEquals,
    GreaterThan,
    GreaterThanOrEquals,
    LessThan,
    LessThanOrEquals,
}

impl CompareOperation {
    fn parse(raw_operation: &str, request: &BuiltinNodeRequest) -> Result<Self, ContractError> {
        match raw_operation.trim() {
            "" | "run" | "equals" => Ok(Self::Equals),
            "not_equals" => Ok(Self::NotEquals),
            "greater_than" => Ok(Self::GreaterThan),
            "greater_than_or_equals" => Ok(Self::GreaterThanOrEquals),
            "less_than" => Ok(Self::LessThan),
            "less_than_or_equals" => Ok(Self::LessThanOrEquals),
            other => Err(ContractError::CliUsage {
                message: format!(
                    "workflow {} node {} has unsupported data.compare operation {}",
                    request.workflow_id, request.node_id, other
                ),
            }),
        }
    }
}

fn evaluate_compare(
    operation: CompareOperation,
    left: &serde_json::Value,
    right: &serde_json::Value,
    request: &BuiltinNodeRequest,
) -> Result<bool, ContractError> {
    Ok(match operation {
        CompareOperation::Equals => left == right,
        CompareOperation::NotEquals => left != right,
        CompareOperation::GreaterThan => {
            compare_scalars(left, right, request)? == Ordering::Greater
        }
        CompareOperation::GreaterThanOrEquals => {
            compare_scalars(left, right, request)? != Ordering::Less
        }
        CompareOperation::LessThan => compare_scalars(left, right, request)? == Ordering::Less,
        CompareOperation::LessThanOrEquals => {
            compare_scalars(left, right, request)? != Ordering::Greater
        }
    })
}

fn compare_scalars(
    left: &serde_json::Value,
    right: &serde_json::Value,
    request: &BuiltinNodeRequest,
) -> Result<Ordering, ContractError> {
    match (left, right) {
        (serde_json::Value::Number(left), serde_json::Value::Number(right)) => {
            let left = left.as_f64().ok_or_else(|| ContractError::CliUsage {
                message: format!(
                    "workflow {} node {} could not compare non-finite left number",
                    request.workflow_id, request.node_id
                ),
            })?;
            let right = right.as_f64().ok_or_else(|| ContractError::CliUsage {
                message: format!(
                    "workflow {} node {} could not compare non-finite right number",
                    request.workflow_id, request.node_id
                ),
            })?;
            left.partial_cmp(&right)
                .ok_or_else(|| ContractError::CliUsage {
                    message: format!(
                        "workflow {} node {} could not compare non-finite numbers",
                        request.workflow_id, request.node_id
                    ),
                })
        }
        (serde_json::Value::String(left), serde_json::Value::String(right)) => Ok(left.cmp(right)),
        (serde_json::Value::Bool(left), serde_json::Value::Bool(right)) => Ok(left.cmp(right)),
        _ => Err(ContractError::CliUsage {
            message: format!(
                "workflow {} node {} supports ordered compare only for matching scalar types",
                request.workflow_id, request.node_id
            ),
        }),
    }
}
