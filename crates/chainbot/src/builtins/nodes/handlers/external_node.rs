use std::sync::Arc;

use crate::builtins::nodes::context::BuiltinRuntimeContext;
use crate::builtins::nodes::contract::{BuiltinNodeHandler, BuiltinNodeRequest, BuiltinNodeResult};
use crate::builtins::nodes::input_resolver::resolve_node_inputs;
use crate::errors::ContractError;
use crate::plugin::{
    ExternalNodePluginHost, ExternalNodePluginRequest, PluginKind, NODE_PLUGIN_CONTRACT_VERSION,
    NODE_PLUGIN_EXECUTE_CAPABILITY,
};
use crate::secrets::redact_text;

#[derive(Debug, Clone)]
pub struct ExternalNodeHandler {
    context: Arc<BuiltinRuntimeContext>,
}

impl ExternalNodeHandler {
    pub fn new(context: Arc<BuiltinRuntimeContext>) -> Self {
        Self { context }
    }
}

impl BuiltinNodeHandler for ExternalNodeHandler {
    fn kind(&self) -> &str {
        super::super::registry::BUILTIN_EXTERNAL_NODE_KIND
    }

    fn handle(&self, request: &BuiltinNodeRequest) -> Result<BuiltinNodeResult, ContractError> {
        let plugin_id = request.operation.trim();
        if plugin_id.is_empty() {
            return Err(ContractError::CliUsage {
                message: format!(
                    "workflow {} node {} missing plugin id in operation field",
                    request.workflow_id, request.node_id
                ),
            });
        }

        let manifest =
            self.context
                .manifests
                .get(plugin_id)
                .ok_or_else(|| ContractError::CliUsage {
                    message: format!(
                        "workflow {} node {} references unknown external node plugin {}",
                        request.workflow_id, request.node_id, plugin_id
                    ),
                })?;

        if manifest.kind()? != PluginKind::ExternalNode {
            return Err(ContractError::CliUsage {
                message: format!(
                    "workflow {} node {} expected external node plugin kind for {}",
                    request.workflow_id, request.node_id, plugin_id
                ),
            });
        }

        let resolved = resolve_node_inputs(
            &self.context.root_layout.secrets_dir,
            self.context.secret_mode,
            &request.inputs,
        )?;
        let host = ExternalNodePluginHost::new(self.context.root_layout.plugins_dir.clone());
        let response = host
            .execute(
                manifest,
                &ExternalNodePluginRequest {
                    contract_version: NODE_PLUGIN_CONTRACT_VERSION.to_owned(),
                    plugin_id: plugin_id.to_owned(),
                    node_id: request.node_id.clone(),
                    operation: "execute".to_owned(),
                    requested_capabilities: vec![NODE_PLUGIN_EXECUTE_CAPABILITY.to_owned()],
                    input: resolved.values,
                },
            )
            .map_err(|source| ContractError::CliUsage {
                message: redact_text(&source.to_string(), &resolved.resolved_secrets),
            })?;

        Ok(BuiltinNodeResult {
            outputs: response.output.clone(),
            run_scoped: response.output,
            ..BuiltinNodeResult::default()
        })
    }
}
