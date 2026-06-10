use bridge_action_core::{
    failure_response as bridge_failure_response, handle_request_json as handle_bridge_request_json,
    OperationSpec, PluginError, PluginSpec,
};

const OPERATIONS: &[OperationSpec] = &[
    OperationSpec {
        name: "layerzero_prepare_oft_send",
        action_kind: "oft_send",
        target_input: "oft",
        chain_input: "source_chain_id",
        required_inputs: &["source_chain_id", "oft", "dstEid", "to", "amountLD", "minAmountLD", "nativeFee", "lzTokenFee", "refundAddress"],
        optional_inputs: &["extraOptions", "composeMsg", "oftCmd", "value"],
    },
    OperationSpec {
        name: "layerzero_prepare_set_peer",
        action_kind: "oapp_set_peer",
        target_input: "oapp",
        chain_input: "chain_id",
        required_inputs: &["chain_id", "oapp", "eid", "peer"],
        optional_inputs: &[],
    },
];

const SPEC: PluginSpec = PluginSpec {
    plugin_id: "layerzero-node",
    provider: "layerzero",
    operations: OPERATIONS,
};

pub async fn handle_request_json(input: &str) -> Result<String, PluginError> {
    handle_bridge_request_json(input, &SPEC).await
}

pub fn failure_response(message: &str) -> String {
    bridge_failure_response(message)
}
