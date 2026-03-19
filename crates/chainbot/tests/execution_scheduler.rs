//! [INPUT]
//! Workflow fixtures, normalized run requests, builtin node handlers, and scheduler-ready DAG edges.
//!
//! [OUTPUT]
//! Verifies scheduler orchestration, dependency semantics, conditional execution, and builtin node dispatch failures.
//!
//! [ROLE]
//! Covers the execution scheduler boundary as an integration test.

use std::collections::BTreeMap;
use std::path::PathBuf;

use chainbot::errors::ContractError;
use chainbot::executor::{
    BuiltinNodeRegistry, BuiltinNodeRequest, ExecutionPlane, NodeDefinition, NormalizedRunRequest,
    ScheduledNodeState, WorkflowRunStatus,
};
use chainbot::workflow::{
    DependsMode, RuntimeVariableLayers, RuntimeVariableNamespace, VariableBinding,
    VariableReference, WhenCondition, WhenOperator, WorkflowDefinition,
};
use serde_json::json;

#[test]
fn scheduler_parallel_ready_nodes() {
    let workflow = WorkflowDefinition {
        api_version: "2.0.0".to_owned(),
        workflow_id: "wf-parallel".to_owned(),
        name: "parallel".to_owned(),
        runtime: RuntimeVariableLayers::default(),
        nodes: vec![
            NodeDefinition {
                api_version: "2.0.0".to_owned(),
                node_id: "start".to_owned(),
                kind: "builtin.identity".to_owned(),
                plugin_id: "builtin.identity".to_owned(),
                operation: "run".to_owned(),
                depends_mode: DependsMode::All,
                depends_on: vec![],
                inputs: vec![VariableBinding {
                    target: "seed_value".to_owned(),
                    source: VariableReference {
                        namespace: RuntimeVariableNamespace::RunScoped,
                        key: "seed".to_owned(),
                    },
                }],
                when: None,
                subflow: None,
            },
            builtin_node("left", "builtin.identity", vec!["start"], DependsMode::All),
            builtin_node("right", "builtin.identity", vec!["start"], DependsMode::All),
            builtin_node(
                "join",
                "builtin.identity",
                vec!["left", "right"],
                DependsMode::All,
            ),
        ],
        package_root: PathBuf::new(),
    };

    let execution_plane = ExecutionPlane::new(
        vec![workflow],
        BTreeMap::new(),
        BuiltinNodeRegistry::with_test_handlers(),
    )
    .expect("plane");
    let mut request = NormalizedRunRequest::new("run-parallel", "wf-parallel");
    request.cli_args.insert("seed".to_owned(), json!("BTCUSDT"));

    let report = execution_plane
        .execute(&request)
        .expect("parallel workflow should execute");

    assert_eq!(report.status, WorkflowRunStatus::Succeeded);
    assert_eq!(
        report.schedule_waves,
        vec![
            vec!["start".to_owned()],
            vec!["left".to_owned(), "right".to_owned()],
            vec!["join".to_owned()],
        ]
    );
    assert_eq!(
        report.node_states.get("left"),
        Some(&ScheduledNodeState::Succeeded)
    );
    assert_eq!(
        report.node_states.get("right"),
        Some(&ScheduledNodeState::Succeeded)
    );
    assert_eq!(
        report.node_states.get("join"),
        Some(&ScheduledNodeState::Succeeded)
    );
}

