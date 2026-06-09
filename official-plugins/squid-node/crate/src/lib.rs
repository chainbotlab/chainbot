use bridge_node_core::{
    handle_request_json as handle_bridge_request_json, failure_response as bridge_failure_response,
    InputPlacement, MethodSpec, OperationSpec, PluginError, PluginSpec,
};

const OPERATIONS: &[OperationSpec] = &[
    OperationSpec {
        name: "squid_get_route",
        method: MethodSpec::Post,
        default_path: "route",
        input_placement: InputPlacement::Body,
        output_key: "route_response",
        result_state: Some("prepared"),
        required_inputs: &[
            "fromAddress",
            "fromChain",
            "fromToken",
            "fromAmount",
            "toChain",
            "toToken",
            "toAddress",
        ],
        optional_inputs: &["slippage", "quoteOnly", "enableForecall", "prefer", "receiveGasOnDestination"],
    },
    OperationSpec {
        name: "squid_get_status",
        method: MethodSpec::Get,
        default_path: "status",
        input_placement: InputPlacement::Query,
        output_key: "status_response",
        result_state: None,
        required_inputs: &["transactionId", "requestId", "fromChainId", "toChainId"],
        optional_inputs: &["quoteId"],
    },
];

const SPEC: PluginSpec = PluginSpec {
    plugin_id: "squid-node",
    provider: "squid",
    default_base_url: "https://v2.api.squidrouter.com/v2",
    operations: OPERATIONS,
    api_key_header: Some("x-integrator-id"),
    api_key_secret: Some("integrator_id"),
};

pub async fn handle_request_json(input: &str) -> Result<String, PluginError> {
    handle_bridge_request_json(input, &SPEC).await
}

pub fn failure_response(message: &str) -> String {
    bridge_failure_response(message)
}
