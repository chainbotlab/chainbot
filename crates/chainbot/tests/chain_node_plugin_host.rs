//! [INPUT]
//! Workflow/plugin fixtures, plaintext secret files, and captured official-chain plugin requests.
//!
//! [OUTPUT]
//! Verifies execution-time activation secret resolution, request injection, and failure redaction for chain node plugins.
//!
//! [ROLE]
//! Covers managed-secret host guardrails for official chain node plugin surfaces.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use chainbot::app::ExecutionPlane;
use chainbot::builtins::nodes::registry_store::BuiltinNodeRegistry;
use chainbot::builtins::SecretDecryptMode;
use chainbot::domain::runtime::{NodeDefinition, NormalizedRunRequest};
use chainbot::domain::workflow::{
    DependsMode, RuntimeVariableLayers, RuntimeVariableNamespace, VariableBinding,
    VariableReference, WorkflowDefinition,
};
use chainbot::plugin::{
    PluginManifest, PluginOperationDescriptor, PluginOperationKind,
    EXTERNAL_NODE_ENTRYPOINT_EXEC_V1, NODE_PLUGIN_EXECUTE_CAPABILITY, PLUGIN_KIND_EXTERNAL_NODE,
};
use chainbot::secrets::SecretReference;
use serde_json::json;

#[test]
fn chain_node_runtime_injects_activation_secrets_and_redacts_failures() {
    let root = unique_test_root("chain-node-activation-runtime");
    let plugins_root = root.join("plugins");
    let secrets_root = root.join("secrets");
    let plugin_root = plugins_root.join("eth-node");
    let captured_request = root.join("captured-request.json");
    let executable = plugin_root.join("bin").join("node.sh");
    write_plugin_script(
        &executable,
        &format!(
            "#!/bin/sh\ncat > \"{}\"\nprintf '%s' '{{\"contract_version\":\"1.0.0\",\"success\":false,\"error\":\"secret=super-secret-value\"}}'\n",
            captured_request.display()
        ),
    );
    write_plaintext_secret(
        &secrets_root,
        "secret://wallets/eth/hot#private_key",
        "super-secret-value",
    );

    let workflow = WorkflowDefinition {
        api_version: "2.0.0".to_owned(),
        workflow_id: "wf-chain-node".to_owned(),
        name: "wf-chain-node".to_owned(),
        runtime: RuntimeVariableLayers::default(),
        nodes: vec![NodeDefinition {
            api_version: "2.0.0".to_owned(),
            node_id: "submit".to_owned(),
            kind: "plugin".to_owned(),
            plugin_id: "eth-node".to_owned(),
            operation: "eth_raw_write".to_owned(),
            depends_mode: DependsMode::All,
            depends_on: Vec::new(),
            inputs: vec![VariableBinding {
                target: "signer_ref".to_owned(),
                source: VariableReference {
                    namespace: RuntimeVariableNamespace::ManualInvocationInput,
                    key: "signer_ref".to_owned(),
                },
            }],
            when: None,
            subflow: None,
        }],
        package_root: PathBuf::new(),
    };

    let manifest = PluginManifest {
        api_version: "2.0.0".to_owned(),
        plugin_id: "eth-node".to_owned(),
        kind: PLUGIN_KIND_EXTERNAL_NODE.to_owned(),
        entrypoint: EXTERNAL_NODE_ENTRYPOINT_EXEC_V1.to_owned(),
        capabilities: vec![NODE_PLUGIN_EXECUTE_CAPABILITY.to_owned()],
        executable: Some("bin/node.sh".to_owned()),
        input_schema: Vec::new(),
        output_schema: Vec::new(),
        trigger_runtime: None,
        operations: vec![PluginOperationDescriptor {
            name: "eth_raw_write".to_owned(),
            summary: Some("Submit signed payload".to_owned()),
            input_schema: vec!["signer_ref".to_owned(), "confirmation_mode".to_owned()],
            output_schema: vec!["status".to_owned()],
            kind: PluginOperationKind::RawWrite,
            requires_managed_signing: true,
            default_confirmation: Some("safe".to_owned()),
        }],
        event_schema: None,
        mcp: None,
        manifest_path: plugin_root.join("config.toml"),
    };

    let execution_plane = ExecutionPlane::with_plugin_runtime(
        vec![workflow],
        BTreeMap::new(),
        BuiltinNodeRegistry::with_test_handlers(),
        vec![manifest],
        BTreeMap::from([(
            String::from("eth-node"),
            BTreeMap::from([(
                String::from("signer"),
                SecretReference::parse("secret://wallets/eth/hot#private_key")
                    .expect("secret ref should parse"),
            )]),
        )]),
        plugins_root,
        secrets_root,
        SecretDecryptMode::Plaintext,
    )
    .expect("execution plane should build");

    let mut request = NormalizedRunRequest::new("run-chain-node", "wf-chain-node");
    request
        .manual_invocation_input
        .insert(String::from("signer_ref"), json!("signer"));

    let report = execution_plane
        .execute(&request)
        .expect("runtime should return a failed workflow report");
    assert_eq!(
        report.status,
        chainbot::domain::runtime::WorkflowRunStatus::Failed
    );
    let failure = report
        .node_failures
        .get("submit")
        .expect("failed node should surface a failure message");
    assert!(failure.contains("[REDACTED_SECRET]"));
    assert!(!failure.contains("super-secret-value"));

    let captured_body =
        fs::read_to_string(&captured_request).expect("captured request should exist");
    assert!(
        captured_body.contains("\"activation\":{\"secrets\":{\"signer\":\"super-secret-value\"}}")
    );
    assert!(captured_body.contains("\"confirmation_mode\":\"safe\""));
}

fn write_plugin_script(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("plugin script parent should be creatable");
    }
    fs::write(path, contents).expect("plugin script should be writable");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path)
            .expect("metadata should exist")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).expect("permissions should update");
    }
}

fn write_plaintext_secret(root: &Path, secret_ref: &str, value: &str) {
    let reference = SecretReference::parse(secret_ref).expect("secret ref should parse");
    let path = root
        .join(&reference.namespace)
        .join(format!("{}.gpg", reference.name));
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("secret parent should be creatable");
    }
    let payload = match reference.key.as_deref() {
        Some(key) if key != "password" => format!("{key}={value}\n"),
        _ => format!("{value}\n"),
    };
    fs::write(path, payload).expect("secret should be writable");
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
        .expect("crates dir")
        .parent()
        .expect("workspace root")
        .to_path_buf()
}
