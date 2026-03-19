use std::sync::Arc;

use crate::builtins::nodes::context::BuiltinRuntimeContext;
use crate::builtins::nodes::contract::BuiltinNodeRegistry;

use super::handlers::{
    emit_subflow_output::EmitSubflowOutputHandler, external_node::ExternalNodeHandler,
    http::HttpHandler, identity::IdentityHandler, script::ScriptHandler,
};

pub(crate) const BUILTIN_IDENTITY_KIND: &str = "builtin.identity";
pub(crate) const BUILTIN_EMIT_SUBFLOW_OUTPUT_KIND: &str = "builtin.emit_subflow_output";
pub(crate) const BUILTIN_EXTERNAL_NODE_KIND: &str = "builtin.external_node";
pub(crate) const BUILTIN_SCRIPT_KIND: &str = "builtin.script";
pub(crate) const BUILTIN_HTTP_KIND: &str = "builtin.http";

pub fn build_builtin_registry(context: BuiltinRuntimeContext) -> BuiltinNodeRegistry {
    let context = Arc::new(context);
    let mut registry = BuiltinNodeRegistry::new();

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
}
