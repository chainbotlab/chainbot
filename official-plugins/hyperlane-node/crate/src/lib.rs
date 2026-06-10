use bridge_action_core::{
    failure_response as bridge_failure_response, handle_request_json as handle_bridge_request_json,
    OperationSpec, PluginError, PluginSpec,
};

const OPERATIONS: &[OperationSpec] = &[
    OperationSpec {
        name: "hyperlane_prepare_warp_transfer",
        action_kind: "warp_route_transfer",
        target_input: "warp_router",
        chain_input: "source_chain_id",
        required_inputs: &["source_chain_id", "warp_router", "destination_domain", "recipient", "amount"],
        optional_inputs: &["hookMetadata", "interchainGasPayment", "value"],
    },
    OperationSpec {
        name: "hyperlane_prepare_message",
        action_kind: "mailbox_dispatch",
        target_input: "mailbox",
        chain_input: "source_chain_id",
        required_inputs: &["source_chain_id", "mailbox", "destination_domain", "recipient", "messageBody"],
        optional_inputs: &["hookMetadata", "value"],
    },
];

const SPEC: PluginSpec = PluginSpec {
    plugin_id: "hyperlane-node",
    provider: "hyperlane",
    operations: OPERATIONS,
};

pub async fn handle_request_json(input: &str) -> Result<String, PluginError> {
    handle_bridge_request_json(input, &SPEC).await
}

pub fn failure_response(message: &str) -> String {
    bridge_failure_response(message)
}
