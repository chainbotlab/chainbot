//! [INPUT]
//! Trigger-plane fixtures, plaintext secret files, and captured external trigger start envelopes.
//!
//! [OUTPUT]
//! Verifies execution-time activation secret injection for official chain trigger listeners.
//!
//! [ROLE]
//! Covers live-only chain trigger startup envelopes without adding runtime-side chain semantics.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use chainbot::domain::trigger::{
    TriggerDefinition, TriggerHostMessage, TriggerPlane, TriggerPluginActivationBindings,
    TriggerPluginHostPolicy,
    TriggerStartCommand, REQUIRED_TRIGGER_PLUGIN_CAPABILITY,
};
use chainbot::plugin::{
    ExternalTriggerRuntimeContract, PluginEventSchemaDescriptor, PluginManifest,
    TriggerDurableAckSemantics, TriggerHostErrorCategory, TriggerPushCallbackSemantics,
    TriggerRuntimeLifecycle,
};
use chainbot::secrets::SecretReference;

#[test]
fn chain_trigger_runtime_injects_activation_secrets_into_start_message() {
    let previous_secret_mode = std::env::var_os("CHAINBOT_SECRET_DECRYPTOR");
    unsafe {
        std::env::set_var("CHAINBOT_SECRET_DECRYPTOR", "plaintext");
    }
    let root = unique_test_root("chain-trigger-activation-runtime");
    let plugin_root = root.join("plugins");
    let secrets_root = root.join("secrets");
    let capture_path = root.join("captured-start.json");
    let executable = plugin_root
        .join("eth-trigger")
        .join("bin")
        .join("trigger.sh");
    write_script(
        &executable,
        &format!(
            "#!/bin/sh\nIFS= read -r line\nprintf '%s' \"$line\" > \"{}\"\nprintf '%s\\n' '{{\"type\":\"ready\",\"protocol_version\":\"2.0.0\"}}'\n",
            capture_path.display()
        ),
    );
    write_plaintext_secret(
        &secrets_root,
        "secret://providers/ethereum/mainnet#token",
        "eth-provider-token",
    );

    let manifest = PluginManifest {
        api_version: "2.0.0".to_owned(),
        plugin_id: "eth-trigger".to_owned(),
        kind: "external_trigger".to_owned(),
        entrypoint: "trigger.exec.v1".to_owned(),
        capabilities: vec![REQUIRED_TRIGGER_PLUGIN_CAPABILITY.to_owned()],
        executable: Some("bin/trigger.sh".to_owned()),
        input_schema: Vec::new(),
        output_schema: Vec::new(),
        trigger_runtime: Some(ExternalTriggerRuntimeContract {
            lifecycle: Some(TriggerRuntimeLifecycle::ProcessShortLived),
            push_callback: Some(TriggerPushCallbackSemantics::InlineResponse),
            durable_ack: Some(TriggerDurableAckSemantics::CallerScope),
            host_error_categories: vec![
                TriggerHostErrorCategory::Transport,
                TriggerHostErrorCategory::ProtocolContract,
                TriggerHostErrorCategory::PluginFatal,
            ],
            module: None,
        }),
        operations: Vec::new(),
        event_schema: Some(PluginEventSchemaDescriptor {
            summary: Some("Ethereum listener payload".to_owned()),
            fields: vec!["event_id".to_owned()],
            listener_modes: vec![],
        }),
        activation: Some(chainbot::plugin::PluginActivationContract {
            required_secret_slots: Vec::new(),
            optional_secret_slots: vec!["rpc_token".to_owned()],
            requires_allowed_origins: true,
        }),
        mcp: None,
        manifest_path: plugin_root.join("eth-trigger").join("config.toml"),
    };
    let definition = TriggerDefinition {
        api_version: "2.0.0".to_owned(),
        trigger_id: "eth-live".to_owned(),
        kind: "external_plugin".to_owned(),
        source: "eth_log".to_owned(),
        plugin: Some("eth-trigger".to_owned()),
        workflow_id: "wf-chain".to_owned(),
        enabled: true,
        params: BTreeMap::from([(
            String::from("endpoint"),
            serde_json::json!("wss://rpc.example"),
        )]),
        input_mapping: BTreeMap::new(),
        package_root: PathBuf::new(),
    };
    let state_layout =
        chainbot::infrastructure::state::StateLayout::from_state_root(root.join("state"));
    let policy = TriggerPluginHostPolicy {
        allowlisted_plugin_ids: BTreeSet::from([String::from("eth-trigger")]),
        allowed_capabilities: BTreeSet::from([REQUIRED_TRIGGER_PLUGIN_CAPABILITY.to_owned()]),
        plugin_root_dir: plugin_root.clone(),
        plugin_activation: BTreeMap::from([(
            String::from("eth-trigger"),
            TriggerPluginActivationBindings {
                secret_bindings: BTreeMap::from([(
                    String::from("rpc_token"),
                    SecretReference::parse("secret://providers/ethereum/mainnet#token")
                        .expect("secret ref should parse"),
                )]),
                allowed_origins: vec![String::from("wss://rpc.example")],
            },
        )]),
        secrets_root_dir: secrets_root,
    };

    let mut plane = TriggerPlane::open_legacy_state_layout_for_tests(
        state_layout,
        vec![definition],
        vec![manifest],
        policy,
        BTreeMap::new(),
        1_710_100_000_000,
    )
    .expect("trigger plane should open");

    let _ = plane
        .collect_run_requests(1_710_100_000_010)
        .expect("collect_run_requests should succeed");

    let captured: TriggerHostMessage = serde_json::from_str(
        &fs::read_to_string(&capture_path).expect("captured start should be readable"),
    )
    .expect("captured start should decode");
    let TriggerHostMessage::Start(TriggerStartCommand { activation, .. }) = captured else {
        panic!("expected start message");
    };
    assert_eq!(
        activation.and_then(|value| value.secrets.get("rpc_token").cloned()),
        Some(String::from("eth-provider-token"))
    );
    let captured_activation = captured_start_activation(&capture_path);
    assert_eq!(
        captured_activation.get("allowed_origins"),
        Some(&serde_json::json!(["wss://rpc.example"]))
    );
    unsafe {
        match previous_secret_mode {
            Some(value) => std::env::set_var("CHAINBOT_SECRET_DECRYPTOR", value),
            None => std::env::remove_var("CHAINBOT_SECRET_DECRYPTOR"),
        }
    }
}

