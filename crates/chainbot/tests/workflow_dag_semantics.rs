//! [INPUT]
//! Workflow fixtures with node dependencies, runtime variable layers, and subflow mappings.
//!
//! [OUTPUT]
//! Verifies DAG validation, namespace precedence, and subflow boundary enforcement.
//!
//! [ROLE]
//! Covers workflow semantic rules before execution-plane scheduling begins.


use std::collections::BTreeMap;
use std::path::PathBuf;

use chainbot::errors::ContractError;
use chainbot::executor::NodeDefinition;
use chainbot::workflow::{
    DependsMode, RuntimeVariableLayers, RuntimeVariableNamespace, RuntimeVariableNamespaces,
    RuntimeVariableSource, SubflowContract, SubflowExport, SubflowImport, VariableBinding,
    VariableReference, WhenCondition, WhenOperator, WorkflowDefinition,
};
use serde_json::json;

#[test]
fn dag_cycle_validation() {
    let valid = WorkflowDefinition {
        api_version: "2.0.0".to_owned(),
        workflow_id: "wf-dag-ok".to_owned(),
        name: "dag-ok".to_owned(),
        runtime: RuntimeVariableLayers::default(),
        nodes: vec![
            node("node-a", vec![]),
            node("node-b", vec!["node-a"]),
            node("node-c", vec!["node-b"]),
        ],
        package_root: PathBuf::new(),
    };
    valid.validate().expect("valid DAG should pass");

    let cyclic = WorkflowDefinition {
        api_version: "2.0.0".to_owned(),
        workflow_id: "wf-dag-cycle".to_owned(),
        name: "dag-cycle".to_owned(),
        runtime: RuntimeVariableLayers::default(),
        nodes: vec![
            node("node-a", vec!["node-c"]),
            node("node-b", vec!["node-a"]),
            node("node-c", vec!["node-b"]),
        ],
        package_root: PathBuf::new(),
    };

    let error = cyclic.validate().expect_err("cyclic DAG must be rejected");
    assert!(matches!(
        error,
        ContractError::DagCycleDetected {
            workflow_id,
            node_ids
        } if workflow_id == "wf-dag-cycle" && node_ids == vec!["node-a", "node-b", "node-c"]
    ));
}

#[test]
fn runtime_variable_precedence() {
    let layers = RuntimeVariableLayers {
        cli_args: BTreeMap::from_iter([
            ("region".to_owned(), json!("eu")),
            ("cli_only".to_owned(), json!(true)),
        ]),
        manual_invocation_input: BTreeMap::from_iter([
            ("region".to_owned(), json!("us")),
            ("manual_only".to_owned(), json!(1)),
        ]),
        trigger_payload_mapping: BTreeMap::from_iter([
            ("region".to_owned(), json!("ap")),
            ("trigger_only".to_owned(), json!(2)),
        ]),
        workflow_defaults: BTreeMap::from_iter([
            ("region".to_owned(), json!("workflow")),
            ("workflow_only".to_owned(), json!(3)),
        ]),
        config_defaults: BTreeMap::from_iter([
            ("region".to_owned(), json!("config")),
            ("config_only".to_owned(), json!(4)),
        ]),
    };

    let resolved = layers.resolve();
    assert_eq!(
        resolved.get("region").expect("region should resolve").value,
        json!("eu")
    );
    assert_eq!(
        resolved
            .get("region")
            .expect("region should resolve")
            .source,
        RuntimeVariableSource::CliArgs
    );
    assert_eq!(
        resolved
            .get("manual_only")
            .expect("manual_only should resolve")
            .source,
        RuntimeVariableSource::ManualInvocationInput
    );
    assert_eq!(
        resolved
            .get("trigger_only")
            .expect("trigger_only should resolve")
            .source,
        RuntimeVariableSource::TriggerPayloadMapping
    );
    assert_eq!(
        resolved
            .get("workflow_only")
            .expect("workflow_only should resolve")
            .source,
        RuntimeVariableSource::WorkflowDefaults
    );
    assert_eq!(
        resolved
            .get("config_only")
            .expect("config_only should resolve")
            .source,
        RuntimeVariableSource::ConfigDefaults
    );
}

