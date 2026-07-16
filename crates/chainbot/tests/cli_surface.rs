//! [INPUT]
//! CLI binary invocations, temporary fixture roots, and persisted trigger package state under test roots.
//!
//! [OUTPUT]
//! Verifies command help, init bootstrap behavior, exit behavior, and persisted trigger list or toggle CLI actions.
//!
//! [ROLE]
//! Covers the user-facing CLI surface as an integration boundary.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::thread;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};

const SERVE_OWNER_ID_PREFIX: &str = "chainbot-serve-pid-";

fn acquire_fixture_lock() -> MutexGuard<'static, ()> {
    fixture_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

#[test]
fn help_lists_expected_commands() {
    let _lock = acquire_fixture_lock();
    ensure_basic_root_fixture();

    let output = Command::new(chainbot_bin())
        .arg("--help")
        .output()
        .expect("chainbot --help should execute");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");

    assert!(stdout.contains("validate"));
    assert!(stdout.contains("version"));
    assert!(stdout.contains("init"));
    assert!(stdout.contains("status"));
    assert!(stdout.contains("observe"));
    assert!(stdout.contains("catalog"));
    assert!(stdout.contains("plugin"));
    assert!(stdout.contains("stop"));
    assert!(stdout.contains("trigger"));
    assert!(stdout.contains("serve"));
    assert!(stdout.contains("run"));
    assert!(stdout.contains("list-runs"));
    assert!(stderr.is_empty());
}

#[test]
fn help_plugin_includes_source_install_guidance() {
    let _lock = acquire_fixture_lock();
    ensure_basic_root_fixture();

    let output = Command::new(chainbot_bin())
        .args(["help", "plugin"])
        .output()
        .expect("chainbot help plugin should execute");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");

    assert!(stdout.contains("chainbot plugin source list github <owner>/<repo>"));
    assert!(stdout.contains("chainbot plugin install <github|git> <target>"));
    assert!(stdout.contains("--ref"));
    assert!(stdout.contains("--plugin"));
    assert!(stdout.contains("--force"));
    assert!(stdout.contains("github"));
    assert!(stdout.contains("git"));
    assert!(stderr.is_empty());
}

#[test]
fn help_plugin_and_catalog_keep_package_centric_mcp_guidance() {
    let _lock = acquire_fixture_lock();
    ensure_basic_root_fixture();

    let plugin_output = Command::new(chainbot_bin())
        .args(["help", "plugin"])
        .output()
        .expect("chainbot help plugin should execute");

    assert!(plugin_output.status.success());
    let plugin_stdout = String::from_utf8(plugin_output.stdout).expect("stdout should be UTF-8");
    let plugin_stderr = String::from_utf8(plugin_output.stderr).expect("stderr should be UTF-8");
    assert!(plugin_stdout
        .contains("catalog remains the installed-capability surface for the current root"));
    assert!(plugin_stdout.contains("chainbot plugin source list"));
    assert!(plugin_stdout.contains("chainbot plugin install"));
    assert!(!plugin_stdout.contains("direct-connect"));
    assert!(!plugin_stdout.contains("server add"));
    assert!(!plugin_stdout.contains("server list"));
    assert!(!plugin_stdout.contains("server remove"));
    assert!(plugin_stderr.is_empty());

    let catalog_output = Command::new(chainbot_bin())
        .args(["help", "catalog"])
        .output()
        .expect("chainbot help catalog should execute");

    assert!(catalog_output.status.success());
    let catalog_stdout = String::from_utf8(catalog_output.stdout).expect("stdout should be UTF-8");
    let catalog_stderr = String::from_utf8(catalog_output.stderr).expect("stderr should be UTF-8");
    assert!(catalog_stdout.contains("installed plugin manifests when a root is available"));
    assert!(catalog_stdout.contains("chainbot catalog list"));
    assert!(!catalog_stdout.contains("direct-connect"));
    assert!(!catalog_stdout.contains("server add"));
    assert!(!catalog_stdout.contains("server list"));
    assert!(!catalog_stdout.contains("server remove"));
    assert!(catalog_stderr.is_empty());
}

#[test]
fn help_catalog_includes_discovery_guidance() {
    let _lock = acquire_fixture_lock();
    ensure_basic_root_fixture();

    let output = Command::new(chainbot_bin())
        .args(["help", "catalog"])
        .output()
        .expect("chainbot help catalog should execute");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");

    assert!(stdout.contains("Discover builtin capabilities and installed plugin contracts"));
    assert!(stdout.contains("chainbot catalog list"));
    assert!(stdout.contains("chainbot catalog show <reference>"));
    assert!(stderr.is_empty());
}

#[test]
fn help_catalog_distinguishes_trigger_lifecycle_models() {
    let _lock = acquire_fixture_lock();
    ensure_basic_root_fixture();

    let output = Command::new(chainbot_bin())
        .args(["help", "catalog"])
        .output()
        .expect("chainbot help catalog should execute");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");

    assert!(stdout.contains("process_short_lived"));
    assert!(stdout.contains("wasm_daemon_persistent_session"));
    assert!(stdout.contains("lifecycle"));
    assert!(stderr.is_empty());
}

#[test]
fn version_command_prints_running_release() {
    let _lock = acquire_fixture_lock();
    ensure_basic_root_fixture();

    let output = Command::new(chainbot_bin())
        .arg("version")
        .output()
        .expect("chainbot version should execute");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");

    assert_eq!(
        stdout.trim(),
        format!("chainbot {}", env!("CARGO_PKG_VERSION"))
    );
    assert!(stderr.is_empty());
}

#[test]
fn version_flag_prints_running_release() {
    let _lock = acquire_fixture_lock();
    ensure_basic_root_fixture();

    let output = Command::new(chainbot_bin())
        .arg("--version")
        .output()
        .expect("chainbot --version should execute");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");

    assert_eq!(
        stdout.trim(),
        format!("chainbot {}", env!("CARGO_PKG_VERSION"))
    );
    assert!(stderr.is_empty());
}

#[test]
fn help_init_includes_bootstrap_guidance() {
    let _lock = acquire_fixture_lock();
    ensure_basic_root_fixture();

    let output = Command::new(chainbot_bin())
        .args(["help", "init"])
        .output()
        .expect("chainbot help init should execute");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");

    assert!(stdout.contains("chainbot.toml"));
    assert!(stdout.contains("Bootstrap a minimal ChainBot root"));
    assert!(stdout.contains("CHAINBOT_CONFIG_DIR=/tmp/demo-root chainbot init"));
    assert!(stderr.is_empty());
}

#[test]
fn help_version_includes_release_guidance() {
    let _lock = acquire_fixture_lock();
    ensure_basic_root_fixture();

    let output = Command::new(chainbot_bin())
        .args(["help", "version"])
        .output()
        .expect("chainbot help version should execute");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");

    assert!(stdout.contains("Print the running ChainBot version"));
    assert!(stdout.contains("chainbot --version"));
    assert!(stderr.is_empty());
}