#[test]
fn chain_trigger_runtime_rejects_missing_required_allowed_origins_activation() {
    let root = unique_test_root("chain-trigger-missing-allowed-origins-activation");
    let plugin_root = root.join("plugins");
    let secrets_root = root.join("secrets");
    let capture_path = root.join("captured-start.json");
    let executable = plugin_root
        .join("eth-trigger")
        .join("bin")
        .join("trigger.sh");
    write_script(
        &executable,
        &format!(
            "#!/bin/sh\nIFS= read -r line\nprintf '%s' \"$line\" > \"{}\"\nprintf '%s\\n' '{{\"type\":\"ready\",\"protocol_version\":\"2.0.0\"}}'\n",
            capture_path.display()
        ),
    );

    let manifest = PluginManifest {
        api_version: "2.0.0".to_owned(),
        plugin_id: "eth-trigger".to_owned(),
        kind: "external_trigger".to_owned(),
        entrypoint: "trigger.exec.v1".to_owned(),
        capabilities: vec![REQUIRED_TRIGGER_PLUGIN_CAPABILITY.to_owned()],
        executable: Some("bin/trigger.sh".to_owned()),
        input_schema: Vec::new(),
        output_schema: Vec::new(),
        trigger_runtime: Some(ExternalTriggerRuntimeContract {
            lifecycle: Some(TriggerRuntimeLifecycle::ProcessShortLived),
            push_callback: Some(TriggerPushCallbackSemantics::InlineResponse),
            durable_ack: Some(TriggerDurableAckSemantics::CallerScope),
            host_error_categories: vec![
                TriggerHostErrorCategory::Transport,
                TriggerHostErrorCategory::ProtocolContract,
                TriggerHostErrorCategory::PluginFatal,
            ],
            module: None,
        }),
        operations: Vec::new(),
        event_schema: Some(PluginEventSchemaDescriptor {
            summary: Some("Ethereum listener payload".to_owned()),
            fields: vec!["event_id".to_owned()],
            listener_modes: vec![],
        }),
        activation: Some(chainbot::plugin::PluginActivationContract {
            required_secret_slots: Vec::new(),
            optional_secret_slots: vec!["rpc_token".to_owned()],
            requires_allowed_origins: true,
        }),
        mcp: None,
        manifest_path: plugin_root.join("eth-trigger").join("config.toml"),
    };
    let definition = TriggerDefinition {
        api_version: "2.0.0".to_owned(),
        trigger_id: "eth-live".to_owned(),
        kind: "external_plugin".to_owned(),
        source: "eth_log".to_owned(),
        plugin: Some("eth-trigger".to_owned()),
        workflow_id: "wf-chain".to_owned(),
        enabled: true,
        params: BTreeMap::from([(
            String::from("endpoint"),
            serde_json::json!("wss://rpc.example"),
        )]),
        input_mapping: BTreeMap::new(),
        package_root: PathBuf::new(),
    };
    let state_layout =
        chainbot::infrastructure::state::StateLayout::from_state_root(root.join("state"));
    let policy = TriggerPluginHostPolicy {
        allowlisted_plugin_ids: BTreeSet::from([String::from("eth-trigger")]),
        allowed_capabilities: BTreeSet::from([REQUIRED_TRIGGER_PLUGIN_CAPABILITY.to_owned()]),
        plugin_root_dir: plugin_root.clone(),
        plugin_activation: BTreeMap::new(),
        secrets_root_dir: secrets_root,
    };

    let mut plane = TriggerPlane::open_legacy_state_layout_for_tests(
        state_layout,
        vec![definition],
        vec![manifest],
        policy,
        BTreeMap::new(),
        1_710_100_000_000,
    )
    .expect("trigger plane should open");

    let error = plane
        .collect_run_requests(1_710_100_000_010)
        .expect_err("missing trigger activation must fail closed");
    assert!(matches!(
        error,
        chainbot::domain::trigger::TriggerPlaneError::Contract(
            chainbot::errors::ContractError::InvalidTriggerDefinitionField {
                field: "root_config.plugin_activation",
                detail,
                ..
            }
        ) if detail.contains("allowed_origins must be configured")
    ));
    assert!(
        !capture_path.exists(),
        "trigger plugin should not be spawned when activation requirements are missing"
    );
}