#[test]
fn scheduler_when_and_depends_mode() {
    let mut registry = BuiltinNodeRegistry::with_test_handlers();
    registry.register("builtin.fail", |_request| {
        Err(ContractError::CliUsage {
            message: "forced node failure".to_owned(),
        })
    });

    let workflow = WorkflowDefinition {
        api_version: "2.0.0".to_owned(),
        workflow_id: "wf-when-depends".to_owned(),
        name: "when-depends".to_owned(),
        runtime: RuntimeVariableLayers::default(),
        nodes: vec![
            builtin_node("a-fail", "builtin.fail", vec![], DependsMode::All),
            NodeDefinition {
                api_version: "2.0.0".to_owned(),
                node_id: "b-ok".to_owned(),
                kind: "builtin.identity".to_owned(),
                plugin_id: "builtin.identity".to_owned(),
                operation: "run".to_owned(),
                depends_mode: DependsMode::All,
                depends_on: vec![],
                inputs: vec![VariableBinding {
                    target: "enabled".to_owned(),
                    source: VariableReference {
                        namespace: RuntimeVariableNamespace::RunScoped,
                        key: "enabled".to_owned(),
                    },
                }],
                when: None,
                subflow: None,
            },
            builtin_node(
                "all-after",
                "builtin.identity",
                vec!["a-fail", "b-ok"],
                DependsMode::All,
            ),
            builtin_node(
                "any-after",
                "builtin.identity",
                vec!["a-fail", "b-ok"],
                DependsMode::Any,
            ),
            NodeDefinition {
                api_version: "2.0.0".to_owned(),
                node_id: "gated".to_owned(),
                kind: "builtin.identity".to_owned(),
                plugin_id: "builtin.identity".to_owned(),
                operation: "run".to_owned(),
                depends_mode: DependsMode::All,
                depends_on: vec![],
                inputs: vec![],
                when: Some(WhenCondition {
                    source: VariableReference {
                        namespace: RuntimeVariableNamespace::RunScoped,
                        key: "enabled".to_owned(),
                    },
                    operator: WhenOperator::Truthy,
                    expected: None,
                }),
                subflow: None,
            },
        ],
        package_root: PathBuf::new(),
    };

    let execution_plane =
        ExecutionPlane::new(vec![workflow], BTreeMap::new(), registry).expect("plane");
    let mut request = NormalizedRunRequest::new("run-when-depends", "wf-when-depends");
    request
        .manual_invocation_input
        .insert("enabled".to_owned(), json!(false));

    let report = execution_plane
        .execute(&request)
        .expect("when/depends workflow should complete with bounded failures");

    assert_eq!(report.status, WorkflowRunStatus::Failed);
    assert_eq!(
        report.schedule_waves,
        vec![
            vec!["a-fail".to_owned(), "b-ok".to_owned()],
            vec!["any-after".to_owned()],
        ]
    );
    assert_eq!(
        report.node_states.get("a-fail"),
        Some(&ScheduledNodeState::Failed)
    );
    assert_eq!(
        report.node_states.get("b-ok"),
        Some(&ScheduledNodeState::Succeeded)
    );
    assert_eq!(
        report.node_states.get("all-after"),
        Some(&ScheduledNodeState::Skipped)
    );
    assert_eq!(
        report.node_states.get("any-after"),
        Some(&ScheduledNodeState::Succeeded)
    );
    assert_eq!(
        report.node_states.get("gated"),
        Some(&ScheduledNodeState::Skipped)
    );
    assert_eq!(
        report.node_failures.get("a-fail"),
        Some(&"forced node failure".to_owned())
    );
}

#[test]
fn builtin_node_registry_dispatch() {
    let registry = BuiltinNodeRegistry::with_test_handlers();
    let request = BuiltinNodeRequest {
        run_id: "run-registry".to_owned(),
        workflow_id: "wf-registry".to_owned(),
        workflow_package_root: PathBuf::new(),
        node_id: "node-registry".to_owned(),
        operation: "run".to_owned(),
        inputs: BTreeMap::from_iter([(String::from("symbol"), json!("ETHUSDT"))]),
        runtime_namespaces: Default::default(),
    };

    let result = registry
        .dispatch("builtin.identity", &request)
        .expect("known builtin kind should dispatch");
    assert_eq!(result.outputs.get("symbol"), Some(&json!("ETHUSDT")));

    let error = registry
        .dispatch("builtin.unknown", &request)
        .expect_err("unknown builtin kind should be typed failure");
    assert!(matches!(
        error,
        ContractError::UnknownBuiltinNodeKind {
            workflow_id,
            node_id,
            kind,
        } if workflow_id == "wf-registry" && node_id == "node-registry" && kind == "builtin.unknown"
    ));
}

