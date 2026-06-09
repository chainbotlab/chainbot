use bridge_action_core::{
    failure_response as bridge_failure_response, handle_request_json as handle_bridge_request_json,
    OperationSpec, PluginError, PluginSpec,
};

const OPERATIONS: &[OperationSpec] = &[
    OperationSpec {
        name: "stargate_prepare_send_token",
        action_kind: "stargate_send_token",
        target_input: "stargate",
        chain_input: "source_chain_id",
        required_inputs: &["source_chain_id", "stargate", "dstEid", "to", "amountLD", "minAmountLD", "nativeFee", "lzTokenFee", "refundAddress"],
        optional_inputs: &["extraOptions", "composeMsg", "oftCmd", "value"],
    },
    OperationSpec {
        name: "stargate_prepare_oft_send",
        action_kind: "oft_send",
        target_input: "oft",
        chain_input: "source_chain_id",
        required_inputs: &["source_chain_id", "oft", "dstEid", "to", "amountLD", "minAmountLD", "nativeFee", "lzTokenFee", "refundAddress"],
        optional_inputs: &["extraOptions", "composeMsg", "oftCmd", "value"],
    },
];

const SPEC: PluginSpec = PluginSpec {
    plugin_id: "stargate-node",
    provider: "stargate",
    operations: OPERATIONS,
};

pub async fn handle_request_json(input: &str) -> Result<String, PluginError> {
    handle_bridge_request_json(input, &SPEC).await
}

pub fn failure_response(message: &str) -> String {
    bridge_failure_response(message)
}
