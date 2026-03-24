use std::collections::BTreeMap;

use crate::builtins::nodes::contract::{BuiltinNodeHandler, BuiltinNodeRequest, BuiltinNodeResult};
use crate::errors::ContractError;

#[derive(Debug, Clone, Copy)]
pub struct DataMathHandler;

impl BuiltinNodeHandler for DataMathHandler {
    fn kind(&self) -> &str {
        super::super::registry::BUILTIN_DATA_MATH_KIND
    }

    fn handle(&self, request: &BuiltinNodeRequest) -> Result<BuiltinNodeResult, ContractError> {
        let operation = MathOperation::parse(&request.operation, request)?;
        let result = match operation {
            MathOperation::Add => {
                let (left, right) = read_binary_number(request, "left", "right")?;
                left + right
            }
            MathOperation::Subtract => {
                let (left, right) = read_binary_number(request, "left", "right")?;
                left - right
            }
            MathOperation::Multiply => {
                let (left, right) = read_binary_number(request, "left", "right")?;
                left * right
            }
            MathOperation::Divide => {
                let (left, right) = read_binary_number(request, "left", "right")?;
                if right == 0.0 {
                    return Err(ContractError::CliUsage {
                        message: format!(
                            "workflow {} node {} cannot divide by zero",
                            request.workflow_id, request.node_id
                        ),
                    });
                }
                left / right
            }
            MathOperation::Min => {
                let (left, right) = read_binary_number(request, "left", "right")?;
                left.min(right)
            }
            MathOperation::Max => {
                let (left, right) = read_binary_number(request, "left", "right")?;
                left.max(right)
            }
            MathOperation::Round => {
                let value = read_number_input(request, "value")
                    .or_else(|_| read_number_input(request, "left"))?;
                let precision = request
                    .inputs
                    .get("precision")
                    .map(|value| read_number_value(value, "precision", request))
                    .transpose()?
                    .unwrap_or(0.0);
                let factor = 10_f64.powf(precision);
                (value * factor).round() / factor
            }
        };

        let outputs = BTreeMap::from([(String::from("result"), json_number(result, request)?)]);
        Ok(BuiltinNodeResult {
            outputs: outputs.clone(),
            run_scoped: outputs,
            ..BuiltinNodeResult::default()
        })
    }
}

#[derive(Debug, Clone, Copy)]
enum MathOperation {
    Add,
    Subtract,
    Multiply,
    Divide,
    Min,
    Max,
    Round,
}

impl MathOperation {
    fn parse(raw_operation: &str, request: &BuiltinNodeRequest) -> Result<Self, ContractError> {
        match raw_operation.trim() {
            "add" => Ok(Self::Add),
            "subtract" => Ok(Self::Subtract),
            "multiply" => Ok(Self::Multiply),
            "divide" => Ok(Self::Divide),
            "min" => Ok(Self::Min),
            "max" => Ok(Self::Max),
            "round" | "run" | "" => Ok(Self::Round),
            other => Err(ContractError::CliUsage {
                message: format!(
                    "workflow {} node {} has unsupported math operation {}",
                    request.workflow_id, request.node_id, other
                ),
            }),
        }
    }
}

fn read_binary_number(
    request: &BuiltinNodeRequest,
    left_key: &str,
    right_key: &str,
) -> Result<(f64, f64), ContractError> {
    Ok((
        read_number_input(request, left_key)?,
        read_number_input(request, right_key)?,
    ))
}

fn read_number_input(request: &BuiltinNodeRequest, key: &str) -> Result<f64, ContractError> {
    let value = request
        .inputs
        .get(key)
        .ok_or_else(|| ContractError::CliUsage {
            message: format!(
                "workflow {} node {} requires data.math input {}",
                request.workflow_id, request.node_id, key
            ),
        })?;
    read_number_value(value, key, request)
}

fn read_number_value(
    value: &serde_json::Value,
    field: &str,
    request: &BuiltinNodeRequest,
) -> Result<f64, ContractError> {
    value.as_f64().ok_or_else(|| ContractError::CliUsage {
        message: format!(
            "workflow {} node {} expects data.math input {} to be numeric",
            request.workflow_id, request.node_id, field
        ),
    })
}

fn json_number(
    value: f64,
    request: &BuiltinNodeRequest,
) -> Result<serde_json::Value, ContractError> {
    serde_json::Number::from_f64(value)
        .map(serde_json::Value::Number)
        .ok_or_else(|| ContractError::CliUsage {
            message: format!(
                "workflow {} node {} produced non-finite math result",
                request.workflow_id, request.node_id
            ),
        })
}