#[test]
fn help_status_includes_skill_guidance() {
    let _lock = acquire_fixture_lock();
    ensure_basic_root_fixture();

    let output = Command::new(chainbot_bin())
        .args(["help", "status"])
        .output()
        .expect("chainbot help status should execute");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");

    assert!(stdout.contains("Use when:"));
    assert!(stdout.contains("chainbot status --json"));
    assert!(stdout.contains("See also:"));
    assert!(stderr.is_empty());
}

#[test]
fn help_observe_includes_history_guidance() {
    let _lock = acquire_fixture_lock();
    ensure_basic_root_fixture();

    let output = Command::new(chainbot_bin())
        .args(["help", "observe"])
        .output()
        .expect("chainbot help observe should execute");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");

    assert!(stdout.contains("Inspect persisted trigger events"));
    assert!(stdout.contains("chainbot observe --json"));
    assert!(stdout.contains("--trigger-id tr-market"));
    assert!(stderr.is_empty());
}

#[test]
fn help_stop_includes_shutdown_guidance() {
    let _lock = acquire_fixture_lock();
    ensure_basic_root_fixture();

    let output = Command::new(chainbot_bin())
        .args(["help", "stop"])
        .output()
        .expect("chainbot help stop should execute");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");

    assert!(stdout.contains("Request graceful daemon shutdown"));
    assert!(stdout.contains("chainbot stop"));
    assert!(stderr.is_empty());
}

#[test]
fn help_trigger_includes_toggle_guidance() {
    let _lock = acquire_fixture_lock();
    ensure_basic_root_fixture();

    let output = Command::new(chainbot_bin())
        .args(["help", "trigger"])
        .output()
        .expect("chainbot help trigger should execute");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");

    assert!(stdout.contains("Inspect or persist trigger package state"));
    assert!(stdout.contains("chainbot trigger list"));
    assert!(stdout.contains("chainbot trigger enable tr-market"));
    assert!(stderr.is_empty());
}

#[test]
fn help_validate_includes_config_examples() {
    let _lock = acquire_fixture_lock();
    ensure_basic_root_fixture();

    let output = Command::new(chainbot_bin())
        .args(["help", "validate"])
        .output()
        .expect("chainbot help validate should execute");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");

    assert!(stdout.contains("Root config example:"));
    assert!(stdout.contains("Workflow package example:"));
    assert!(stdout.contains("Trigger package example:"));
    assert!(stdout.contains("Webhook trigger example:"));
    assert!(stdout.contains("WebSocket trigger example:"));
    assert!(stdout.contains("Plugin package example:"));
    assert!(stdout.contains("chainbot.toml"));
    assert!(stdout.contains("idempotency_header = \"x-event-id\""));
    assert!(stdout.contains("idle_timeout_ms = 30000"));
    assert!(stderr.is_empty());
}

#[test]
fn help_serve_includes_ingress_trigger_examples() {
    let _lock = acquire_fixture_lock();
    ensure_basic_root_fixture();

    let output = Command::new(chainbot_bin())
        .args(["help", "serve"])
        .output()
        .expect("chainbot help serve should execute");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");

    assert!(stdout.contains("Webhook trigger example:"));
    assert!(stdout.contains("WebSocket trigger example:"));
    assert!(stdout.contains("source = \"webhook\""));
    assert!(stdout.contains("source = \"websocket\""));
    assert!(stderr.is_empty());
}

#[test]
fn init_creates_minimal_root_and_validate_accepts_it() {
    let _lock = acquire_fixture_lock();
    let root = unique_root("init-minimal");
    let _ = fs::remove_dir_all(&root);

    let init_output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", &root)
        .arg("init")
        .output()
        .expect("init should execute");

    assert!(init_output.status.success());

    let init_stdout = String::from_utf8(init_output.stdout).expect("stdout should be UTF-8");
    let init_stderr = String::from_utf8(init_output.stderr).expect("stderr should be UTF-8");
    assert!(init_stdout.contains("init completed:"));
    assert!(init_stdout.contains(root.to_string_lossy().as_ref()));
    assert!(init_stderr.is_empty());

    assert!(!root.join("config").exists());
    assert!(root.join("workflows").is_dir());
    assert!(root.join("triggers").is_dir());
    assert!(root.join("plugins").join("bin").is_dir());
    assert!(root.join("secrets").is_dir());
    assert!(root.join("state").is_dir());

    let root_config = fs::read_to_string(root.join("chainbot.toml"))
        .expect("root config should be created by init");
    assert!(root_config.contains("manifest_version = \"2.0.0\""));
    assert!(root_config.contains(&format!(
        "chainbot_version = \"{}\"",
        env!("CARGO_PKG_VERSION")
    )));
    assert!(root_config.contains("profile = \"default\""));
    assert!(root_config.contains("workflows_dir = \"workflows\""));
    assert!(!root_config.contains("plugins/manifests/*.toml"));

    let validate_output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", &root)
        .arg("validate")
        .output()
        .expect("validate should execute after init");

    assert!(validate_output.status.success());
    let validate_stdout =
        String::from_utf8(validate_output.stdout).expect("stdout should be UTF-8");
    let validate_stderr =
        String::from_utf8(validate_output.stderr).expect("stderr should be UTF-8");
    assert!(validate_stdout.contains("validated root:"));
    assert!(!validate_stdout.contains("legacy layout detected:"));
    assert!(validate_stderr.is_empty());
}