#[test]
fn subflow_contract_boundaries() {
    let subflow = SubflowContract {
        workflow_id: "wf-child".to_owned(),
        imports: vec![
            SubflowImport {
                child_key: "ticker".to_owned(),
                source: VariableReference {
                    namespace: RuntimeVariableNamespace::TriggerPayloadMapping,
                    key: "symbol".to_owned(),
                },
            },
            SubflowImport {
                child_key: "dry_run".to_owned(),
                source: VariableReference {
                    namespace: RuntimeVariableNamespace::ManualInvocationInput,
                    key: "dry_run".to_owned(),
                },
            },
        ],
        exports: vec![SubflowExport {
            child_key: "decision".to_owned(),
            parent_key: "subflow_decision".to_owned(),
        }],
    };

    let namespaces = RuntimeVariableNamespaces {
        trigger_payload_mapping: BTreeMap::from_iter([
            ("symbol".to_owned(), json!("BTCUSDT")),
            ("ignored".to_owned(), json!("hidden")),
        ]),
        manual_invocation_input: BTreeMap::from_iter([
            ("dry_run".to_owned(), json!(true)),
            ("admin".to_owned(), json!("root")),
        ]),
        ..RuntimeVariableNamespaces::default()
    };

    let child_inputs = subflow.build_child_inputs(&namespaces);
    assert_eq!(child_inputs.len(), 2);
    assert_eq!(child_inputs.get("ticker"), Some(&json!("BTCUSDT")));
    assert_eq!(child_inputs.get("dry_run"), Some(&json!(true)));
    assert!(!child_inputs.contains_key("ignored"));
    assert!(!child_inputs.contains_key("admin"));

    let child_outputs = BTreeMap::from_iter([
        ("decision".to_owned(), json!("buy")),
        ("internal_state".to_owned(), json!("secret")),
    ]);
    let exports = subflow.collect_exports(&child_outputs);
    assert_eq!(
        exports,
        BTreeMap::from_iter([("subflow_decision".to_owned(), json!("buy"))])
    );

    let workflow = WorkflowDefinition {
        api_version: "2.0.0".to_owned(),
        workflow_id: "wf-subflow".to_owned(),
        name: "subflow".to_owned(),
        runtime: RuntimeVariableLayers::default(),
        nodes: vec![NodeDefinition {
            api_version: "2.0.0".to_owned(),
            node_id: "subflow-node".to_owned(),
            kind: "subflow".to_owned(),
            plugin_id: "builtin-subflow".to_owned(),
            operation: "run".to_owned(),
            depends_mode: DependsMode::All,
            depends_on: vec![],
            inputs: vec![VariableBinding {
                target: "ticker".to_owned(),
                source: VariableReference {
                    namespace: RuntimeVariableNamespace::TriggerPayloadMapping,
                    key: "symbol".to_owned(),
                },
            }],
            when: Some(WhenCondition {
                source: VariableReference {
                    namespace: RuntimeVariableNamespace::ManualInvocationInput,
                    key: "enabled".to_owned(),
                },
                operator: WhenOperator::Truthy,
                expected: None,
            }),
            subflow: Some(subflow),
        }],
        package_root: PathBuf::new(),
    };
    workflow
        .validate()
        .expect("subflow boundaries should validate");
}

#[test]
fn invalid_dag_and_variable_fixtures_rejected() {
    let missing_dependency = WorkflowDefinition {
        api_version: "2.0.0".to_owned(),
        workflow_id: "wf-missing-dep".to_owned(),
        name: "missing-dep".to_owned(),
        runtime: RuntimeVariableLayers::default(),
        nodes: vec![node("node-a", vec!["missing"])],
        package_root: PathBuf::new(),
    };
    let dependency_error = missing_dependency
        .validate()
        .expect_err("unknown dependency must be rejected");
    assert!(matches!(
        dependency_error,
        ContractError::UnknownNodeDependency {
            workflow_id,
            node_id,
            dependency_id
        } if workflow_id == "wf-missing-dep" && node_id == "node-a" && dependency_id == "missing"
    ));

    let invalid_namespace = WorkflowDefinition {
        api_version: "2.0.0".to_owned(),
        workflow_id: "wf-invalid-namespace".to_owned(),
        name: "invalid-namespace".to_owned(),
        runtime: RuntimeVariableLayers::default(),
        nodes: vec![NodeDefinition {
            api_version: "2.0.0".to_owned(),
            node_id: "subflow-node".to_owned(),
            kind: "subflow".to_owned(),
            plugin_id: "builtin-subflow".to_owned(),
            operation: "run".to_owned(),
            depends_mode: DependsMode::All,
            depends_on: vec![],
            inputs: vec![],
            when: None,
            subflow: Some(SubflowContract {
                workflow_id: "wf-child".to_owned(),
                imports: vec![SubflowImport {
                    child_key: "illegal".to_owned(),
                    source: VariableReference {
                        namespace: RuntimeVariableNamespace::SubflowOutput,
                        key: "x".to_owned(),
                    },
                }],
                exports: vec![],
            }),
        }],
        package_root: PathBuf::new(),
    };
    let namespace_error = invalid_namespace
        .validate()
        .expect_err("invalid namespace references must be rejected");
    assert!(matches!(
        namespace_error,
        ContractError::InvalidVariableReference {
            workflow_id,
            node_id,
            context: "subflow.imports",
            namespace,
            key
        } if workflow_id == "wf-invalid-namespace" && node_id == "subflow-node" && namespace == "subflow_output" && key == "x"
    ));
}

fn node(node_id: &str, depends_on: Vec<&str>) -> NodeDefinition {
    NodeDefinition {
        api_version: "2.0.0".to_owned(),
        node_id: node_id.to_owned(),
        kind: "plugin".to_owned(),
        plugin_id: "quote-plugin".to_owned(),
        operation: "normalize".to_owned(),
        depends_mode: DependsMode::All,
        depends_on: depends_on.into_iter().map(str::to_owned).collect(),
        inputs: vec![],
        when: None,
        subflow: None,
    }
}
