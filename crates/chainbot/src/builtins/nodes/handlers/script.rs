//! [INPUT]
//! Builtin script requests, runtime context, secret-aware input resolution, worker process contracts, and script protocol envelopes.
//!
//! [OUTPUT]
//! Executes builtin script nodes through worker runtimes and returns normalized outputs or execution failures.
//!
//! [ROLE]
//! Implements the builtin node that runs script-based operations.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::builtins::nodes::context::BuiltinRuntimeContext;
use crate::builtins::nodes::contract::{BuiltinNodeHandler, BuiltinNodeRequest, BuiltinNodeResult};
use crate::builtins::nodes::input_resolver::resolve_node_inputs;
use crate::builtins::nodes::script_worker::{ScriptRuntime, WorkerProcessSpec};
use crate::errors::ContractError;
use crate::script_protocol::WorkerRequestEnvelope;
use crate::secrets::redact_text;

const SCRIPT_RUNTIME_PYTHON: &str = "python";
const SCRIPT_RUNTIME_JAVASCRIPT: &str = "javascript";

#[derive(Debug, Clone)]
pub struct ScriptHandler {
    context: Arc<BuiltinRuntimeContext>,
}

#[derive(Debug, Clone)]
struct ScriptNodeSpec {
    runtime: ScriptRuntime,
    script_relative_path: String,
}

impl ScriptHandler {
    pub fn new(context: Arc<BuiltinRuntimeContext>) -> Self {
        Self { context }
    }
}

impl BuiltinNodeHandler for ScriptHandler {
    fn kind(&self) -> &str {
        super::super::registry::BUILTIN_SCRIPT_KIND
    }

    fn handle(&self, request: &BuiltinNodeRequest) -> Result<BuiltinNodeResult, ContractError> {
        let script_spec = parse_script_node_operation(&request.operation)?;
        let interpreter_path =
            resolve_interpreter_for_runtime(script_spec.runtime).ok_or_else(|| {
                ContractError::CliUsage {
                    message: format!(
                        "workflow {} node {} cannot resolve interpreter for runtime {}",
                        request.workflow_id,
                        request.node_id,
                        match script_spec.runtime {
                            ScriptRuntime::Python => SCRIPT_RUNTIME_PYTHON,
                            ScriptRuntime::JavaScript => SCRIPT_RUNTIME_JAVASCRIPT,
                        }
                    ),
                }
            })?;

        let script_base_dir = if request.workflow_package_root.as_os_str().is_empty() {
            self.context.root_layout.root.clone()
        } else {
            request.workflow_package_root.clone()
        };
        let script_path = script_base_dir.join(&script_spec.script_relative_path);
        let resolved = resolve_node_inputs(
            &self.context.root_layout.secrets_dir,
            self.context.secret_mode,
            &request.inputs,
        )?;

        let process = WorkerProcessSpec::new(script_spec.runtime, interpreter_path, script_path);
        let response = self
            .context
            .worker_host
            .execute(
                &process,
                &WorkerRequestEnvelope {
                    protocol_version: "1.0.0".to_owned(),
                    request_id: format!("{}::{}", request.run_id, request.node_id),
                    worker_id: request.node_id.clone(),
                    workflow_id: request.workflow_id.clone(),
                    payload: serde_json::to_value(&resolved.values)?,
                },
            )
            .map_err(|source| ContractError::CliUsage {
                message: format!(
                    "script worker failed for workflow {} node {}: {}",
                    request.workflow_id,
                    request.node_id,
                    redact_text(&source.to_string(), &resolved.resolved_secrets)
                ),
            })?;

        let outputs = match response.output {
            serde_json::Value::Object(values) => values.into_iter().collect(),
            other => BTreeMap::from([(String::from("result"), other)]),
        };

        Ok(BuiltinNodeResult {
            outputs: outputs.clone(),
            run_scoped: outputs,
            ..BuiltinNodeResult::default()
        })
    }
}

fn parse_script_node_operation(operation: &str) -> Result<ScriptNodeSpec, ContractError> {
    let Some((runtime_raw, script_relative_path)) = operation.split_once(':') else {
        return Err(ContractError::CliUsage {
            message: format!(
                "script node operation must use <runtime>:<relative_script_path>, got {operation}"
            ),
        });
    };

    let runtime = match runtime_raw.trim() {
        SCRIPT_RUNTIME_PYTHON => ScriptRuntime::Python,
        SCRIPT_RUNTIME_JAVASCRIPT => ScriptRuntime::JavaScript,
        other => {
            return Err(ContractError::CliUsage {
                message: format!(
                    "script node runtime {other} is unsupported; use python or javascript"
                ),
            });
        }
    };

    let script_relative_path = script_relative_path.trim();
    if script_relative_path.is_empty() {
        return Err(ContractError::CliUsage {
            message: "script node operation requires a relative script path".to_owned(),
        });
    }

    Ok(ScriptNodeSpec {
        runtime,
        script_relative_path: script_relative_path.to_owned(),
    })
}

fn resolve_interpreter_for_runtime(runtime: ScriptRuntime) -> Option<PathBuf> {
    let candidates: &[&str] = match runtime {
        ScriptRuntime::Python => &["python3", "python"],
        ScriptRuntime::JavaScript => &["node", "nodejs"],
    };
    let path_env = std::env::var_os("PATH")?;

    for candidate in candidates {
        for root in std::env::split_paths(&path_env) {
            let path = root.join(candidate);
            if path.is_file() {
                return Some(path);
            }
        }
    }

    None
}