#[test]
fn init_is_idempotent_when_root_already_exists() {
    let _lock = acquire_fixture_lock();
    let root = unique_root("init-idempotent");
    let _ = fs::remove_dir_all(&root);

    let first_output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", &root)
        .arg("init")
        .output()
        .expect("first init should execute");
    assert!(first_output.status.success());

    let second_output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", &root)
        .arg("init")
        .output()
        .expect("second init should execute");
    assert!(second_output.status.success());

    let stdout = String::from_utf8(second_output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(second_output.stderr).expect("stderr should be UTF-8");
    assert!(stdout.contains("reused:"));
    assert!(stdout.contains(root.join("chainbot.toml").to_string_lossy().as_ref()));
    assert!(stderr.is_empty());
}

#[test]
fn init_respects_existing_root_path_overrides() {
    let _lock = acquire_fixture_lock();
    let root = unique_root("init-overrides");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("root directory should be creatable");
    fs::write(
        root.join("chainbot.toml"),
        &format!(
            "manifest_version = \"2.0.0\"\nchainbot_version = \"{}\"\nprofile = \"custom\"\nsecret_refs = []\n\n[paths]\nworkflows_dir = \"defs/workflows\"\ntriggers_dir = \"defs/triggers\"\nplugins_dir = \"extensions\"\nsecrets_dir = \"vault\"\nstate_dir = \"runtime\"\n\n[storage]\nmode = \"local\"\n\n[storage.local]\ndatabase_path = \"runtime/runtime.sqlite3\"\n",
            env!("CARGO_PKG_VERSION")
        ),
    )
    .expect("custom root config should be writable");

    let output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", &root)
        .arg("init")
        .output()
        .expect("init should execute with existing overrides");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(stdout.contains(
        root.join("defs")
            .join("workflows")
            .to_string_lossy()
            .as_ref()
    ));
    assert!(stderr.is_empty());

    assert!(root.join("defs").join("workflows").is_dir());
    assert!(root.join("defs").join("triggers").is_dir());
    assert!(root.join("extensions").is_dir());
    assert!(root.join("extensions").join("bin").is_dir());
    assert!(root.join("vault").is_dir());
    assert!(root.join("runtime").is_dir());

    let validate_output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", &root)
        .arg("validate")
        .output()
        .expect("validate should execute after override init");
    assert!(validate_output.status.success());
}

#[test]
fn init_prefers_chainbot_toml_when_both_root_config_paths_exist() {
    let _lock = acquire_fixture_lock();
    let root = unique_root("init-dual-root-config");
    let _ = fs::remove_dir_all(&root);

    fs::create_dir_all(&root).expect("root directory should be creatable");
    fs::write(
        root.join("chainbot.toml"),
        &format!(
            "manifest_version = \"2.0.0\"\nchainbot_version = \"{}\"\nprofile = \"new\"\nsecret_refs = []\n\n[paths]\nplugins_dir = \"new-plugins\"\n\n[storage]\nmode = \"local\"\n\n[storage.local]\ndatabase_path = \"state/runtime.sqlite3\"\n",
            env!("CARGO_PKG_VERSION")
        ),
    )
    .expect("new root config should be writable");

    let output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", &root)
        .arg("init")
        .output()
        .expect("init should execute with dual root configs");

    assert!(output.status.success());
    assert!(root.join("new-plugins").join("bin").is_dir());
}

#[test]
fn validate_accepts_basic_root() {
    let _lock = acquire_fixture_lock();
    ensure_basic_root_fixture();

    let output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", basic_root())
        .arg("validate")
        .output()
        .expect("validate should execute");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");

    assert!(stdout.contains("validated root:"));
    assert!(stderr.is_empty());
}

#[test]
fn validate_surfaces_legacy_node_reference_in_human_and_json_output() {
    let _lock = acquire_fixture_lock();
    ensure_basic_root_fixture();
    fs::write(
        basic_root()
            .join("workflows")
            .join("wf-alpha")
            .join("config.toml"),
        "[workflow]\nmanifest_version = \"2.0.0\"\nid = \"wf-alpha\"\nname = \"alpha\"\n\n[[nodes]]\nmanifest_version = \"2.0.0\"\nid = \"node-1\"\nkind = \"plugin\"\nplugin = \"quote-plugin\"\noperation = \"normalize\"\ndepends_on = []\n\n[[nodes]]\nmanifest_version = \"2.0.0\"\nid = \"node-2\"\nkind = \"plugin\"\nplugin = \"quote-plugin\"\noperation = \"normalize\"\ndepends_on = [\"node-1\"]\n\n[[nodes.inputs]]\ntarget = \"price\"\nsource = \"node.price\"\n",
    )
    .expect("legacy workflow fixture should be writable");
    let legacy_plugin_root = basic_root().join("plugins").join("legacy-wasm");
    fs::create_dir_all(&legacy_plugin_root)
        .expect("legacy Wasm plugin directory should be creatable");
    fs::write(
        legacy_plugin_root.join("config.toml"),
        "manifest_version = \"2.0.0\"\nplugin_id = \"legacy-wasm\"\nkind = \"external_trigger\"\nentrypoint = \"trigger.exec.v1\"\ncapabilities = [\"trigger.listen.event\"]\n\n[trigger_runtime]\nlifecycle = \"wasm_daemon_persistent_session\"\npush_callback = \"host_callback\"\ndurable_ack = \"after_store_persist\"\nhost_error_categories = [\"transport\", \"protocol_contract\", \"plugin_fatal\"]\nmodule = \"bin/external_trigger.wasm\"\n\n[event_schema]\nsummary = \"legacy Wasm\"\nfields = [\"price\"]\n",
    )
    .expect("legacy Wasm manifest should be writable");

    let human = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", basic_root())
        .arg("validate")
        .output()
        .expect("human validation should execute");
    assert!(human.status.success());
    let human_stdout = String::from_utf8(human.stdout).expect("stdout should be UTF-8");
    assert!(human_stdout.contains("warning[legacy_node_output_reference]"));
    assert!(human_stdout.contains("node.<producer_id>.price"));
    assert!(human_stdout.contains("warning[legacy_wasm_core_v0_abi]"));

    let json = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", basic_root())
        .args(["validate", "--json"])
        .output()
        .expect("JSON validation should execute");
    assert!(json.status.success());
    let payload: serde_json::Value =
        serde_json::from_slice(&json.stdout).expect("validation output should be JSON");
    assert_eq!(payload["valid"], true);
    let warnings = payload["warnings"]
        .as_array()
        .expect("warnings should be an array");
    assert_eq!(warnings.len(), 2);
    let node_warning = warnings
        .iter()
        .find(|warning| warning["code"] == "legacy_node_output_reference")
        .expect("legacy node warning should exist");
    assert_eq!(node_warning["consumer_node_id"], "node-2");
    assert_eq!(node_warning["original_reference"], "node.price");
    let wasm_warning = warnings
        .iter()
        .find(|warning| warning["code"] == "legacy_wasm_core_v0_abi")
        .expect("legacy Wasm warning should exist");
    assert_eq!(wasm_warning["plugin_id"], "legacy-wasm");
    assert_eq!(wasm_warning["original_reference"], "<missing>");

    fs::remove_dir_all(legacy_plugin_root).expect("legacy Wasm fixture should be removable");
    ensure_basic_root_fixture();
}

#[test]
fn validate_accepts_curated_examples() {
    let single_workflow_root = workspace_root().join("examples").join("single-workflow");
    let core_builtins_root = workspace_root().join("examples").join("core-builtins");
    let builtin_triggers_root = workspace_root().join("examples").join("builtin-triggers");
    let workflow_composition_root = workspace_root()
        .join("examples")
        .join("workflow-composition");
    let plugin_integrations_root = workspace_root()
        .join("examples")
        .join("plugin-integrations");
    let http_plugin_integrations_root = workspace_root()
        .join("examples")
        .join("http-plugin-integrations");
    let eth_plugin_integrations_root = workspace_root()
        .join("examples")
        .join("eth-plugin-integrations");
    let solana_plugin_integrations_root = workspace_root()
        .join("examples")
        .join("solana-plugin-integrations");
    let custom_paths_root = workspace_root().join("examples").join("custom-paths");

    for root in [
        single_workflow_root,
        core_builtins_root,
        builtin_triggers_root,
        workflow_composition_root,
        plugin_integrations_root,
        http_plugin_integrations_root,
        eth_plugin_integrations_root,
        solana_plugin_integrations_root,
        custom_paths_root,
    ] {
        let output = Command::new(chainbot_bin())
            .env("CHAINBOT_CONFIG_DIR", &root)
            .arg("validate")
            .output()
            .expect("validate should execute for curated example root");

        assert!(
            output.status.success(),
            "example root should validate: {}",
            root.display()
        );

        let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
        let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");

        assert!(stdout.contains("validated root:"));
        assert!(stderr.is_empty());
    }
}

#[test]
fn list_runs_prints_empty_list_for_basic_root() {
    let _lock = acquire_fixture_lock();
    ensure_basic_root_fixture();

    let output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", basic_root())
        .arg("list-runs")
        .output()
        .expect("list-runs should execute");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");

    assert_eq!(stdout.trim(), "[]");
    assert!(stderr.is_empty());
}

#[test]
fn status_prints_human_summary_for_basic_root() {
    let _lock = acquire_fixture_lock();
    ensure_basic_root_fixture();

    let output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", basic_root())
        .arg("status")
        .output()
        .expect("status should execute");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");

    assert!(stdout.contains("Root"));
    assert!(stdout.contains("serve: idle"));
    assert!(stdout.contains("wf-alpha"));
    assert!(stdout.contains("tr-market"));
    assert!(!stdout.contains("Legacy Layout"));
    assert!(stdout.contains("Plugins"));
    assert!(stdout.contains("installed=1 builtin=1 external_node=0 external_trigger=0"));
    assert!(stderr.is_empty());
}

#[test]
fn observe_json_reports_recent_runtime_history() {
    let _lock = acquire_fixture_lock();
    ensure_basic_root_fixture();
    let root = basic_root().to_path_buf();
    upsert_run_summary(
        &root,
        "observe-run-1",
        "wf-alpha",
        "succeeded",
        1_710_555_000_000,
        Some(1_710_555_001_000),
    );
    append_workflow_log_entry(
        &root,
        "observe-run-1",
        "run_finished",
        "fixture completed",
        1_710_555_001_000,
    );
    insert_trigger_event_record(
        &root,
        "tr-market",
        1,
        "observe-run-1",
        "wf-alpha",
        "event-observe-1",
        "builtin.market",
        1_710_555_000_500,
        serde_json::json!({"price": 101}),
    );

    let output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", &root)
        .args(["observe", "--json", "--limit", "5"])
        .output()
        .expect("observe should execute");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    let payload: serde_json::Value =
        serde_json::from_str(&stdout).expect("observe JSON should decode");

    assert_eq!(payload["summary"]["requested_limit"], 5);
    assert_eq!(payload["summary"]["archived"]["run_summaries"], 0);
    assert_eq!(payload["runs"][0]["run_id"], "observe-run-1");
    assert_eq!(payload["workflow_logs"][0]["run_id"], "observe-run-1");
    assert_eq!(payload["trigger_events"][0]["trigger_id"], "tr-market");
    assert!(stderr.is_empty());
}

#[test]
fn repeated_status_json_reads_remain_read_only() {
    let _lock = acquire_fixture_lock();
    ensure_basic_root_fixture();
    let root = basic_root().to_path_buf();
    upsert_run_summary(
        &root,
        "status-soak-run-1",
        "wf-alpha",
        "succeeded",
        1_710_556_000_000,
        Some(1_710_556_001_000),
    );

    for _ in 0..64 {
        let output = Command::new(chainbot_bin())
            .env("CHAINBOT_CONFIG_DIR", &root)
            .args(["status", "--json"])
            .output()
            .expect("status --json should execute repeatedly");
        assert!(output.status.success());
        let payload: serde_json::Value =
            serde_json::from_slice(&output.stdout).expect("status JSON should decode");
        assert_eq!(payload["summary"]["run_count"], 1);
        assert_eq!(payload["serve"]["state"], "idle");
    }

    assert_eq!(count_run_summaries(&root), 1);
    assert_eq!(count_trigger_event_records(&root), 0);
}

#[test]
fn invalid_toml_validate_reports_line_context() {
    let _lock = acquire_fixture_lock();
    let root = unique_root("validate-invalid-toml");
    let _ = fs::remove_dir_all(&root);

    fs::create_dir_all(root.join("workflows").join("wf-alpha"))
        .expect("workflow package directory should be creatable");
    fs::create_dir_all(root.join("triggers").join("tr-market"))
        .expect("trigger package directory should be creatable");
    fs::create_dir_all(root.join("plugins").join("quote-plugin"))
        .expect("plugin package directory should be creatable");
    fs::create_dir_all(root.join("secrets")).expect("secrets directory should be creatable");
    fs::create_dir_all(root.join("state")).expect("state directory should be creatable");

    fs::write(
        root.join("chainbot.toml"),
        &format!(
            "manifest_version = \"2.0.0\"\nchainbot_version = \"{}\"\nprofile = \"broken\"\n\n[storage]\nmode = \"local\"\n\n[storage.local]\ndatabase_path = \"state/runtime.sqlite3\"\n",
            env!("CARGO_PKG_VERSION")
        ),
    )
    .expect("root config fixture should be writable");
    fs::write(
        root.join("workflows").join("wf-alpha").join("config.toml"),
        "[workflow]\nmanifest_version = \"2.0.0\"\nid = \"wf-alpha\n",
    )
    .expect("invalid workflow config should be writable");
    fs::write(
        root.join("triggers").join("tr-market").join("config.toml"),
        "manifest_version = \"2.0.0\"\ntrigger_id = \"tr-market\"\nkind = \"builtin\"\nsource = \"market_tick\"\nworkflow_id = \"wf-alpha\"\nenabled = true\n",
    )
    .expect("trigger fixture should be writable");
    fs::write(
        root.join("plugins").join("quote-plugin").join("config.toml"),
        "manifest_version = \"2.0.0\"\nplugin_id = \"quote-plugin\"\nkind = \"builtin\"\nentrypoint = \"plugins.quote\"\ncapabilities = [\"normalize\"]\n",
    )
    .expect("plugin fixture should be writable");

    let output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", &root)
        .arg("validate")
        .output()
        .expect("validate should execute for invalid TOML root");

    assert_eq!(output.status.code(), Some(3));

    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");

    assert!(stdout.is_empty());
    assert!(stderr.contains("definition file is invalid TOML"));
    assert!(stderr.contains("line:"));
    assert!(stderr.contains("column:"));
    assert!(stderr.contains("3 | id = \"wf-alpha"));
    assert!(stderr.contains("^"));
}

#[test]
fn unexpected_argument_reports_argument_position() {
    let _lock = acquire_fixture_lock();
    ensure_basic_root_fixture();

    let output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", basic_root())
        .args(["status", "unexpected"])
        .output()
        .expect("status should execute with bad argument");

    assert_eq!(output.status.code(), Some(2));
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");

    assert!(stdout.is_empty());
    assert!(stderr.contains("Unexpected argument #1 after `chainbot status`: `unexpected`"));
}

#[test]
fn invalid_help_topic_reports_help_argument_position() {
    let output = Command::new(chainbot_bin())
        .args(["help", "unknown-topic"])
        .output()
        .expect("help should execute with bad topic");

    assert_eq!(output.status.code(), Some(2));
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");

    assert!(stdout.is_empty());
    assert!(stderr
        .contains("Unsupported help topic at argument #1 after `chainbot help`: `unknown-topic`"));
}

#[test]
fn invalid_json_flag_value_reports_argument_position() {
    let _lock = acquire_fixture_lock();
    ensure_basic_root_fixture();

    let output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", basic_root())
        .args(["status", "--json=maybe"])
        .output()
        .expect("status should execute with bad json value");

    assert_eq!(output.status.code(), Some(2));
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");

    assert!(stdout.is_empty());
    assert!(
        stderr.contains("Unsupported --json value at argument #1 after `chainbot status`: `maybe`")
    );
}

#[test]
fn trigger_enable_and_disable_persist_trigger_state() {
    let _lock = acquire_fixture_lock();
    ensure_basic_root_fixture();

    let enable_output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", basic_root())
        .args(["trigger", "enable", "tr-market"])
        .output()
        .expect("trigger enable should execute");

    assert!(enable_output.status.success());
    let enable_stdout = String::from_utf8(enable_output.stdout).expect("stdout should be UTF-8");
    let enable_stderr = String::from_utf8(enable_output.stderr).expect("stderr should be UTF-8");
    assert!(enable_stdout.contains("trigger updated:"));
    assert!(enable_stdout.contains("state=enabled"));
    assert!(enable_stderr.is_empty());

    let trigger_config_path = basic_root()
        .join("triggers")
        .join("tr-market")
        .join("config.toml");
    let trigger_config = fs::read_to_string(&trigger_config_path)
        .expect("trigger config should remain readable after enable");
    assert!(trigger_config.contains("enabled = true"));

    let status_output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", basic_root())
        .arg("status")
        .output()
        .expect("status should execute after trigger enable");
    assert!(status_output.status.success());
    let status_stdout = String::from_utf8(status_output.stdout).expect("stdout should be UTF-8");
    assert!(status_stdout.contains("tr-market  enabled"));

    let disable_output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", basic_root())
        .args(["trigger", "disable", "tr-market"])
        .output()
        .expect("trigger disable should execute");

    assert!(disable_output.status.success());
    let disable_stdout = String::from_utf8(disable_output.stdout).expect("stdout should be UTF-8");
    let disable_stderr = String::from_utf8(disable_output.stderr).expect("stderr should be UTF-8");
    assert!(disable_stdout.contains("trigger updated:"));
    assert!(disable_stdout.contains("state=disabled"));
    assert!(disable_stderr.is_empty());

    let trigger_config = fs::read_to_string(&trigger_config_path)
        .expect("trigger config should remain readable after disable");
    assert!(trigger_config.contains("enabled = false"));
}

#[test]
fn trigger_list_reports_configured_triggers() {
    let _lock = acquire_fixture_lock();
    ensure_basic_root_fixture();

    let output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", basic_root())
        .args(["trigger", "list"])
        .output()
        .expect("trigger list should execute");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(stdout.contains("Triggers"));
    assert!(stdout.contains("tr-market"));
    assert!(stdout.contains("workflow=wf-alpha"));
    assert!(stdout.contains("source=market_tick"));
    assert!(stderr.is_empty());
}

#[test]
fn trigger_list_json_reports_machine_readable_payload() {
    let _lock = acquire_fixture_lock();
    ensure_basic_root_fixture();

    let output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", basic_root())
        .args(["trigger", "list", "--json"])
        .output()
        .expect("trigger list --json should execute");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    let payload = serde_json::from_str::<serde_json::Value>(&stdout)
        .expect("trigger list json payload should decode");

    assert_eq!(payload.as_array().map(Vec::len), Some(1));
    assert_eq!(payload[0]["manifest_version"], "2.0.0");
    assert_eq!(payload[0]["trigger_id"], "tr-market");
    assert_eq!(payload[0]["kind"], "builtin");
    assert_eq!(payload[0]["workflow_id"], "wf-alpha");
    assert_eq!(payload[0]["enabled"], false);
    assert_eq!(payload[0]["source"], "market_tick");
    assert_eq!(payload[0]["input_mapping"], serde_json::json!({}));
    assert_eq!(payload[0].get("package_root"), None);
    assert!(stderr.is_empty());
}

#[test]
fn trigger_list_only_requires_trigger_root_inputs() {
    let _lock = acquire_fixture_lock();
    let root = unique_root("trigger-minimal");
    let _ = fs::remove_dir_all(&root);

    fs::create_dir_all(root.join("triggers").join("tr-market"))
        .expect("trigger directory should be creatable");
    fs::write(
        root.join("chainbot.toml"),
        &format!(
            "manifest_version = \"2.0.0\"\nchainbot_version = \"{}\"\nprofile = \"minimal\"\nsecret_refs = []\n\n[storage]\nmode = \"local\"\n\n[storage.local]\ndatabase_path = \"state/runtime.sqlite3\"\n",
            env!("CARGO_PKG_VERSION")
        ),
    )
    .expect("root config should be writable");
    fs::write(
        root.join("triggers").join("tr-market").join("config.toml"),
        "manifest_version = \"2.0.0\"\ntrigger_id = \"tr-market\"\nkind = \"builtin\"\nsource = \"market_tick\"\nworkflow_id = \"wf-alpha\"\nenabled = false\n",
    )
    .expect("trigger config should be writable");

    let list_output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", &root)
        .args(["trigger", "list"])
        .output()
        .expect("trigger list should execute on minimal root");
    assert!(list_output.status.success());

    let list_stdout = String::from_utf8(list_output.stdout).expect("stdout should be UTF-8");
    let list_stderr = String::from_utf8(list_output.stderr).expect("stderr should be UTF-8");
    assert!(list_stdout.contains("tr-market"));
    assert!(list_stderr.is_empty());

    let enable_output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", &root)
        .args(["trigger", "enable", "tr-market"])
        .output()
        .expect("trigger enable should execute on minimal root");
    assert!(enable_output.status.success());
    let enable_stdout = String::from_utf8(enable_output.stdout).expect("stdout should be UTF-8");
    assert!(enable_stdout.contains("state=enabled"));

    let trigger_config =
        fs::read_to_string(root.join("triggers").join("tr-market").join("config.toml"))
            .expect("trigger config should remain readable after enable");
    assert!(trigger_config.contains("enabled = true"));
}

#[test]
fn trigger_enable_is_idempotent_when_state_already_matches() {
    let _lock = acquire_fixture_lock();
    ensure_basic_root_fixture();

    let first_output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", basic_root())
        .args(["trigger", "enable", "tr-market"])
        .output()
        .expect("first trigger enable should execute");
    assert!(first_output.status.success());

    let second_output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", basic_root())
        .args(["trigger", "enable", "tr-market"])
        .output()
        .expect("second trigger enable should execute");

    assert!(second_output.status.success());
    let stdout = String::from_utf8(second_output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(second_output.stderr).expect("stderr should be UTF-8");
    assert!(stdout.contains("trigger unchanged:"));
    assert!(stdout.contains("state=enabled"));
    assert!(stderr.is_empty());

    let trigger_config_path = basic_root()
        .join("triggers")
        .join("tr-market")
        .join("config.toml");
    let trigger_config = fs::read_to_string(&trigger_config_path)
        .expect("trigger config should remain readable after idempotent enable");
    assert!(trigger_config.contains("enabled = true"));
}

#[test]
fn status_json_reports_active_serve_lease() {
    let _lock = acquire_fixture_lock();
    ensure_basic_root_fixture();

    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after UNIX_EPOCH")
        .as_millis()
        .min(i64::MAX as u128) as i64;
    ensure_runtime_db_ready(&basic_root());
    let owner_id = format!("{SERVE_OWNER_ID_PREFIX}{}", std::process::id());
    upsert_serve_lease(&basic_root(), &owner_id, now_ms, now_ms + 60_000);

    let output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", basic_root())
        .args(["status", "--json"])
        .output()
        .expect("status json should execute");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    let payload = serde_json::from_str::<serde_json::Value>(&stdout)
        .expect("status json payload should decode");

    assert_eq!(payload["serve"]["state"], "active");
    assert_eq!(payload["serve"]["owner"], owner_id);
    assert_eq!(payload["root"]["profile"], "basic");
    assert_eq!(payload["summary"]["workflow_count"], 1);
    assert_eq!(payload["summary"]["trigger_count"], 1);
    assert!(stderr.is_empty());
}

#[test]
fn status_does_not_recover_or_mutate_incomplete_runs() {
    let _lock = acquire_fixture_lock();
    ensure_basic_root_fixture();

    upsert_run_summary(
        &basic_root(),
        "run-incomplete",
        "wf-alpha",
        "running",
        1_710_000_040_000,
        None,
    );

    let output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", basic_root())
        .arg("status")
        .output()
        .expect("status should execute without mutating incomplete runs");

    assert!(output.status.success());

    let status = read_run_status(&basic_root(), "run-incomplete")
        .expect("run summary should remain present after status");
    assert_eq!(status, "running");
}

#[test]
fn status_reads_trigger_snapshot_without_record_scan_side_effects() {
    let _lock = acquire_fixture_lock();
    ensure_basic_root_fixture();

    upsert_trigger_snapshot(
        &basic_root(),
        "tr-market",
        Some("event-snapshot"),
        Some(1_710_000_050_000),
        4,
    );

    let output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", basic_root())
        .args(["status", "--json"])
        .output()
        .expect("status json should execute with trigger snapshot only");

    assert!(output.status.success());
    let payload = serde_json::from_slice::<serde_json::Value>(&output.stdout)
        .expect("status json payload should decode");
    assert_eq!(payload["triggers"][0]["last_event_id"], "event-snapshot");
    assert_eq!(
        payload["triggers"][0]["last_accepted_at_ms"],
        1_710_000_050_000_i64
    );
}

#[test]
fn list_runs_does_not_recover_or_promote_staged_summaries() {
    let _lock = acquire_fixture_lock();
    ensure_basic_root_fixture();

    upsert_run_summary(
        &basic_root(),
        "run-committed",
        "wf-alpha",
        "running",
        1_710_000_060_000,
        None,
    );

    let output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", basic_root())
        .arg("list-runs")
        .output()
        .expect("list-runs should execute without recovery");

    assert!(output.status.success());
    let payload = serde_json::from_slice::<serde_json::Value>(&output.stdout)
        .expect("list-runs payload should decode");
    let runs = payload
        .as_array()
        .expect("list-runs should return a JSON array");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0]["run_id"], "run-committed");
    assert_eq!(runs[0]["status"], "running");
}

