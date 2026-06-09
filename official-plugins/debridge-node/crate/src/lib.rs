use bridge_node_core::{
    handle_request_json as handle_bridge_request_json, failure_response as bridge_failure_response,
    InputPlacement, MethodSpec, OperationSpec, PluginError, PluginSpec,
};

const OPERATIONS: &[OperationSpec] = &[
    OperationSpec {
        name: "debridge_create_order_tx",
        method: MethodSpec::Get,
        default_path: "v1.0/dln/order/create-tx",
        input_placement: InputPlacement::Query,
        output_key: "create_tx_response",
        result_state: Some("prepared"),
        required_inputs: &[
            "srcChainId",
            "srcChainTokenIn",
            "srcChainTokenInAmount",
            "dstChainId",
            "dstChainTokenOut",
            "dstChainTokenOutAmount",
        ],
        optional_inputs: &[
            "dstChainTokenOutRecipient",
            "srcChainOrderAuthorityAddress",
            "dstChainOrderAuthorityAddress",
            "affiliateFeePercent",
            "affiliateFeeRecipient",
            "prependOperatingExpenses",
            "referralCode",
            "allowedTaker",
        ],
    },
    OperationSpec {
        name: "debridge_get_order_status",
        method: MethodSpec::Get,
        default_path: "v1.0/dln/order",
        input_placement: InputPlacement::Query,
        output_key: "order_status_response",
        result_state: None,
        required_inputs: &["orderId"],
        optional_inputs: &[],
    },
];

const SPEC: PluginSpec = PluginSpec {
    plugin_id: "debridge-node",
    provider: "debridge",
    default_base_url: "https://dln.debridge.finance",
    operations: OPERATIONS,
    api_key_header: None,
    api_key_secret: None,
};

pub async fn handle_request_json(input: &str) -> Result<String, PluginError> {
    handle_bridge_request_json(input, &SPEC).await
}

pub fn failure_response(message: &str) -> String {
    bridge_failure_response(message)
}
