use std::sync::Arc;

use crate::builtins::nodes::context::BuiltinRuntimeContext;
use crate::builtins::nodes::contract::BuiltinNodeRegistry;

use super::handlers::{
    assert::AssertHandler, data_coalesce::DataCoalesceHandler, data_compare::DataCompareHandler,
    data_get::DataGetHandler, data_math::DataMathHandler, data_merge::DataMergeHandler,
    data_parse_json::DataParseJsonHandler, data_pick::DataPickHandler,
    data_stringify_json::DataStringifyJsonHandler, data_template::DataTemplateHandler,
    emit_subflow_output::EmitSubflowOutputHandler, external_node::ExternalNodeHandler,
    fail::FailHandler, http::HttpHandler, identity::IdentityHandler, script::ScriptHandler,
};

pub(crate) const BUILTIN_ASSERT_KIND: &str = "builtin.flow.assert";
pub(crate) const BUILTIN_FAIL_KIND: &str = "builtin.flow.fail";
pub(crate) const BUILTIN_DATA_PICK_KIND: &str = "builtin.data.pick";
pub(crate) const BUILTIN_DATA_MERGE_KIND: &str = "builtin.data.merge";
pub(crate) const BUILTIN_DATA_TEMPLATE_KIND: &str = "builtin.data.template";
pub(crate) const BUILTIN_DATA_GET_KIND: &str = "builtin.data.get";
pub(crate) const BUILTIN_DATA_COALESCE_KIND: &str = "builtin.data.coalesce";
pub(crate) const BUILTIN_DATA_COMPARE_KIND: &str = "builtin.data.compare";
pub(crate) const BUILTIN_DATA_PARSE_JSON_KIND: &str = "builtin.data.parse_json";
pub(crate) const BUILTIN_DATA_STRINGIFY_JSON_KIND: &str = "builtin.data.stringify_json";
pub(crate) const BUILTIN_DATA_MATH_KIND: &str = "builtin.data.math";
pub(crate) const BUILTIN_IDENTITY_KIND: &str = "builtin.identity";
pub(crate) const BUILTIN_EMIT_SUBFLOW_OUTPUT_KIND: &str = "builtin.emit_subflow_output";
pub(crate) const BUILTIN_EXTERNAL_NODE_KIND: &str = "builtin.external_node";
pub(crate) const BUILTIN_SCRIPT_KIND: &str = "builtin.script";
pub(crate) const BUILTIN_HTTP_KIND: &str = "builtin.http";