#[test]
fn invalid_root_returns_stable_user_facing_error() {
    let missing_root = workspace_root()
        .join("target")
        .join("test-roots")
        .join("missing-cli-root");

    let output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", &missing_root)
        .arg("validate")
        .output()
        .expect("validate should execute for invalid root");

    assert_eq!(output.status.code(), Some(3));

    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");

    assert!(stdout.is_empty());
    assert!(stderr.contains("error_code=validation_error"));
    assert!(stderr.contains("Root is missing required root directory:"));
    assert!(!stderr.contains("panicked at"));
    assert!(!stderr.contains("thread 'main'"));
}

#[test]
fn serve_starts_background_daemon_and_stop_shuts_it_down() {
    let _lock = acquire_fixture_lock();
    ensure_basic_root_fixture();

    let serve_output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", basic_root())
        .arg("serve")
        .output()
        .expect("serve should execute");
    assert!(serve_output.status.success());
    let serve_stdout = String::from_utf8(serve_output.stdout).expect("stdout should be UTF-8");
    let serve_stderr = String::from_utf8(serve_output.stderr).expect("stderr should be UTF-8");
    assert!(serve_stdout.contains("serve started: owner="));
    assert!(serve_stderr.is_empty());

    let status_output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", basic_root())
        .args(["status", "--json"])
        .output()
        .expect("status should execute after daemon start");
    assert!(status_output.status.success());
    let status_payload = serde_json::from_slice::<serde_json::Value>(&status_output.stdout)
        .expect("status json should decode after daemon start");
    assert_eq!(status_payload["serve"]["state"], "active");
    assert!(status_payload["serve"]["pid"].as_i64().is_some());
    assert!(status_payload["serve"]["started_at_ms"].as_i64().is_some());
    assert!(status_payload["serve"]["last_heartbeat_at_ms"]
        .as_i64()
        .is_some());

    let stop_output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", basic_root())
        .arg("stop")
        .output()
        .expect("stop should execute");
    assert!(stop_output.status.success());
    let stop_stdout = String::from_utf8(stop_output.stdout).expect("stdout should be UTF-8");
    let stop_stderr = String::from_utf8(stop_output.stderr).expect("stderr should be UTF-8");
    assert!(stop_stdout.contains("stop completed:"));
    assert!(stop_stderr.is_empty());

    let stopped_status_output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", basic_root())
        .args(["status", "--json"])
        .output()
        .expect("status should execute after stop");
    assert!(stopped_status_output.status.success());
    let stopped_status_payload =
        serde_json::from_slice::<serde_json::Value>(&stopped_status_output.stdout)
            .expect("status json should decode after stop");
    assert_eq!(stopped_status_payload["serve"]["state"], "idle");
}

