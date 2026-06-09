use bridge_action_core::{
    failure_response as bridge_failure_response, handle_request_json as handle_bridge_request_json,
    OperationSpec, PluginError, PluginSpec,
};

const OPERATIONS: &[OperationSpec] = &[
    OperationSpec {
        name: "base_prepare_l1_standard_bridge_deposit",
        action_kind: "op_stack_l1_standard_bridge_deposit",
        target_input: "l1_standard_bridge",
        chain_input: "l1_chain_id",
        required_inputs: &["l1_chain_id", "l1_standard_bridge", "l1_token", "l2_token", "to", "amount", "min_gas_limit"],
        optional_inputs: &["extra_data", "value"],
    },
    OperationSpec {
        name: "base_prepare_l2_standard_bridge_withdraw",
        action_kind: "op_stack_l2_standard_bridge_withdraw",
        target_input: "l2_standard_bridge",
        chain_input: "l2_chain_id",
        required_inputs: &["l2_chain_id", "l2_standard_bridge", "l2_token", "to", "amount", "min_gas_limit"],
        optional_inputs: &["extra_data", "value"],
    },
];

const SPEC: PluginSpec = PluginSpec {
    plugin_id: "base-bridge-node",
    provider: "base",
    operations: OPERATIONS,
};

pub async fn handle_request_json(input: &str) -> Result<String, PluginError> {
    handle_bridge_request_json(input, &SPEC).await
}

pub fn failure_response(message: &str) -> String {
    bridge_failure_response(message)
}
