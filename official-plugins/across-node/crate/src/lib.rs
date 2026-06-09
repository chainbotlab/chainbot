use bridge_node_core::{
    handle_request_json as handle_bridge_request_json, failure_response as bridge_failure_response,
    InputPlacement, MethodSpec, OperationSpec, PluginError, PluginSpec,
};

const OPERATIONS: &[OperationSpec] = &[
    OperationSpec {
        name: "across_get_swap_approval",
        method: MethodSpec::Get,
        default_path: "swap/approval",
        input_placement: InputPlacement::Query,
        output_key: "swap_approval_response",
        result_state: Some("prepared"),
        required_inputs: &[
            "tradeType",
            "amount",
            "inputToken",
            "outputToken",
            "originChainId",
            "destinationChainId",
            "depositor",
        ],
        optional_inputs: &[
            "recipient",
            "integratorId",
            "slippage",
            "refundAddress",
            "refundOnOrigin",
            "appFee",
            "appFeeRecipient",
            "skipOriginTxEstimation",
            "strictTradeType",
            "excludeSources",
            "includeSources",
        ],
    },
    OperationSpec {
        name: "across_get_deposit_status",
        method: MethodSpec::Get,
        default_path: "deposit/status",
        input_placement: InputPlacement::Query,
        output_key: "deposit_status_response",
        result_state: None,
        required_inputs: &["originChainId", "depositId"],
        optional_inputs: &["destinationChainId", "depositTxHash"],
    },
    OperationSpec {
        name: "across_get_available_routes",
        method: MethodSpec::Get,
        default_path: "available-routes",
        input_placement: InputPlacement::Query,
        output_key: "available_routes_response",
        result_state: None,
        required_inputs: &[],
        optional_inputs: &["originChainId", "destinationChainId", "inputToken", "outputToken"],
    },
];

const SPEC: PluginSpec = PluginSpec {
    plugin_id: "across-node",
    provider: "across",
    default_base_url: "https://app.across.to/api",
    operations: OPERATIONS,
    api_key_header: Some("authorization"),
    api_key_secret: Some("api_key"),
};

pub async fn handle_request_json(input: &str) -> Result<String, PluginError> {
    handle_bridge_request_json(input, &SPEC).await
}

pub fn failure_response(message: &str) -> String {
    bridge_failure_response(message)
}