#[test]
fn second_serve_returns_conflict_with_stable_error_code() {
    let _lock = acquire_fixture_lock();
    ensure_basic_root_fixture();

    let first_output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", basic_root())
        .arg("serve")
        .output()
        .expect("first serve should execute");
    assert!(first_output.status.success());

    let second_output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", basic_root())
        .arg("serve")
        .output()
        .expect("second serve should execute");
    assert_eq!(second_output.status.code(), Some(6));
    let second_stdout = String::from_utf8(second_output.stdout).expect("stdout should be UTF-8");
    let second_stderr = String::from_utf8(second_output.stderr).expect("stderr should be UTF-8");
    assert!(second_stdout.is_empty());
    assert!(second_stderr.contains("error_code=daemon_already_running"));

    let stop_output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", basic_root())
        .arg("stop")
        .output()
        .expect("stop should execute for cleanup");
    assert!(stop_output.status.success());
}

#[test]
fn serve_start_timeout_does_not_leave_a_ghost_daemon() {
    let _lock = acquire_fixture_lock();
    ensure_basic_root_fixture();

    let output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", basic_root())
        .env("CHAINBOT_TEST_DAEMON_START_DELAY_MS", "300")
        .env("CHAINBOT_TEST_DAEMON_START_ACK_TIMEOUT_MS", "100")
        .arg("serve")
        .output()
        .expect("serve should execute with delayed daemon start");

    assert_eq!(output.status.code(), Some(5));
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(stdout.is_empty());
    assert!(stderr.contains("error_code=daemon_start_failed"));

    let status_output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", basic_root())
        .args(["status", "--json"])
        .output()
        .expect("status should execute after failed daemon start");
    assert!(status_output.status.success());
    let status_payload = serde_json::from_slice::<serde_json::Value>(&status_output.stdout)
        .expect("status json should decode after failed daemon start");
    assert_eq!(status_payload["serve"]["state"], "idle");
}

