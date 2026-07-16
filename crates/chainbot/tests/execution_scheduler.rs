//! [INPUT]
//! Workflow fixtures, normalized run requests, builtin node handlers, and scheduler-ready DAG edges.
//!
//! [OUTPUT]
//! Verifies scheduler orchestration, dependency semantics, conditional execution, and builtin node dispatch failures.
//!
//! [ROLE]
//! Covers the execution scheduler boundary as an integration test.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use chainbot::app::ExecutionPlane;
use chainbot::builtins::nodes::contract::{BuiltinNodeRequest, BuiltinNodeResult};
use chainbot::builtins::nodes::registry_store::BuiltinNodeRegistry;
use chainbot::builtins::nodes::script_worker::{WorkerHost, WorkerHostLimits};
use chainbot::builtins::{build_builtin_registry, BuiltinRuntimeContext, SecretDecryptMode};
use chainbot::domain::runtime::{
    NodeDefinition, NormalizedRunRequest, ScheduledNodeState, WorkflowRunStatus,
};
use chainbot::domain::workflow::{
    DependsMode, RuntimeVariableLayers, RuntimeVariableNamespace, RuntimeVariableNamespaces,
    VariableBinding, VariableReference, WhenCondition, WhenOperator, WorkflowDefinition,
};
use chainbot::errors::ContractError;
use chainbot::infrastructure::config::RootLayout;
use chainbot::plugin::{
    McpPluginContract, McpStdioTransportConfig, McpTransportKind, PluginManifest,
    PluginOperationDescriptor, PluginOperationKind, EXTERNAL_NODE_ENTRYPOINT_EXEC_V1,
    EXTERNAL_NODE_ENTRYPOINT_EXEC_V2, EXTERNAL_NODE_ENTRYPOINT_MCP_TOOL_V1,
    NODE_PLUGIN_EXECUTE_CAPABILITY,
    PLUGIN_KIND_EXTERNAL_NODE,
};
use serde_json::json;

const MCP_RUNTIME_STDIO_FIXTURE_SCRIPT: &str = r#"#!/usr/bin/env python3
import json
import sys


def read_message():
    line = sys.stdin.readline()
    if not line:
        return None
    return json.loads(line)


def send_message(message):
    sys.stdout.write(json.dumps(message, separators=(",", ":")) + "\n")
    sys.stdout.flush()


def send_initialize_response(message):
    protocol_version = message.get("params", {}).get("protocolVersion", "2025-11-25")
    send_message(
        {
            "jsonrpc": "2.0",
            "id": message["id"],
            "result": {
                "protocolVersion": protocol_version,
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "chainbot-runtime-fixture", "version": "1.0.0"},
            },
        }
    )


while True:
    message = read_message()
    if message is None:
        break

    method = message.get("method")

    if method == "initialize":
        send_initialize_response(message)
        continue

    if method == "notifications/initialized":
        continue

    if method == "tools/list":
        send_message(
            {
                "jsonrpc": "2.0",
                "id": message["id"],
                "result": {
                    "tools": [
                        {
                            "name": "echo",
                            "description": "Echo tool",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "message": {"type": "string"}
                                },
                                "required": ["message"],
                            },
                        }
                    ]
                },
            }
        )
        continue

    if method == "tools/call":
        arguments = message.get("params", {}).get("arguments", {})
        send_message(
            {
                "jsonrpc": "2.0",
                "id": message["id"],
                "result": {
                    "content": [],
                    "structuredContent": {"message": arguments.get("message")},
                    "isError": False,
                },
            }
        )
        continue

    if message.get("id") is not None:
        send_message(
            {
                "jsonrpc": "2.0",
                "id": message["id"],
                "error": {"code": -32601, "message": "unsupported method"},
            }
        )
