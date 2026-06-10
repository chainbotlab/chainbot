use bridge_action_core::{
    failure_response as bridge_failure_response, handle_request_json as handle_bridge_request_json,
    OperationSpec, PluginError, PluginSpec,
};

const OPERATIONS: &[OperationSpec] = &[
    OperationSpec {
        name: "axelar_prepare_interchain_transfer",
        action_kind: "its_interchain_transfer",
        target_input: "interchain_token_service",
        chain_input: "source_chain_id",
        required_inputs: &["source_chain_id", "interchain_token_service", "tokenId", "destinationChain", "destinationAddress", "amount"],
        optional_inputs: &["metadata", "gasValue", "value"],
    },
    OperationSpec {
        name: "axelar_prepare_call_contract",
        action_kind: "gmp_call_contract",
        target_input: "gateway",
        chain_input: "source_chain_id",
        required_inputs: &["source_chain_id", "gateway", "destinationChain", "destinationContractAddress", "payload"],
        optional_inputs: &["gasService", "gasValue", "refundAddress", "value"],
    },
];

const SPEC: PluginSpec = PluginSpec {
    plugin_id: "axelar-node",
    provider: "axelar",
    operations: OPERATIONS,
};

pub async fn handle_request_json(input: &str) -> Result<String, PluginError> {
    handle_bridge_request_json(input, &SPEC).await
}

pub fn failure_response(message: &str) -> String {
    bridge_failure_response(message)
}