#[test]
fn stop_during_startup_cleans_up_pending_daemon_launch() {
    let _lock = acquire_fixture_lock();
    ensure_basic_root_fixture();

    let serve_child = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", basic_root())
        .env("CHAINBOT_TEST_DAEMON_START_DELAY_MS", "300")
        .env("CHAINBOT_TEST_DAEMON_START_ACK_TIMEOUT_MS", "150")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .arg("serve")
        .spawn()
        .expect("serve should spawn for startup-stop race coverage");

    thread::sleep(Duration::from_millis(50));

    let stop_output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", basic_root())
        .arg("stop")
        .output()
        .expect("stop should execute during delayed startup");
    assert!(stop_output.status.success());
    let stop_stdout = String::from_utf8(stop_output.stdout).expect("stdout should be UTF-8");
    assert!(stop_stdout.contains("stop completed:"));

    let serve_output = serve_child
        .wait_with_output()
        .expect("serve child should finish after stop");
    assert_eq!(serve_output.status.code(), Some(5));
    let serve_stdout = String::from_utf8(serve_output.stdout).expect("stdout should be UTF-8");
    let serve_stderr = String::from_utf8(serve_output.stderr).expect("stderr should be UTF-8");
    assert!(
        serve_stderr.contains("error_code=daemon_start_failed")
            || serve_stdout.contains("error_code=daemon_start_failed")
    );

    let status_output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", basic_root())
        .args(["status", "--json"])
        .output()
        .expect("status should execute after startup-stop race");
    assert!(status_output.status.success());
    let status_payload = serde_json::from_slice::<serde_json::Value>(&status_output.stdout)
        .expect("status json should decode after startup-stop race");
    assert_eq!(status_payload["serve"]["state"], "idle");
}

