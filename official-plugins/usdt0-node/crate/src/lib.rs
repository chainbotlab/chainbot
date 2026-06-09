use bridge_action_core::{
    failure_response as bridge_failure_response, handle_request_json as handle_bridge_request_json,
    OperationSpec, PluginError, PluginSpec,
};

const OPERATIONS: &[OperationSpec] = &[
    OperationSpec {
        name: "usdt0_prepare_oft_send",
        action_kind: "oft_send",
        target_input: "oft",
        chain_input: "source_chain_id",
        required_inputs: &["source_chain_id", "oft", "dstEid", "to", "amountLD", "minAmountLD", "nativeFee", "lzTokenFee", "refundAddress"],
        optional_inputs: &["asset", "extraOptions", "composeMsg", "oftCmd", "value"],
    },
    OperationSpec {
        name: "usdt0_prepare_quote_send",
        action_kind: "oft_quote_send",
        target_input: "oft",
        chain_input: "source_chain_id",
        required_inputs: &["source_chain_id", "oft", "dstEid", "to", "amountLD", "minAmountLD"],
        optional_inputs: &["asset", "extraOptions", "composeMsg", "oftCmd", "payInLzToken"],
    },
];

const SPEC: PluginSpec = PluginSpec {
    plugin_id: "usdt0-node",
    provider: "usdt0",
    operations: OPERATIONS,
};

pub async fn handle_request_json(input: &str) -> Result<String, PluginError> {
    handle_bridge_request_json(input, &SPEC).await
}

pub fn failure_response(message: &str) -> String {
    bridge_failure_response(message)
}
