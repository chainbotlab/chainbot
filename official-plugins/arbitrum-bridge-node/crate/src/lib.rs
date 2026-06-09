use bridge_action_core::{
    failure_response as bridge_failure_response, handle_request_json as handle_bridge_request_json,
    OperationSpec, PluginError, PluginSpec,
};

const OPERATIONS: &[OperationSpec] = &[
    OperationSpec {
        name: "arbitrum_prepare_l1_erc20_deposit",
        action_kind: "arbitrum_l1_gateway_deposit",
        target_input: "l1_gateway_router",
        chain_input: "l1_chain_id",
        required_inputs: &["l1_chain_id", "l1_gateway_router", "l1_token", "to", "amount", "max_gas", "gas_price_bid"],
        optional_inputs: &["refund_to", "gateway_data", "value"],
    },
    OperationSpec {
        name: "arbitrum_prepare_l2_erc20_withdraw",
        action_kind: "arbitrum_l2_gateway_withdraw",
        target_input: "l2_gateway_router",
        chain_input: "l2_chain_id",
        required_inputs: &["l2_chain_id", "l2_gateway_router", "l2_token", "to", "amount"],
        optional_inputs: &["gateway_data", "value"],
    },
];

const SPEC: PluginSpec = PluginSpec {
    plugin_id: "arbitrum-bridge-node",
    provider: "arbitrum",
    operations: OPERATIONS,
};

pub async fn handle_request_json(input: &str) -> Result<String, PluginError> {
    handle_bridge_request_json(input, &SPEC).await
}

pub fn failure_response(message: &str) -> String {
    bridge_failure_response(message)
}
