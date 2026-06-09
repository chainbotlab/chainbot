use bridge_node_core::{
    handle_request_json as handle_bridge_request_json, failure_response as bridge_failure_response,
    InputPlacement, MethodSpec, OperationSpec, PluginError, PluginSpec,
};

const OPERATIONS: &[OperationSpec] = &[
    OperationSpec {
        name: "rango_best_route",
        method: MethodSpec::Get,
        default_path: "basic/quote",
        input_placement: InputPlacement::Query,
        output_key: "best_route_response",
        result_state: None,
        required_inputs: &["from", "to", "amount"],
        optional_inputs: &[
            "fromAddress",
            "toAddress",
            "slippage",
            "disableEstimate",
            "swappers",
            "excludeSwappers",
            "referrerAddress",
            "referrerFee",
        ],
    },
    OperationSpec {
        name: "rango_create_transaction",
        method: MethodSpec::Get,
        default_path: "tx/create",
        input_placement: InputPlacement::Query,
        output_key: "transaction_response",
        result_state: Some("prepared"),
        required_inputs: &["requestId", "step"],
        optional_inputs: &["userSettings", "validationStatus", "wallets"],
    },
    OperationSpec {
        name: "rango_transaction_status",
        method: MethodSpec::Get,
        default_path: "tx/status",
        input_placement: InputPlacement::Query,
        output_key: "status_response",
        result_state: None,
        required_inputs: &["requestId", "txId"],
        optional_inputs: &["step"],
    },
];

const SPEC: PluginSpec = PluginSpec {
    plugin_id: "rango-node",
    provider: "rango",
    default_base_url: "https://public-api.rango.exchange",
    operations: OPERATIONS,
    api_key_header: Some("api-key"),
    api_key_secret: Some("api_key"),
};

pub async fn handle_request_json(input: &str) -> Result<String, PluginError> {
    handle_bridge_request_json(input, &SPEC).await
}

pub fn failure_response(message: &str) -> String {
    bridge_failure_response(message)
}
