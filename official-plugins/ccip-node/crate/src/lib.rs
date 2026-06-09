use bridge_action_core::{
    failure_response as bridge_failure_response, handle_request_json as handle_bridge_request_json,
    OperationSpec, PluginError, PluginSpec,
};

const OPERATIONS: &[OperationSpec] = &[
    OperationSpec {
        name: "ccip_prepare_token_transfer",
        action_kind: "ccip_token_transfer",
        target_input: "router",
        chain_input: "source_chain_id",
        required_inputs: &["source_chain_id", "router", "destination_chain_selector", "receiver", "token", "amount"],
        optional_inputs: &["fee_token", "message_data", "extra_args", "value"],
    },
    OperationSpec {
        name: "ccip_prepare_token_pool_admin",
        action_kind: "ccip_token_pool_admin",
        target_input: "token_pool",
        chain_input: "chain_id",
        required_inputs: &["chain_id", "token_pool"],
        optional_inputs: &["remote_chain_selector", "remote_pool", "rate_limiter_config", "value", "call_data"],
    },
];

const SPEC: PluginSpec = PluginSpec {
    plugin_id: "ccip-node",
    provider: "chainlink-ccip",
    operations: OPERATIONS,
};

pub async fn handle_request_json(input: &str) -> Result<String, PluginError> {
    handle_bridge_request_json(input, &SPEC).await
}

pub fn failure_response(message: &str) -> String {
    bridge_failure_response(message)
}