#[test]
fn status_json_includes_plugin_summary_without_breaking_existing_fields() {
    let _lock = acquire_fixture_lock();
    ensure_basic_root_fixture();

    let output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", basic_root())
        .args(["status", "--json"])
        .output()
        .expect("status --json should execute");

    assert!(output.status.success());
    let payload = serde_json::from_slice::<serde_json::Value>(&output.stdout)
        .expect("status json should decode");
    assert_eq!(payload["root"]["profile"], "basic");
    assert_eq!(payload["plugins"]["installed_count"], 1);
    assert_eq!(payload["plugins"]["builtin_count"], 1);
    assert_eq!(payload["plugins"]["external_node_count"], 0);
    assert_eq!(payload["plugins"]["external_trigger_count"], 0);
}

fn chainbot_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_chainbot"))
}

fn basic_root() -> PathBuf {
    workspace_root()
        .join("target")
        .join("test-roots")
        .join("basic")
}

fn unique_root(label: &str) -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after UNIX_EPOCH")
        .as_nanos();
    workspace_root()
        .join("target")
        .join("test-roots")
        .join(format!("{label}-{suffix}"))
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

fn ensure_basic_root_fixture() {
    let root = basic_root();
    let state_root = root.join("state");

    let _ = fs::remove_dir_all(state_root.join("runs"));
    let _ = fs::remove_dir_all(state_root.join("triggers"));
    let _ = fs::remove_file(state_root.join("coordination.sqlite3"));
    let _ = fs::remove_file(state_root.join("runtime.sqlite3"));
    let _ = fs::remove_dir_all(root.join("plugins").join("legacy-wasm"));

    fs::create_dir_all(root.join("workflows")).expect("workflows directory should be creatable");
    fs::create_dir_all(root.join("triggers")).expect("triggers directory should be creatable");
    fs::create_dir_all(root.join("plugins").join("quote-plugin"))
        .expect("plugin package directory should be creatable");
    fs::create_dir_all(root.join("secrets")).expect("secrets directory should be creatable");
    fs::create_dir_all(&state_root).expect("state directory should be creatable");
    fs::create_dir_all(root.join("workflows").join("wf-alpha"))
        .expect("workflow package directory should be creatable");
    fs::create_dir_all(root.join("triggers").join("tr-market"))
        .expect("trigger package directory should be creatable");

    fs::write(
        root.join("chainbot.toml"),
        &format!(
            "manifest_version = \"2.0.0\"\nchainbot_version = \"{}\"\nprofile = \"basic\"\nsecret_refs = [\"secret://ops/slack/webhook\"]\n\n[storage]\nmode = \"local\"\n\n[storage.local]\ndatabase_path = \"state/runtime.sqlite3\"\n",
            env!("CARGO_PKG_VERSION")
        ),
    )
    .expect("root config fixture should be writable");

    fs::write(
        root.join("workflows").join("wf-alpha").join("config.toml"),
        "[workflow]\nmanifest_version = \"2.0.0\"\nid = \"wf-alpha\"\nname = \"alpha\"\n\n[[nodes]]\nmanifest_version = \"2.0.0\"\nid = \"node-1\"\nkind = \"plugin\"\nplugin = \"quote-plugin\"\noperation = \"normalize\"\ndepends_on = []\n",
    )
    .expect("workflow fixture should be writable");

    fs::write(
        root.join("triggers").join("tr-market").join("config.toml"),
        "manifest_version = \"2.0.0\"\ntrigger_id = \"tr-market\"\nkind = \"builtin\"\nsource = \"market_tick\"\nworkflow_id = \"wf-alpha\"\nenabled = false\n",
    )
    .expect("trigger fixture should be writable");

    fs::write(
        root.join("plugins").join("quote-plugin").join("config.toml"),
        "manifest_version = \"2.0.0\"\nplugin_id = \"quote-plugin\"\nkind = \"builtin\"\nentrypoint = \"plugins.quote\"\ncapabilities = [\"normalize\"]\n",
    )
    .expect("plugin fixture should be writable");
}