#[test]
fn builtin_node_registry_test_handlers_extend_with_custom_registration() {
    let mut registry = BuiltinNodeRegistry::with_test_handlers();
    registry.register("builtin.capture_runtime", |request| {
        Ok(chainbot::executor::BuiltinNodeResult {
            outputs: BTreeMap::from([(
                String::from("workflow"),
                json!(request
                    .runtime_namespaces
                    .run_scoped
                    .get("workflow")
                    .cloned()),
            )]),
            ..Default::default()
        })
    });

    let request = BuiltinNodeRequest {
        run_id: "run-registry".to_owned(),
        workflow_id: "wf-registry".to_owned(),
        workflow_package_root: PathBuf::new(),
        node_id: "node-registry".to_owned(),
        operation: "run".to_owned(),
        inputs: BTreeMap::from_iter([(String::from("symbol"), json!("ETHUSDT"))]),
        runtime_namespaces: chainbot::workflow::RuntimeVariableNamespaces {
            run_scoped: BTreeMap::from([(String::from("workflow"), json!("alpha"))]),
            ..Default::default()
        },
    };

    let identity = registry
        .dispatch("builtin.identity", &request)
        .expect("seeded test builtin should remain dispatchable after extension registration");
    assert_eq!(identity.outputs.get("symbol"), Some(&json!("ETHUSDT")));

    let subflow = registry
        .dispatch("builtin.emit_subflow_output", &request)
        .expect("seeded subflow emitter builtin should remain dispatchable");
    assert_eq!(
        subflow.subflow_output.get("symbol"),
        Some(&json!("ETHUSDT"))
    );

    let custom = registry
        .dispatch("builtin.capture_runtime", &request)
        .expect("custom builtin registration should coexist with defaults");
    assert_eq!(custom.outputs.get("workflow"), Some(&json!("alpha")));
}

#[test]
fn builtin_node_registry_test_handlers_only_seed_scheduler_basics() {
    let registry = BuiltinNodeRegistry::with_test_handlers();
    let request = BuiltinNodeRequest {
        run_id: "run-registry".to_owned(),
        workflow_id: "wf-registry".to_owned(),
        workflow_package_root: PathBuf::new(),
        node_id: "node-registry".to_owned(),
        operation: "run".to_owned(),
        inputs: BTreeMap::from_iter([(String::from("symbol"), json!("ETHUSDT"))]),
        runtime_namespaces: Default::default(),
    };

    registry
        .dispatch("builtin.identity", &request)
        .expect("seeded test registry should include builtin.identity");
    registry
        .dispatch("builtin.emit_subflow_output", &request)
        .expect("seeded test registry should include builtin.emit_subflow_output");

    let error = registry
        .dispatch("builtin.http", &request)
        .expect_err("seeded test registry should not imply production-complete builtins");
    assert!(matches!(
        error,
        ContractError::UnknownBuiltinNodeKind { kind, .. } if kind == "builtin.http"
    ));
}

fn builtin_node(
    node_id: &str,
    kind: &str,
    depends_on: Vec<&str>,
    depends_mode: DependsMode,
) -> NodeDefinition {
    NodeDefinition {
        api_version: "2.0.0".to_owned(),
        node_id: node_id.to_owned(),
        kind: kind.to_owned(),
        plugin_id: kind.to_owned(),
        operation: "run".to_owned(),
        depends_mode,
        depends_on: depends_on.into_iter().map(str::to_owned).collect(),
        inputs: vec![],
        when: None,
        subflow: None,
    }
}
