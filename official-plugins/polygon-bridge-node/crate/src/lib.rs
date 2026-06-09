use bridge_action_core::{
    failure_response as bridge_failure_response, handle_request_json as handle_bridge_request_json,
    OperationSpec, PluginError, PluginSpec,
};

const OPERATIONS: &[OperationSpec] = &[
    OperationSpec {
        name: "polygon_prepare_pos_deposit",
        action_kind: "polygon_pos_deposit",
        target_input: "root_chain_manager",
        chain_input: "l1_chain_id",
        required_inputs: &["l1_chain_id", "root_chain_manager", "user", "root_token", "deposit_data"],
        optional_inputs: &["value"],
    },
    OperationSpec {
        name: "polygon_prepare_pos_exit",
        action_kind: "polygon_pos_exit",
        target_input: "root_chain_manager",
        chain_input: "l1_chain_id",
        required_inputs: &["l1_chain_id", "root_chain_manager", "exit_payload"],
        optional_inputs: &["value"],
    },
    OperationSpec {
        name: "polygon_prepare_state_sync",
        action_kind: "polygon_state_sync",
        target_input: "state_sender",
        chain_input: "l1_chain_id",
        required_inputs: &["l1_chain_id", "state_sender", "receiver", "data"],
        optional_inputs: &["value"],
    },
];

const SPEC: PluginSpec = PluginSpec {
    plugin_id: "polygon-bridge-node",
    provider: "polygon-pos",
    operations: OPERATIONS,
};

pub async fn handle_request_json(input: &str) -> Result<String, PluginError> {
    handle_bridge_request_json(input, &SPEC).await
}

pub fn failure_response(message: &str) -> String {
    bridge_failure_response(message)
}
