use bridge_action_core::{
    failure_response as bridge_failure_response, handle_request_json as handle_bridge_request_json,
    OperationSpec, PluginError, PluginSpec,
};

const OPERATIONS: &[OperationSpec] = &[
    OperationSpec {
        name: "wormhole_prepare_ntt_transfer",
        action_kind: "ntt_transfer",
        target_input: "ntt_manager",
        chain_input: "source_chain_id",
        required_inputs: &["source_chain_id", "ntt_manager", "amount", "recipient", "recipient_chain"],
        optional_inputs: &["refund_address", "queue", "transceiver_instructions", "value"],
    },
    OperationSpec {
        name: "wormhole_prepare_message",
        action_kind: "wormhole_message",
        target_input: "core_bridge",
        chain_input: "source_chain_id",
        required_inputs: &["source_chain_id", "core_bridge", "payload"],
        optional_inputs: &["nonce", "consistency_level", "value"],
    },
];

const SPEC: PluginSpec = PluginSpec {
    plugin_id: "wormhole-node",
    provider: "wormhole",
    operations: OPERATIONS,
};

pub async fn handle_request_json(input: &str) -> Result<String, PluginError> {
    handle_bridge_request_json(input, &SPEC).await
}

pub fn failure_response(message: &str) -> String {
    bridge_failure_response(message)
}
