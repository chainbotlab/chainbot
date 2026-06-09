use bridge_node_core::{
    handle_request_json as handle_bridge_request_json, failure_response as bridge_failure_response,
    InputPlacement, MethodSpec, OperationSpec, PluginError, PluginSpec,
};

const OPERATIONS: &[OperationSpec] = &[
    OperationSpec {
        name: "lifi_get_routes",
        method: MethodSpec::Post,
        default_path: "v1/advanced/routes",
        input_placement: InputPlacement::Body,
        output_key: "routes_response",
        result_state: None,
        required_inputs: &[
            "fromChainId",
            "toChainId",
            "fromTokenAddress",
            "toTokenAddress",
            "fromAmount",
        ],
        optional_inputs: &["fromAddress", "toAddress", "fromAmountForGas", "options"],
    },
    OperationSpec {
        name: "lifi_get_quote",
        method: MethodSpec::Get,
        default_path: "v1/quote",
        input_placement: InputPlacement::Query,
        output_key: "quote_response",
        result_state: Some("prepared"),
        required_inputs: &["fromChain", "toChain", "fromToken", "toToken", "fromAddress"],
        optional_inputs: &[
            "fromAmount",
            "toAmount",
            "toAddress",
            "fromAmountForGas",
            "integrator",
            "fee",
            "slippage",
            "referrer",
            "allowBridges",
            "denyBridges",
            "preferBridges",
            "allowExchanges",
            "denyExchanges",
            "preferExchanges",
            "allowProtocols",
            "denyProtocols",
        ],
    },
    OperationSpec {
        name: "lifi_get_contract_calls_quote",
        method: MethodSpec::Post,
        default_path: "v1/quote/contractCalls",
        input_placement: InputPlacement::Body,
        output_key: "contract_calls_quote_response",
        result_state: Some("prepared"),
        required_inputs: &[
            "fromAddress",
            "fromChain",
            "fromToken",
            "toAmount",
            "toChain",
            "toToken",
            "contractCalls",
        ],
        optional_inputs: &["integrator", "fee", "slippage", "referrer"],
    },
];

const SPEC: PluginSpec = PluginSpec {
    plugin_id: "lifi-node",
    provider: "lifi",
    default_base_url: "https://li.quest",
    operations: OPERATIONS,
    api_key_header: Some("x-lifi-api-key"),
    api_key_secret: Some("api_key"),
};

pub async fn handle_request_json(input: &str) -> Result<String, PluginError> {
    handle_bridge_request_json(input, &SPEC).await
}

pub fn failure_response(message: &str) -> String {
    bridge_failure_response(message)
}