pub fn build_builtin_registry(context: BuiltinRuntimeContext) -> BuiltinNodeRegistry {
    let context = Arc::new(context);
    let mut registry = BuiltinNodeRegistry::new();

    registry.register_handler(AssertHandler);
    registry.register_handler(FailHandler);
    registry.register_handler(DataPickHandler);
    registry.register_handler(DataMergeHandler);
    registry.register_handler(DataTemplateHandler);
    registry.register_handler(DataGetHandler);
    registry.register_handler(DataCoalesceHandler);
    registry.register_handler(DataCompareHandler);
    registry.register_handler(DataParseJsonHandler);
    registry.register_handler(DataStringifyJsonHandler);
    registry.register_handler(DataMathHandler);
    registry.register_handler(IdentityHandler);
    registry.register_handler(EmitSubflowOutputHandler);
    registry.register_handler(ExternalNodeHandler::new(Arc::clone(&context)));
    registry.register_handler(ScriptHandler::new(Arc::clone(&context)));
    registry.register_handler(HttpHandler::new(Arc::clone(&context)));

    registry
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use crate::builtins::nodes::contract::BuiltinNodeRequest;
    use crate::builtins::nodes::script_worker::{WorkerHost, WorkerHostLimits};
    use crate::executor::NodeDefinition;
    use crate::plugin::PluginManifest;
    use serde_json::json;

    use super::*;

    fn test_context() -> BuiltinRuntimeContext {
        BuiltinRuntimeContext {
            root_layout: crate::config::RootLayout::from_root(PathBuf::from(
                "/tmp/chainbot-builtins-test",
            )),
            manifests: BTreeMap::<String, PluginManifest>::new(),
            secret_mode: crate::builtins::nodes::context::SecretDecryptMode::Plaintext,
            worker_host: WorkerHost::new(WorkerHostLimits::default()),
        }
    }

    #[test]
    fn build_builtin_registry_keeps_default_and_extended_handlers() {
        let registry = build_builtin_registry(test_context());
        let request = crate::builtins::nodes::contract::BuiltinNodeRequest {
            run_id: "run-registry".to_owned(),
            workflow_id: "wf-registry".to_owned(),
            workflow_package_root: PathBuf::new(),
            node_id: "node-registry".to_owned(),
            operation: "run".to_owned(),
            inputs: BTreeMap::from([(String::from("symbol"), serde_json::json!("ETHUSDT"))]),
            runtime_namespaces: Default::default(),
        };

        let identity = registry
            .dispatch(BUILTIN_IDENTITY_KIND, &request)
            .expect("default builtin.identity should remain registered");
        assert_eq!(
            identity.outputs.get("symbol"),
            Some(&serde_json::json!("ETHUSDT"))
        );

        let http_error = registry
            .dispatch(
                BUILTIN_HTTP_KIND,
                &BuiltinNodeRequest {
                    operation: String::new(),
                    ..request
                },
            )
            .expect_err("builtin.http handler should be registered and validate its input");
        assert!(http_error
            .to_string()
            .contains("HTTP node requires a URL in operation"));
    }

    #[test]
    fn builtin_dispatch_kind_keeps_builtin_alias_resolution() {
        let builtin_alias = NodeDefinition {
            api_version: "2.0.0".to_owned(),
            node_id: "node".to_owned(),
            kind: "builtin".to_owned(),
            plugin_id: BUILTIN_HTTP_KIND.to_owned(),
            operation: "https://example.com".to_owned(),
            depends_mode: Default::default(),
            depends_on: Vec::new(),
            inputs: Vec::new(),
            when: None,
            subflow: None,
        };
        assert_eq!(
            super::super::dispatch::builtin_dispatch_kind(&builtin_alias),
            Some(BUILTIN_HTTP_KIND)
        );
    }

    #[test]
    fn build_builtin_registry_dispatches_first_wave_core_nodes() {
        let registry = build_builtin_registry(test_context());
        let request = BuiltinNodeRequest {
            run_id: "run-core".to_owned(),
            workflow_id: "wf-core".to_owned(),
            workflow_package_root: PathBuf::new(),
            node_id: "node-core".to_owned(),
            operation: "truthy".to_owned(),
            inputs: BTreeMap::from([(String::from("value"), json!("ready"))]),
            runtime_namespaces: Default::default(),
        };

        let assert_result = registry
            .dispatch(BUILTIN_ASSERT_KIND, &request)
            .expect("builtin.flow.assert should be registered");
        assert_eq!(assert_result.outputs.get("ok"), Some(&json!(true)));

        let pick_result = registry
            .dispatch(
                BUILTIN_DATA_PICK_KIND,
                &BuiltinNodeRequest {
                    operation: "fields".to_owned(),
                    inputs: BTreeMap::from([
                        (
                            String::from("input"),
                            json!({"symbol": "ETHUSDT", "price": 3000}),
                        ),
                        (String::from("fields"), json!(["symbol"])),
                    ]),
                    ..request.clone()
                },
            )
            .expect("builtin.data.pick should be registered");
        assert_eq!(
            pick_result.outputs.get("result"),
            Some(&json!({"symbol": "ETHUSDT"}))
        );

        let merge_result = registry
            .dispatch(
                BUILTIN_DATA_MERGE_KIND,
                &BuiltinNodeRequest {
                    operation: "objects".to_owned(),
                    inputs: BTreeMap::from([(
                        String::from("objects"),
                        json!([
                            {"symbol": "ETHUSDT"},
                            {"price": 3000},
                            {"price": 3100, "venue": "demo"}
                        ]),
                    )]),
                    ..request.clone()
                },
            )
            .expect("builtin.data.merge should be registered");
        assert_eq!(
            merge_result.outputs.get("result"),
            Some(&json!({"symbol": "ETHUSDT", "price": 3100, "venue": "demo"}))
        );

        let template_result = registry
            .dispatch(
                BUILTIN_DATA_TEMPLATE_KIND,
                &BuiltinNodeRequest {
                    operation: "render".to_owned(),
                    inputs: BTreeMap::from([
                        (
                            String::from("template"),
                            json!("pair={{symbol}} price={{price}}"),
                        ),
                        (
                            String::from("values"),
                            json!({"symbol": "ETHUSDT", "price": 3000}),
                        ),
                    ]),
                    ..request.clone()
                },
            )
            .expect("builtin.data.template should be registered");
        assert_eq!(
            template_result.outputs.get("result"),
            Some(&json!("pair=ETHUSDT price=3000"))
        );

        let get_result = registry
            .dispatch(
                BUILTIN_DATA_GET_KIND,
                &BuiltinNodeRequest {
                    operation: "path".to_owned(),
                    inputs: BTreeMap::from([
                        (
                            String::from("input"),
                            json!({"quote": {"symbol": "ETHUSDT"}}),
                        ),
                        (String::from("path"), json!("quote.symbol")),
                    ]),
                    ..request.clone()
                },
            )
            .expect("builtin.data.get should be registered");
        assert_eq!(get_result.outputs.get("result"), Some(&json!("ETHUSDT")));

        let coalesce_result = registry
            .dispatch(
                BUILTIN_DATA_COALESCE_KIND,
                &BuiltinNodeRequest {
                    operation: "first".to_owned(),
                    inputs: BTreeMap::from([(
                        String::from("values"),
                        json!([null, "fallback", "later"]),
                    )]),
                    ..request.clone()
                },
            )
            .expect("builtin.data.coalesce should be registered");
        assert_eq!(
            coalesce_result.outputs.get("result"),
            Some(&json!("fallback"))
        );

        let compare_result = registry
            .dispatch(
                BUILTIN_DATA_COMPARE_KIND,
                &BuiltinNodeRequest {
                    operation: "greater_than".to_owned(),
                    inputs: BTreeMap::from([
                        (String::from("left"), json!(12)),
                        (String::from("right"), json!(2.5)),
                    ]),
                    ..request.clone()
                },
            )
            .expect("builtin.data.compare should be registered");
        assert_eq!(compare_result.outputs.get("result"), Some(&json!(true)));

        let parse_json_result = registry
            .dispatch(
                BUILTIN_DATA_PARSE_JSON_KIND,
                &BuiltinNodeRequest {
                    operation: "parse".to_owned(),
                    inputs: BTreeMap::from([(
                        String::from("text"),
                        json!("{\"symbol\":\"ETHUSDT\",\"price\":3000}"),
                    )]),
                    ..request.clone()
                },
            )
            .expect("builtin.data.parse_json should be registered");
        assert_eq!(
            parse_json_result.outputs.get("result"),
            Some(&json!({"symbol": "ETHUSDT", "price": 3000}))
        );

        let stringify_json_result = registry
            .dispatch(
                BUILTIN_DATA_STRINGIFY_JSON_KIND,
                &BuiltinNodeRequest {
                    operation: "stringify".to_owned(),
                    inputs: BTreeMap::from([(
                        String::from("value"),
                        json!({"symbol": "ETHUSDT", "price": 3000}),
                    )]),
                    ..request.clone()
                },
            )
            .expect("builtin.data.stringify_json should be registered");
        assert_eq!(
            stringify_json_result.outputs.get("result"),
            Some(&json!("{\"price\":3000,\"symbol\":\"ETHUSDT\"}"))
        );

        let math_result = registry
            .dispatch(
                BUILTIN_DATA_MATH_KIND,
                &BuiltinNodeRequest {
                    operation: "multiply".to_owned(),
                    inputs: BTreeMap::from([
                        (String::from("left"), json!(12)),
                        (String::from("right"), json!(2.5)),
                    ]),
                    ..request.clone()
                },
            )
            .expect("builtin.data.math should be registered");
        assert_eq!(math_result.outputs.get("result"), Some(&json!(30.0)));
    }
}