"#;

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
fn depends_mode_any_waits_for_all_dependencies_to_finish() {
    let mut registry = BuiltinNodeRegistry::with_test_handlers();
    registry.register("builtin.fail", |_request| {
        Err(ContractError::CliUsage {
            message: "forced node failure".to_owned(),
        })
    });

    let workflow = WorkflowDefinition {
        api_version: "2.0.0".to_owned(),
        workflow_id: "wf-any-waits".to_owned(),
        name: "any-waits".to_owned(),
        runtime: RuntimeVariableLayers::default(),
        nodes: vec![
            builtin_node("fast-ok", "builtin.identity", vec![], DependsMode::All),
            builtin_node("prep", "builtin.identity", vec![], DependsMode::All),
            builtin_node("late-fail", "builtin.fail", vec!["prep"], DependsMode::All),
            builtin_node(
                "any-after",
                "builtin.identity",
                vec!["fast-ok", "late-fail"],
                DependsMode::Any,
            ),
        ],
        package_root: PathBuf::new(),
    };

    let execution_plane =
        ExecutionPlane::new(vec![workflow], BTreeMap::new(), registry).expect("plane");
    let report = execution_plane
        .execute(&NormalizedRunRequest::new("run-any-waits", "wf-any-waits"))
        .expect("workflow should complete with bounded failure");

    assert_eq!(report.status, WorkflowRunStatus::Failed);
    assert_eq!(
        report.schedule_waves,
        vec![
            vec!["fast-ok".to_owned(), "prep".to_owned()],
            vec!["late-fail".to_owned()],
            vec!["any-after".to_owned()],
        ]
    );
    assert_eq!(
        report.node_states.get("fast-ok"),
        Some(&ScheduledNodeState::Succeeded)
    );
    assert_eq!(
        report.node_states.get("late-fail"),
        Some(&ScheduledNodeState::Failed)
    );
    assert_eq!(
        report.node_states.get("any-after"),
        Some(&ScheduledNodeState::Succeeded)
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
fn production_registry_locks_merge_template_math_and_json_semantics() {
    let registry = build_builtin_registry(test_runtime_context());
    let request = BuiltinNodeRequest {
        run_id: "run-contract".to_owned(),
        workflow_id: "wf-contract".to_owned(),
        workflow_package_root: PathBuf::new(),
        node_id: "node-contract".to_owned(),
        operation: "run".to_owned(),
        inputs: BTreeMap::new(),
        runtime_namespaces: Default::default(),
    };

    let merge_result = registry
        .dispatch(
            "builtin.data.merge",
            &BuiltinNodeRequest {
                operation: "objects".to_owned(),
                inputs: BTreeMap::from([(
                    String::from("objects"),
                    json!([
                        {"outer": {"left": 1}, "shared": "left"},
                        {"outer": {"right": 2}, "shared": "right"}
                    ]),
                )]),
                ..request.clone()
            },
        )
        .expect("merge should support objects array");
    assert_eq!(
        merge_result.outputs.get("result"),
        Some(&json!({"outer": {"right": 2}, "shared": "right"}))
    );

    let template_nested = registry
        .dispatch(
            "builtin.data.template",
            &BuiltinNodeRequest {
                operation: "render".to_owned(),
                inputs: BTreeMap::from([
                    (String::from("template"), json!("symbol={{quote.symbol}}")),
                    (
                        String::from("values"),
                        json!({"quote": {"symbol": "ETHUSDT"}}),
                    ),
                ]),
                ..request.clone()
            },
        )
        .expect("template should support dotted placeholders");
    assert_eq!(
        template_nested.outputs.get("result"),
        Some(&json!("symbol=ETHUSDT"))
    );

    let missing_template_error = registry
        .dispatch(
            "builtin.data.template",
            &BuiltinNodeRequest {
                operation: "render".to_owned(),
                inputs: BTreeMap::from([
                    (String::from("template"), json!("symbol={{quote.symbol}}")),
                    (String::from("values"), json!({"quote": {}})),
                ]),
                ..request.clone()
            },
        )
        .expect_err("template should fail on missing placeholders");
    assert!(missing_template_error
        .to_string()
        .contains("missing template value for quote.symbol"));

    let divide_by_zero_error = registry
        .dispatch(
            "builtin.data.math",
            &BuiltinNodeRequest {
                operation: "divide".to_owned(),
                inputs: BTreeMap::from([
                    (String::from("left"), json!(10)),
                    (String::from("right"), json!(0)),
                ]),
                ..request.clone()
            },
        )
        .expect_err("math divide should reject zero divisor");
    assert!(divide_by_zero_error
        .to_string()
        .contains("cannot divide by zero"));

    let round_result = registry
        .dispatch(
            "builtin.data.math",
            &BuiltinNodeRequest {
                operation: "run".to_owned(),
                inputs: BTreeMap::from([
                    (String::from("value"), json!(3.14159)),
                    (String::from("precision"), json!(2)),
                ]),
                ..request.clone()
            },
        )
        .expect("default math operation should round");
    assert_eq!(round_result.outputs.get("result"), Some(&json!(3.14)));

    let parse_error = registry
        .dispatch(
            "builtin.data.parse_json",
            &BuiltinNodeRequest {
                operation: "parse".to_owned(),
                inputs: BTreeMap::from([(String::from("text"), json!("not-json"))]),
                ..request.clone()
            },
        )
        .expect_err("parse_json should reject invalid JSON text");
    assert!(parse_error
        .to_string()
        .contains("failed to parse JSON text"));

    let stringify_result = registry
        .dispatch(
            "builtin.data.stringify_json",
            &BuiltinNodeRequest {
                operation: "stringify".to_owned(),
                inputs: BTreeMap::from([(String::from("value"), json!({"ok": true, "count": 2}))]),
                ..request.clone()
            },
        )
        .expect("stringify_json should accept generic JSON values");
    assert_eq!(
        stringify_result.outputs.get("result"),
        Some(&json!("{\"count\":2,\"ok\":true}"))
    );

    let get_result = registry
        .dispatch(
            "builtin.data.get",
            &BuiltinNodeRequest {
                operation: "path".to_owned(),
                inputs: BTreeMap::from([
                    (
                        String::from("input"),
                        json!({"items": [{"symbol": "ETHUSDT"}, {"symbol": "BTCUSDT"}]}),
                    ),
                    (String::from("path"), json!("items.1.symbol")),
                ]),
                ..request.clone()
            },
        )
        .expect("data.get should support array indexes in dotted paths");
    assert_eq!(get_result.outputs.get("result"), Some(&json!("BTCUSDT")));

    let get_missing = registry
        .dispatch(
            "builtin.data.get",
            &BuiltinNodeRequest {
                operation: "path".to_owned(),
                inputs: BTreeMap::from([
                    (String::from("input"), json!({"items": []})),
                    (String::from("path"), json!("items.0.symbol")),
                ]),
                ..request.clone()
            },
        )
        .expect_err("data.get should fail on missing path");
    assert!(get_missing
        .to_string()
        .contains("missing data.get path items.0.symbol"));

    let coalesce_result = registry
        .dispatch(
            "builtin.data.coalesce",
            &BuiltinNodeRequest {
                operation: "first".to_owned(),
                inputs: BTreeMap::from([(
                    String::from("values"),
                    json!([null, "fallback", "later"]),
                )]),
                ..request.clone()
            },
        )
        .expect("data.coalesce should return first non-null entry");
    assert_eq!(
        coalesce_result.outputs.get("result"),
        Some(&json!("fallback"))
    );

    let coalesce_null = registry
        .dispatch(
            "builtin.data.coalesce",
            &BuiltinNodeRequest {
                operation: "first".to_owned(),
                inputs: BTreeMap::from([(String::from("values"), json!([null, null]))]),
                ..request.clone()
            },
        )
        .expect("data.coalesce should yield null when all entries are null");
    assert_eq!(
        coalesce_null.outputs.get("result"),
        Some(&serde_json::Value::Null)
    );

    let compare_equals = registry
        .dispatch(
            "builtin.data.compare",
            &BuiltinNodeRequest {
                operation: "equals".to_owned(),
                inputs: BTreeMap::from([
                    (String::from("left"), json!("ETHUSDT")),
                    (String::from("right"), json!("ETHUSDT")),
                ]),
                ..request.clone()
            },
        )
        .expect("data.compare should support equality checks");
    assert_eq!(compare_equals.outputs.get("result"), Some(&json!(true)));

    let compare_order_error = registry
        .dispatch(
            "builtin.data.compare",
            &BuiltinNodeRequest {
                operation: "greater_than".to_owned(),
                inputs: BTreeMap::from([
                    (String::from("left"), json!({"price": 1})),
                    (String::from("right"), json!({"price": 2})),
                ]),
                ..request.clone()
            },
        )
        .expect_err("ordered compare should reject object inputs");
    assert!(compare_order_error
        .to_string()
        .contains("supports ordered compare only for matching scalar types"));
}

#[test]
fn builtin_node_registry_test_handlers_extend_with_custom_registration() {
    let mut registry = BuiltinNodeRegistry::with_test_handlers();
    registry.register("builtin.capture_runtime", |request| {
        Ok(BuiltinNodeResult {
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
        runtime_namespaces: RuntimeVariableNamespaces {
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

#[test]
fn production_registry_executes_first_wave_core_builtins() {
    let workflow = WorkflowDefinition {
        api_version: "2.0.0".to_owned(),
        workflow_id: "wf-core-builtins".to_owned(),
        name: "core-builtins".to_owned(),
        runtime: RuntimeVariableLayers {
            workflow_defaults: BTreeMap::from([
                (
                    String::from("left"),
                    json!({"symbol": "ETHUSDT", "exchange": "demo"}),
                ),
                (String::from("right"), json!({"price": 3000})),
                (
                    String::from("greeting_template"),
                    json!("pair={{symbol}} price={{price}}"),
                ),
                (
                    String::from("expected_greeting"),
                    json!("pair=ETHUSDT price=3000"),
                ),
            ]),
            ..RuntimeVariableLayers::default()
        },
        nodes: vec![
            NodeDefinition {
                api_version: "2.0.0".to_owned(),
                node_id: "merge-values".to_owned(),
                kind: "builtin".to_owned(),
                plugin_id: "builtin.data.merge".to_owned(),
                operation: "objects".to_owned(),
                depends_mode: DependsMode::All,
                depends_on: vec![],
                inputs: vec![
                    VariableBinding {
                        target: "left".to_owned(),
                        source: VariableReference {
                            namespace: RuntimeVariableNamespace::WorkflowDefaults,
                            key: "left".to_owned(),
                        },
                    },
                    VariableBinding {
                        target: "right".to_owned(),
                        source: VariableReference {
                            namespace: RuntimeVariableNamespace::WorkflowDefaults,
                            key: "right".to_owned(),
                        },
                    },
                ],
                when: None,
                subflow: None,
            },
            NodeDefinition {
                api_version: "2.0.0".to_owned(),
                node_id: "render-template".to_owned(),
                kind: "builtin".to_owned(),
                plugin_id: "builtin.data.template".to_owned(),
                operation: "render".to_owned(),
                depends_mode: DependsMode::All,
                depends_on: vec![String::from("merge-values")],
                inputs: vec![
                    VariableBinding {
                        target: "template".to_owned(),
                        source: VariableReference {
                            namespace: RuntimeVariableNamespace::WorkflowDefaults,
                            key: "greeting_template".to_owned(),
                        },
                    },
                    VariableBinding {
                        target: "values".to_owned(),
                        source: VariableReference {
                            namespace: RuntimeVariableNamespace::RunScoped,
                            key: "result".to_owned(),
                        },
                    },
                ],
                when: None,
                subflow: None,
            },
            NodeDefinition {
                api_version: "2.0.0".to_owned(),
                node_id: "assert-template".to_owned(),
                kind: "builtin".to_owned(),
                plugin_id: "builtin.flow.assert".to_owned(),
                operation: "equals".to_owned(),
                depends_mode: DependsMode::All,
                depends_on: vec![String::from("render-template")],
                inputs: vec![
                    VariableBinding {
                        target: "value".to_owned(),
                        source: VariableReference {
                            namespace: RuntimeVariableNamespace::RunScoped,
                            key: "result".to_owned(),
                        },
                    },
                    VariableBinding {
                        target: "expected".to_owned(),
                        source: VariableReference {
                            namespace: RuntimeVariableNamespace::WorkflowDefaults,
                            key: "expected_greeting".to_owned(),
                        },
                    },
                ],
                when: None,
                subflow: None,
            },
        ],
        package_root: PathBuf::new(),
    };

    let execution_plane = ExecutionPlane::new(
        vec![workflow],
        BTreeMap::new(),
        build_builtin_registry(test_runtime_context()),
    )
    .expect("plane");

    let report = execution_plane
        .execute(&NormalizedRunRequest::new(
            "run-core-builtins",
            "wf-core-builtins",
        ))
        .expect("first-wave builtins should execute");

    assert_eq!(report.status, WorkflowRunStatus::Succeeded);
    assert_eq!(
        report
            .node_outputs
            .get("merge-values")
            .and_then(|outputs| outputs.get("result")),
        Some(&json!({"exchange": "demo", "price": 3000, "symbol": "ETHUSDT"}))
    );
    assert_eq!(
        report
            .node_outputs
            .get("render-template")
            .and_then(|outputs| outputs.get("result")),
        Some(&json!("pair=ETHUSDT price=3000"))
    );
    assert_eq!(
        report
            .node_outputs
            .get("assert-template")
            .and_then(|outputs| outputs.get("ok")),
        Some(&json!(true))
    );
}

#[test]
fn production_registry_builtin_flow_fail_marks_workflow_failed() {
    let workflow = WorkflowDefinition {
        api_version: "2.0.0".to_owned(),
        workflow_id: "wf-core-fail".to_owned(),
        name: "core-fail".to_owned(),
        runtime: RuntimeVariableLayers::default(),
        nodes: vec![NodeDefinition {
            api_version: "2.0.0".to_owned(),
            node_id: "stop-now".to_owned(),
            kind: "builtin".to_owned(),
            plugin_id: "builtin.flow.fail".to_owned(),
            operation: "raise".to_owned(),
            depends_mode: DependsMode::All,
            depends_on: vec![],
            inputs: vec![VariableBinding {
                target: "message".to_owned(),
                source: VariableReference {
                    namespace: RuntimeVariableNamespace::WorkflowDefaults,
                    key: "message".to_owned(),
                },
            }],
            when: None,
            subflow: None,
        }],
        package_root: PathBuf::new(),
    };
    let workflow = WorkflowDefinition {
        runtime: RuntimeVariableLayers {
            workflow_defaults: BTreeMap::from([(String::from("message"), json!("stop requested"))]),
            ..workflow.runtime
        },
        ..workflow
    };

    let execution_plane = ExecutionPlane::new(
        vec![workflow],
        BTreeMap::new(),
        build_builtin_registry(test_runtime_context()),
    )
    .expect("plane");

    let report = execution_plane
        .execute(&NormalizedRunRequest::new("run-core-fail", "wf-core-fail"))
        .expect("workflow should complete with bounded failure");

    assert_eq!(report.status, WorkflowRunStatus::Failed);
    assert_eq!(
        report.node_failures.get("stop-now"),
        Some(&String::from(
            "workflow wf-core-fail node stop-now stop requested"
        ))
    );
}

#[test]
fn production_registry_executes_second_wave_data_builtins() {
    let workflow = WorkflowDefinition {
        api_version: "2.0.0".to_owned(),
        workflow_id: "wf-second-wave-builtins".to_owned(),
        name: "second-wave-builtins".to_owned(),
        runtime: RuntimeVariableLayers {
            workflow_defaults: BTreeMap::from([
                (
                    String::from("json_text"),
                    json!("{\"price\":3000,\"size\":2}"),
                ),
                (
                    String::from("payload"),
                    json!({"symbol": "ETHUSDT", "price": 3000}),
                ),
                (String::from("left"), json!(12)),
                (String::from("right"), json!(2.5)),
            ]),
            ..RuntimeVariableLayers::default()
        },
        nodes: vec![
            NodeDefinition {
                api_version: "2.0.0".to_owned(),
                node_id: "parse-json".to_owned(),
                kind: "builtin".to_owned(),
                plugin_id: "builtin.data.parse_json".to_owned(),
                operation: "parse".to_owned(),
                depends_mode: DependsMode::All,
                depends_on: vec![],
                inputs: vec![VariableBinding {
                    target: "text".to_owned(),
                    source: VariableReference {
                        namespace: RuntimeVariableNamespace::WorkflowDefaults,
                        key: "json_text".to_owned(),
                    },
                }],
                when: None,
                subflow: None,
            },
            NodeDefinition {
                api_version: "2.0.0".to_owned(),
                node_id: "stringify-json".to_owned(),
                kind: "builtin".to_owned(),
                plugin_id: "builtin.data.stringify_json".to_owned(),
                operation: "stringify".to_owned(),
                depends_mode: DependsMode::All,
                depends_on: vec![String::from("parse-json")],
                inputs: vec![VariableBinding {
                    target: "value".to_owned(),
                    source: VariableReference {
                        namespace: RuntimeVariableNamespace::WorkflowDefaults,
                        key: "payload".to_owned(),
                    },
                }],
                when: None,
                subflow: None,
            },
            NodeDefinition {
                api_version: "2.0.0".to_owned(),
                node_id: "multiply-values".to_owned(),
                kind: "builtin".to_owned(),
                plugin_id: "builtin.data.math".to_owned(),
                operation: "multiply".to_owned(),
                depends_mode: DependsMode::All,
                depends_on: vec![String::from("stringify-json")],
                inputs: vec![
                    VariableBinding {
                        target: "left".to_owned(),
                        source: VariableReference {
                            namespace: RuntimeVariableNamespace::WorkflowDefaults,
                            key: "left".to_owned(),
                        },
                    },
                    VariableBinding {
                        target: "right".to_owned(),
                        source: VariableReference {
                            namespace: RuntimeVariableNamespace::WorkflowDefaults,
                            key: "right".to_owned(),
                        },
                    },
                ],
                when: None,
                subflow: None,
            },
        ],
        package_root: PathBuf::new(),
    };

    let execution_plane = ExecutionPlane::new(
        vec![workflow],
        BTreeMap::new(),
        build_builtin_registry(test_runtime_context()),
    )
    .expect("plane");

    let report = execution_plane
        .execute(&NormalizedRunRequest::new(
            "run-second-wave-builtins",
            "wf-second-wave-builtins",
        ))
        .expect("second-wave builtins should execute");

    assert_eq!(report.status, WorkflowRunStatus::Succeeded);
    assert_eq!(
        report
            .node_outputs
            .get("parse-json")
            .and_then(|outputs| outputs.get("result")),
        Some(&json!({"price": 3000, "size": 2}))
    );
    assert_eq!(
        report
            .node_outputs
            .get("stringify-json")
            .and_then(|outputs| outputs.get("result")),
        Some(&json!("{\"price\":3000,\"symbol\":\"ETHUSDT\"}"))
    );
    assert_eq!(
        report
            .node_outputs
            .get("multiply-values")
            .and_then(|outputs| outputs.get("result")),
        Some(&json!(30.0))
    );
}

#[test]
fn production_registry_executes_third_wave_data_glue_builtins() {
    let workflow = WorkflowDefinition {
        api_version: "2.0.0".to_owned(),
        workflow_id: "wf-third-wave-builtins".to_owned(),
        name: "third-wave-builtins".to_owned(),
        runtime: RuntimeVariableLayers {
            workflow_defaults: BTreeMap::from([(
                String::from("payload"),
                json!({"quote": {"symbol": "ETHUSDT", "price": 3000}}),
            )]),
            ..RuntimeVariableLayers::default()
        },
        nodes: vec![
            NodeDefinition {
                api_version: "2.0.0".to_owned(),
                node_id: "extract-symbol".to_owned(),
                kind: "builtin".to_owned(),
                plugin_id: "builtin.data.get".to_owned(),
                operation: "path".to_owned(),
                depends_mode: DependsMode::All,
                depends_on: vec![],
                inputs: vec![
                    VariableBinding {
                        target: "input".to_owned(),
                        source: VariableReference {
                            namespace: RuntimeVariableNamespace::WorkflowDefaults,
                            key: "payload".to_owned(),
                        },
                    },
                    VariableBinding {
                        target: "path".to_owned(),
                        source: VariableReference {
                            namespace: RuntimeVariableNamespace::WorkflowDefaults,
                            key: "path_symbol".to_owned(),
                        },
                    },
                ],
                when: None,
                subflow: None,
            },
            NodeDefinition {
                api_version: "2.0.0".to_owned(),
                node_id: "choose-symbol".to_owned(),
                kind: "builtin".to_owned(),
                plugin_id: "builtin.data.coalesce".to_owned(),
                operation: "first".to_owned(),
                depends_mode: DependsMode::All,
                depends_on: vec![String::from("extract-symbol")],
                inputs: vec![VariableBinding {
                    target: "values".to_owned(),
                    source: VariableReference {
                        namespace: RuntimeVariableNamespace::WorkflowDefaults,
                        key: "symbol_candidates".to_owned(),
                    },
                }],
                when: None,
                subflow: None,
            },
            NodeDefinition {
                api_version: "2.0.0".to_owned(),
                node_id: "compare-price".to_owned(),
                kind: "builtin".to_owned(),
                plugin_id: "builtin.data.compare".to_owned(),
                operation: "greater_than".to_owned(),
                depends_mode: DependsMode::All,
                depends_on: vec![String::from("choose-symbol")],
                inputs: vec![
                    VariableBinding {
                        target: "left".to_owned(),
                        source: VariableReference {
                            namespace: RuntimeVariableNamespace::WorkflowDefaults,
                            key: "price_left".to_owned(),
                        },
                    },
                    VariableBinding {
                        target: "right".to_owned(),
                        source: VariableReference {
                            namespace: RuntimeVariableNamespace::WorkflowDefaults,
                            key: "price_right".to_owned(),
                        },
                    },
                ],
                when: None,
                subflow: None,
            },
        ],
        package_root: PathBuf::new(),
    };
    let workflow = WorkflowDefinition {
        runtime: RuntimeVariableLayers {
            workflow_defaults: BTreeMap::from([
                (
                    String::from("payload"),
                    json!({"quote": {"symbol": "ETHUSDT", "price": 3000}}),
                ),
                (String::from("path_symbol"), json!("quote.symbol")),
                (
                    String::from("symbol_candidates"),
                    json!([null, "ETHUSDT", "BTCUSDT"]),
                ),
                (String::from("price_left"), json!(3000)),
                (String::from("price_right"), json!(2500)),
            ]),
            ..RuntimeVariableLayers::default()
        },
        ..workflow
    };

    let execution_plane = ExecutionPlane::new(
        vec![workflow],
        BTreeMap::new(),
        build_builtin_registry(test_runtime_context()),
    )
    .expect("plane");

    let report = execution_plane
        .execute(&NormalizedRunRequest::new(
            "run-third-wave-builtins",
            "wf-third-wave-builtins",
        ))
        .expect("third-wave builtins should execute");

    assert_eq!(report.status, WorkflowRunStatus::Succeeded);
    assert_eq!(
        report
            .node_outputs
            .get("extract-symbol")
            .and_then(|outputs| outputs.get("result")),
        Some(&json!("ETHUSDT"))
    );
    assert_eq!(
        report
            .node_outputs
            .get("choose-symbol")
            .and_then(|outputs| outputs.get("result")),
        Some(&json!("ETHUSDT"))
    );
    assert_eq!(
        report
            .node_outputs
            .get("compare-price")
            .and_then(|outputs| outputs.get("result")),
        Some(&json!(true))
    );
}

#[test]
fn runtime_execution_routes_mcp_tool_entrypoint() {
    let root = unique_test_root("runtime-mcp-entrypoint");
    let plugins_root = root.join("plugins");
    let plugin_root = plugins_root.join("mcp-echo");
    let stdio_script = plugin_root.join("bin").join("mcp_runtime_fixture.py");
    write_executable_script_contents(&stdio_script, MCP_RUNTIME_STDIO_FIXTURE_SCRIPT);

    let workflow = WorkflowDefinition {
        api_version: "2.0.0".to_owned(),
        workflow_id: "wf-runtime-mcp".to_owned(),
        name: "runtime-mcp".to_owned(),
        runtime: RuntimeVariableLayers::default(),
        nodes: vec![NodeDefinition {
            api_version: "2.0.0".to_owned(),
            node_id: "mcp-echo".to_owned(),
            kind: "plugin".to_owned(),
            plugin_id: "mcp-echo".to_owned(),
            operation: "echo".to_owned(),
            depends_mode: DependsMode::All,
            depends_on: Vec::new(),
            inputs: vec![VariableBinding {
                target: "message".to_owned(),
                source: VariableReference {
                    namespace: RuntimeVariableNamespace::ManualInvocationInput,
                    key: "message".to_owned(),
                },
            }],
            when: None,
            subflow: None,
        }],
        package_root: PathBuf::new(),
    };

    let manifest = PluginManifest {
        api_version: "2.0.0".to_owned(),
        plugin_id: "mcp-echo".to_owned(),
        kind: PLUGIN_KIND_EXTERNAL_NODE.to_owned(),
        entrypoint: EXTERNAL_NODE_ENTRYPOINT_MCP_TOOL_V1.to_owned(),
        capabilities: vec![NODE_PLUGIN_EXECUTE_CAPABILITY.to_owned()],
        executable: None,
        trigger_runtime: None,
        input_schema: Vec::new(),
        output_schema: Vec::new(),
        operations: vec![PluginOperationDescriptor {
            name: "echo".to_owned(),
            summary: Some("Echo message payload".to_owned()),
            input_schema: vec!["message".to_owned()],
            output_schema: vec!["message".to_owned()],
            ..PluginOperationDescriptor::default()
        }],
        event_schema: None,
        activation: None,
        mcp: Some(McpPluginContract {
            transport: McpTransportKind::Stdio,
            stdio: Some(McpStdioTransportConfig {
                command: "bin/mcp_runtime_fixture.py".to_owned(),
                args: Vec::new(),
            }),
            streamable_http: None,
            auth: None,
        }),
        manifest_path: plugin_root.join("config.toml"),
    };

    let execution_plane = ExecutionPlane::with_plugin_runtime(
        vec![workflow],
        BTreeMap::new(),
        BuiltinNodeRegistry::with_test_handlers(),
        vec![manifest],
        BTreeMap::new(),
        plugins_root,
        root.join("secrets"),
        SecretDecryptMode::Plaintext,
    )
    .expect("runtime execution plane with MCP plugin should be constructible");

    let mut request = NormalizedRunRequest::new("run-runtime-mcp", "wf-runtime-mcp");
    request
        .manual_invocation_input
        .insert("message".to_owned(), json!("hello-mcp"));

    let report = execution_plane
        .execute(&request)
        .expect("runtime should route mcp.tool.v1 plugins through MCP host path");

    assert_eq!(report.status, WorkflowRunStatus::Succeeded);
    assert_eq!(report.schedule_waves, vec![vec!["mcp-echo".to_owned()]]);
    assert_eq!(
        report.node_states.get("mcp-echo"),
        Some(&ScheduledNodeState::Succeeded)
    );
    assert_eq!(
        report
            .node_outputs
            .get("mcp-echo")
            .and_then(|outputs| outputs.get("message")),
        Some(&json!("hello-mcp"))
    );
    assert_eq!(
        report.runtime_namespaces.run_scoped.get("message"),
        Some(&json!("hello-mcp"))
    );
}

#[test]
fn runtime_execution_preserves_legacy_external_node_dispatch() {
    let root = unique_test_root("runtime-legacy-entrypoint");
    let plugins_root = root.join("plugins");
    let plugin_root = plugins_root.join("legacy-quote");
    let executable = plugin_root.join("bin").join("legacy_node.sh");
    write_executable_script_contents(
        &executable,
        "#!/bin/sh\ncat >/dev/null\nprintf '%s' '{\"contract_version\":\"1.0.0\",\"success\":true,\"output\":{\"decision\":\"buy\"}}'\n",
    );

    let workflow = WorkflowDefinition {
        api_version: "2.0.0".to_owned(),
        workflow_id: "wf-runtime-legacy".to_owned(),
        name: "runtime-legacy".to_owned(),
        runtime: RuntimeVariableLayers::default(),
        nodes: vec![NodeDefinition {
            api_version: "2.0.0".to_owned(),
            node_id: "legacy-node".to_owned(),
            kind: "plugin".to_owned(),
            plugin_id: "legacy-quote".to_owned(),
            operation: "normalize".to_owned(),
            depends_mode: DependsMode::All,
            depends_on: Vec::new(),
            inputs: vec![VariableBinding {
                target: "symbol".to_owned(),
                source: VariableReference {
                    namespace: RuntimeVariableNamespace::ManualInvocationInput,
                    key: "symbol".to_owned(),
                },
            }],
            when: None,
            subflow: None,
        }],
        package_root: PathBuf::new(),
    };

    let manifest = PluginManifest {
        api_version: "2.0.0".to_owned(),
        plugin_id: "legacy-quote".to_owned(),
        kind: PLUGIN_KIND_EXTERNAL_NODE.to_owned(),
        entrypoint: EXTERNAL_NODE_ENTRYPOINT_EXEC_V1.to_owned(),
        capabilities: vec![NODE_PLUGIN_EXECUTE_CAPABILITY.to_owned()],
        executable: Some("bin/legacy_node.sh".to_owned()),
        trigger_runtime: None,
        input_schema: Vec::new(),
        output_schema: Vec::new(),
        operations: vec![PluginOperationDescriptor {
            name: "normalize".to_owned(),
            summary: Some("Normalize quote payload".to_owned()),
            input_schema: vec!["symbol".to_owned()],
            output_schema: vec!["decision".to_owned()],
            ..PluginOperationDescriptor::default()
        }],
        event_schema: None,
        activation: None,
        mcp: None,
        manifest_path: plugin_root.join("config.toml"),
    };

    let execution_plane = ExecutionPlane::with_plugin_runtime(
        vec![workflow],
        BTreeMap::new(),
        BuiltinNodeRegistry::with_test_handlers(),
        vec![manifest],
        BTreeMap::new(),
        plugins_root,
        root.join("secrets"),
        SecretDecryptMode::Plaintext,
    )
    .expect("runtime execution plane with legacy plugin should be constructible");

    let mut request = NormalizedRunRequest::new("run-runtime-legacy", "wf-runtime-legacy");
    request
        .manual_invocation_input
        .insert("symbol".to_owned(), json!("BTCUSDT"));

    let report = execution_plane
        .execute(&request)
        .expect("runtime should preserve legacy node.exec.v1 dispatch path");

    assert_eq!(report.status, WorkflowRunStatus::Succeeded);
    assert_eq!(report.schedule_waves, vec![vec!["legacy-node".to_owned()]]);
    assert_eq!(
        report.node_states.get("legacy-node"),
        Some(&ScheduledNodeState::Succeeded)
    );
    assert_eq!(
        report
            .node_outputs
            .get("legacy-node")
            .and_then(|outputs| outputs.get("decision")),
        Some(&json!("buy"))
    );
    assert_eq!(
        report
            .runtime_namespaces
            .node_outputs_by_producer
            .get("legacy-node")
            .and_then(|outputs| outputs.get("decision")),
        Some(&json!("buy"))
    );
}

#[test]
fn runtime_execution_supports_node_exec_v2_dispatch() {
    let root = unique_test_root("runtime-node-exec-v2");
    let plugins_root = root.join("plugins");
    let plugin_root = plugins_root.join("v2-quote");
    let executable = plugin_root.join("bin").join("v2_node.sh");
    write_executable_script_contents(
        &executable,
        "#!/bin/sh\ncat >/dev/null\nprintf '%s' '{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"contract_version\":\"1.0.0\",\"output\":{\"decision\":\"buy\"}}}'\n",
    );

    let workflow = WorkflowDefinition {
        api_version: "2.0.0".to_owned(),
        workflow_id: "wf-runtime-v2".to_owned(),
        name: "runtime-v2".to_owned(),
        runtime: RuntimeVariableLayers::default(),
        nodes: vec![NodeDefinition {
            api_version: "2.0.0".to_owned(),
            node_id: "v2-node".to_owned(),
            kind: "plugin".to_owned(),
            plugin_id: "v2-quote".to_owned(),
            operation: "normalize".to_owned(),
            depends_mode: DependsMode::All,
            depends_on: Vec::new(),
            inputs: vec![VariableBinding {
                target: "symbol".to_owned(),
                source: VariableReference {
                    namespace: RuntimeVariableNamespace::ManualInvocationInput,
                    key: "symbol".to_owned(),
                },
            }],
            when: None,
            subflow: None,
        }],
        package_root: PathBuf::new(),
    };

    let manifest = PluginManifest {
        api_version: "2.0.0".to_owned(),
        plugin_id: "v2-quote".to_owned(),
        kind: PLUGIN_KIND_EXTERNAL_NODE.to_owned(),
        entrypoint: EXTERNAL_NODE_ENTRYPOINT_EXEC_V2.to_owned(),
        capabilities: vec![NODE_PLUGIN_EXECUTE_CAPABILITY.to_owned()],
        executable: Some("bin/v2_node.sh".to_owned()),
        trigger_runtime: None,
        input_schema: Vec::new(),
        output_schema: Vec::new(),
        operations: vec![PluginOperationDescriptor {
            name: "normalize".to_owned(),
            summary: Some("Normalize quote payload".to_owned()),
            input_schema: vec!["symbol".to_owned()],
            output_schema: vec!["decision".to_owned()],
            ..PluginOperationDescriptor::default()
        }],
        event_schema: None,
        activation: None,
        mcp: None,
        manifest_path: plugin_root.join("config.toml"),
    };

    let execution_plane = ExecutionPlane::with_plugin_runtime(
        vec![workflow],
        BTreeMap::new(),
        BuiltinNodeRegistry::with_test_handlers(),
        vec![manifest],
        BTreeMap::new(),
        plugins_root,
        root.join("secrets"),
        SecretDecryptMode::Plaintext,
    )
    .expect("runtime execution plane with node.exec.v2 plugin should be constructible");

    let mut request = NormalizedRunRequest::new("run-runtime-v2", "wf-runtime-v2");
    request
        .manual_invocation_input
        .insert("symbol".to_owned(), json!("BTCUSDT"));

    let report = execution_plane
        .execute(&request)
        .expect("runtime should dispatch node.exec.v2 plugins through the scheduler");

    assert_eq!(report.status, WorkflowRunStatus::Succeeded);
    assert_eq!(report.schedule_waves, vec![vec!["v2-node".to_owned()]]);
    assert_eq!(report.node_states.get("v2-node"), Some(&ScheduledNodeState::Succeeded));
    assert_eq!(
        report
            .node_outputs
            .get("v2-node")
            .and_then(|outputs| outputs.get("decision")),
        Some(&json!("buy"))
    );
}

#[test]
fn runtime_execution_injects_default_confirmation_for_signed_plugin_operations() {
    let root = unique_test_root("runtime-signed-default-confirmation");
    let plugins_root = root.join("plugins");
    let plugin_root = plugins_root.join("eth-signed");
    let captured_request = root.join("captured-request.json");
    let executable = plugin_root.join("bin").join("signed_node.sh");
    write_executable_script_contents(
        &executable,
        &format!(
            "#!/bin/sh\ncat > \"{}\"\nprintf '%s' '{{\"contract_version\":\"1.0.0\",\"success\":true,\"output\":{{\"status\":\"submitted\"}}}}'\n",
            captured_request.display()
        ),
    );

    let workflow = WorkflowDefinition {
        api_version: "2.0.0".to_owned(),
        workflow_id: "wf-runtime-signed".to_owned(),
        name: "runtime-signed".to_owned(),
        runtime: RuntimeVariableLayers::default(),
        nodes: vec![NodeDefinition {
            api_version: "2.0.0".to_owned(),
            node_id: "signed-node".to_owned(),
            kind: "plugin".to_owned(),
            plugin_id: "eth-signed".to_owned(),
            operation: "eth_raw_write".to_owned(),
            depends_mode: DependsMode::All,
            depends_on: Vec::new(),
            inputs: vec![
                VariableBinding {
                    target: "symbol".to_owned(),
                    source: VariableReference {
                        namespace: RuntimeVariableNamespace::ManualInvocationInput,
                        key: "symbol".to_owned(),
                    },
                },
            ],
            when: None,
            subflow: None,
        }],
        package_root: PathBuf::new(),
    };

    let manifest = PluginManifest {
        api_version: "2.0.0".to_owned(),
        plugin_id: "eth-signed".to_owned(),
        kind: PLUGIN_KIND_EXTERNAL_NODE.to_owned(),
        entrypoint: EXTERNAL_NODE_ENTRYPOINT_EXEC_V1.to_owned(),
        capabilities: vec![NODE_PLUGIN_EXECUTE_CAPABILITY.to_owned()],
        executable: Some("bin/signed_node.sh".to_owned()),
        trigger_runtime: None,
        input_schema: Vec::new(),
        output_schema: Vec::new(),
        operations: vec![PluginOperationDescriptor {
            name: "eth_raw_write".to_owned(),
            summary: Some("Submit signed payload".to_owned()),
            input_schema: vec![
                "symbol".to_owned(),
                "confirmation_mode".to_owned(),
            ],
            optional_input_schema: Vec::new(),
            output_schema: vec!["status".to_owned()],
            kind: PluginOperationKind::RawWrite,
            requires_managed_signing: true,
            default_confirmation: Some("safe".to_owned()),
        }],
        event_schema: None,
        activation: None,
        mcp: None,
        manifest_path: plugin_root.join("config.toml"),
    };

    let execution_plane = ExecutionPlane::with_plugin_runtime(
        vec![workflow],
        BTreeMap::new(),
        BuiltinNodeRegistry::with_test_handlers(),
        vec![manifest],
        BTreeMap::new(),
        plugins_root,
        root.join("secrets"),
        SecretDecryptMode::Plaintext,
    )
    .expect("runtime execution plane with signed plugin should be constructible");

    let mut request = NormalizedRunRequest::new("run-runtime-signed", "wf-runtime-signed");
    request
        .manual_invocation_input
        .insert("symbol".to_owned(), json!("BTCUSDT"));
    let report = execution_plane
        .execute(&request)
        .expect("runtime should inject default confirmation mode for signed plugin operations");

    assert_eq!(report.status, WorkflowRunStatus::Succeeded);
    let captured_body =
        fs::read_to_string(&captured_request).expect("captured request should be readable");
    assert!(captured_body.contains("\"confirmation_mode\":\"safe\""));
    assert!(!captured_body.contains("\"signer_ref\""));
}

fn test_runtime_context() -> BuiltinRuntimeContext {
    BuiltinRuntimeContext {
        root_layout: RootLayout::from_root(PathBuf::from("/tmp/chainbot-builtins-test")),
        secret_mode: SecretDecryptMode::Plaintext,
        worker_host: WorkerHost::new(WorkerHostLimits::default()),
    }
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

fn write_executable_script_contents(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("fixture script parent directory should be creatable");
    }
    fs::write(path, contents).expect("fixture script should be writable");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path)
            .expect("fixture script metadata should exist")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).expect("fixture script should be marked executable");
    }
}

fn unique_test_root(prefix: &str) -> PathBuf {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after UNIX_EPOCH")
        .as_nanos();
    workspace_root()
        .join("target")
        .join("test-roots")
        .join(format!("{prefix}-{now}"))
}

fn workspace_root() -> PathBuf {
    let crate_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    crate_root
        .parent()
        .expect("crates directory should exist")
        .parent()
        .expect("workspace root should exist")
        .to_path_buf()
}