fn runtime_db_path(root: &Path) -> PathBuf {
    root.join("state").join("runtime.sqlite3")
}

fn ensure_runtime_db_ready(root: &Path) {
    let output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", root)
        .args(["status", "--json"])
        .output()
        .expect("status --json should execute to prepare runtime DB");
    assert!(output.status.success());
}

fn open_runtime_db(root: &Path) -> Connection {
    ensure_runtime_db_ready(root);
    Connection::open(runtime_db_path(root)).expect("runtime sqlite database should open")
}

fn upsert_run_summary(
    root: &Path,
    run_id: &str,
    workflow_id: &str,
    status: &str,
    started_at_ms: i64,
    finished_at_ms: Option<i64>,
) {
    let connection = open_runtime_db(root);
    connection
        .execute(
            "INSERT INTO run_summaries (schema_version, run_id, workflow_id, status, started_at_ms, finished_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(run_id)
             DO UPDATE SET schema_version = excluded.schema_version,
                           workflow_id = excluded.workflow_id,
                           status = excluded.status,
                           started_at_ms = excluded.started_at_ms,
                           finished_at_ms = excluded.finished_at_ms",
            params!["1.0.0", run_id, workflow_id, status, started_at_ms, finished_at_ms],
        )
        .expect("run summary should upsert");
}

fn append_workflow_log_entry(
    root: &Path,
    run_id: &str,
    event: &str,
    message: &str,
    occurred_at_ms: i64,
) {
    let connection = open_runtime_db(root);
    let sequence: i64 = connection
        .query_row(
            "SELECT COALESCE(MAX(sequence), 0) + 1 FROM workflow_runtime_logs WHERE run_id = ?1",
            params![run_id],
            |row| row.get(0),
        )
        .expect("next workflow log sequence should query");
    connection
        .execute(
            "INSERT INTO workflow_runtime_logs (run_id, sequence, event, message, occurred_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![run_id, sequence, event, message, occurred_at_ms],
        )
        .expect("workflow log should insert");
}

fn insert_trigger_event_record(
    root: &Path,
    trigger_id: &str,
    sequence: i64,
    run_id: &str,
    workflow_id: &str,
    event_id: &str,
    source: &str,
    accepted_at_ms: i64,
    payload: serde_json::Value,
) {
    let connection = open_runtime_db(root);
    connection
        .execute(
            "INSERT INTO trigger_event_records (
                trigger_id, sequence, schema_version, run_id, workflow_id, event_id,
                checkpoint, source, accepted_at_ms, payload_json,
                dedup_key, dedup_expires_at_ms, cooldown_key, cooldown_expires_at_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, ?7, ?8, ?9, NULL, NULL, NULL, NULL)",
            params![
                trigger_id,
                sequence,
                "1.0.0",
                run_id,
                workflow_id,
                event_id,
                source,
                accepted_at_ms,
                serde_json::to_string(&payload).expect("trigger payload should serialize")
            ],
        )
        .expect("trigger event should insert");
}

fn upsert_trigger_snapshot(
    root: &Path,
    trigger_id: &str,
    last_event_id: Option<&str>,
    last_accepted_at_ms: Option<i64>,
    last_sequence: i64,
) {
    let connection = open_runtime_db(root);
    let accepted = last_event_id
        .map(|event_id| vec![event_id.to_owned()])
        .unwrap_or_default();
    connection
        .execute(
            "INSERT INTO trigger_snapshots (
                trigger_id, schema_version, last_event_id, last_accepted_at_ms, last_sequence,
                accepted_event_ids_json, dedup_tokens_json, cooldown_tokens_json
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(trigger_id)
             DO UPDATE SET schema_version = excluded.schema_version,
                           last_event_id = excluded.last_event_id,
                           last_accepted_at_ms = excluded.last_accepted_at_ms,
                           last_sequence = excluded.last_sequence,
                           accepted_event_ids_json = excluded.accepted_event_ids_json,
                           dedup_tokens_json = excluded.dedup_tokens_json,
                           cooldown_tokens_json = excluded.cooldown_tokens_json",
            params![
                trigger_id,
                "1.0.0",
                last_event_id,
                last_accepted_at_ms,
                last_sequence,
                serde_json::to_string(&accepted).expect("accepted ids should serialize"),
                "[]",
                "[]"
            ],
        )
        .expect("trigger snapshot should upsert");
}

fn upsert_serve_lease(root: &Path, owner_id: &str, acquired_at_ms: i64, expires_at_ms: i64) {
    let connection = open_runtime_db(root);
    connection
        .execute(
            "INSERT INTO serve_leases (lease_key, owner_id, acquired_at_ms, expires_at_ms)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(lease_key)
             DO UPDATE SET owner_id = excluded.owner_id,
                           acquired_at_ms = excluded.acquired_at_ms,
                           expires_at_ms = excluded.expires_at_ms",
            params!["serve", owner_id, acquired_at_ms, expires_at_ms],
        )
        .expect("serve lease should upsert");
}

fn count_run_summaries(root: &Path) -> usize {
    let connection = open_runtime_db(root);
    connection
        .query_row("SELECT COUNT(*) FROM run_summaries", [], |row| {
            row.get::<_, i64>(0)
        })
        .expect("run summary count should query") as usize
}

fn count_trigger_event_records(root: &Path) -> usize {
    let connection = open_runtime_db(root);
    connection
        .query_row("SELECT COUNT(*) FROM trigger_event_records", [], |row| {
            row.get::<_, i64>(0)
        })
        .expect("trigger event count should query") as usize
}

fn read_run_status(root: &Path, run_id: &str) -> Option<String> {
    let connection = open_runtime_db(root);
    connection
        .query_row(
            "SELECT status FROM run_summaries WHERE run_id = ?1",
            params![run_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .expect("run status should query")
}

fn fixture_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}