#[test]
fn chain_trigger_runtime_stages_event_before_ack_and_accepts_same_cycle() {
    let root = unique_test_root("chain-trigger-stage-before-ack");
    let plugin_root = root.join("plugins");
    let secrets_root = root.join("secrets");
    let capture_path = root.join("captured-ack.jsonl");
    let executable = plugin_root
        .join("eth-trigger")
        .join("bin")
        .join("trigger.sh");
    write_script(
        &executable,
        &format!(
            "#!/bin/sh\nIFS= read -r _start\nprintf '%s\\n' '{{\"type\":\"ready\",\"protocol_version\":\"2.0.0\"}}'\nprintf '%s\\n' '{{\"type\":\"event\",\"checkpoint\":\"ck-1\",\"event_key\":\"tx-1\",\"occurred_at_ms\":1710100000010,\"payload\":{{\"amount\":\"1\"}},\"dedup_key\":\"dedup-1\"}}'\nIFS= read -r ack\nprintf '%s' \"$ack\" > \"{}\"\n",
            capture_path.display()
        ),
    );

    let manifest = PluginManifest {
        api_version: "2.0.0".to_owned(),
        plugin_id: "eth-trigger".to_owned(),
        kind: "external_trigger".to_owned(),
        entrypoint: "trigger.exec.v1".to_owned(),
        capabilities: vec![REQUIRED_TRIGGER_PLUGIN_CAPABILITY.to_owned()],
        executable: Some("bin/trigger.sh".to_owned()),
        input_schema: Vec::new(),
        output_schema: Vec::new(),
        trigger_runtime: Some(ExternalTriggerRuntimeContract {
            lifecycle: Some(TriggerRuntimeLifecycle::ProcessShortLived),
            push_callback: Some(TriggerPushCallbackSemantics::InlineResponse),
            durable_ack: Some(TriggerDurableAckSemantics::CallerScope),
            host_error_categories: vec![
                TriggerHostErrorCategory::Transport,
                TriggerHostErrorCategory::ProtocolContract,
                TriggerHostErrorCategory::PluginFatal,
            ],
            module: None,
        }),
        operations: Vec::new(),
        event_schema: Some(PluginEventSchemaDescriptor {
            summary: Some("Ethereum listener payload".to_owned()),
            fields: vec!["event_id".to_owned()],
            listener_modes: vec![],
        }),
        activation: None,
        mcp: None,
        manifest_path: plugin_root.join("eth-trigger").join("config.toml"),
    };
    let definition = TriggerDefinition {
        api_version: "2.0.0".to_owned(),
        trigger_id: "eth-live".to_owned(),
        kind: "external_plugin".to_owned(),
        source: "eth_log".to_owned(),
        plugin: Some("eth-trigger".to_owned()),
        workflow_id: "wf-chain".to_owned(),
        enabled: true,
        params: BTreeMap::from([(
            String::from("endpoint"),
            serde_json::json!("mock://eth-log"),
        )]),
        input_mapping: BTreeMap::new(),
        package_root: PathBuf::new(),
    };
    let state_layout =
        chainbot::infrastructure::state::StateLayout::from_state_root(root.join("state"));
    let policy = TriggerPluginHostPolicy {
        allowlisted_plugin_ids: BTreeSet::from([String::from("eth-trigger")]),
        allowed_capabilities: BTreeSet::from([REQUIRED_TRIGGER_PLUGIN_CAPABILITY.to_owned()]),
        plugin_root_dir: plugin_root.clone(),
        plugin_activation: BTreeMap::new(),
        secrets_root_dir: secrets_root,
    };

    let mut plane = TriggerPlane::open_legacy_state_layout_for_tests(
        state_layout,
        vec![definition],
        vec![manifest],
        policy,
        BTreeMap::new(),
        1_710_100_000_000,
    )
    .expect("trigger plane should open");

    let requests = plane
        .collect_run_requests(1_710_100_000_020)
        .expect("collect_run_requests should succeed");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].event_id, "eth-live:tx-1");

    let ack = fs::read_to_string(capture_path).expect("ack should be captured");
    assert!(ack.contains("\"type\":\"ack\""));
    assert!(ack.contains("\"checkpoint\":\"ck-1\""));
}

fn write_script(path: &Path, contents: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("script parent should be creatable");
    }
    fs::write(path, contents).expect("script should be writable");
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

fn captured_start_activation(path: &Path) -> serde_json::Map<String, serde_json::Value> {
    let captured: TriggerHostMessage = serde_json::from_str(
        &fs::read_to_string(path).expect("captured start should be readable"),
    )
    .expect("captured start should decode");
    let TriggerHostMessage::Start(TriggerStartCommand { activation, .. }) = captured else {
        panic!("expected start message");
    };
    activation
        .and_then(|value| serde_json::to_value(value).ok())
        .and_then(|value| value.as_object().cloned())
        .expect("activation should be present")
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
