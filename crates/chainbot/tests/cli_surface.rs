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
use std::process::Command;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use chainbot::config::{RuntimeStorageBackend, RuntimeStorageConfig};
use chainbot::state::{RunRecordSummary, RunStatus, TriggerSnapshotRecord, SERVE_OWNER_ID_PREFIX};
use chainbot::state_db::RuntimeStateStore;

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
    assert!(stdout.contains("trigger"));
    assert!(stdout.contains("serve"));
    assert!(stdout.contains("run"));
    assert!(stdout.contains("list-runs"));
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
    assert!(stdout.contains("Plugin package example:"));
    assert!(stdout.contains("chainbot.toml"));
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
fn validate_accepts_curated_examples() {
    let single_workflow_root = workspace_root().join("examples").join("single-workflow");
    let builtin_triggers_root = workspace_root().join("examples").join("builtin-triggers");
    let workflow_composition_root = workspace_root()
        .join("examples")
        .join("workflow-composition");
    let plugin_integrations_root = workspace_root()
        .join("examples")
        .join("plugin-integrations");
    let custom_paths_root = workspace_root().join("examples").join("custom-paths");

    for root in [
        single_workflow_root,
        builtin_triggers_root,
        workflow_composition_root,
        plugin_integrations_root,
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
    assert!(stderr.is_empty());
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
    let mut state_store = RuntimeStateStore::open(&storage_config_for_root(&basic_root()), now_ms)
        .expect("runtime state store should open for status fixture");
    let owner_id = format!("{SERVE_OWNER_ID_PREFIX}{}", std::process::id());
    state_store
        .try_acquire_serve_lease(&owner_id, now_ms, 60_000)
        .expect("serve lease should be acquired for status output");

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

    let mut state_store =
        RuntimeStateStore::open(&storage_config_for_root(&basic_root()), 1_710_000_040_000)
            .expect("runtime state store should initialize for status mutation test");
    state_store
        .write_run_summary(&RunRecordSummary {
            schema_version: "1.0.0".to_string(),
            run_id: "run-incomplete".to_string(),
            workflow_id: "wf-alpha".to_string(),
            status: RunStatus::Running,
            started_at_ms: 1_710_000_040_000,
            finished_at_ms: None,
        })
        .expect("incomplete run summary should persist");

    let output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", basic_root())
        .arg("status")
        .output()
        .expect("status should execute without mutating incomplete runs");

    assert!(output.status.success());

    let summaries = state_store
        .list_run_summaries()
        .expect("run summaries should remain readable after status");
    let summary = summaries
        .into_iter()
        .find(|summary| summary.run_id == "run-incomplete")
        .expect("run summary should remain present after status");
    assert_eq!(summary.status, RunStatus::Running);
}

#[test]
fn status_reads_trigger_snapshot_without_record_scan_side_effects() {
    let _lock = acquire_fixture_lock();
    ensure_basic_root_fixture();

    let mut state_store =
        RuntimeStateStore::open(&storage_config_for_root(&basic_root()), 1_710_000_050_000)
            .expect("runtime state store should initialize for trigger snapshot status test");

    let mut snapshot = TriggerSnapshotRecord::new("tr-market");
    snapshot.last_event_id = Some("event-snapshot".to_string());
    snapshot.last_accepted_at_ms = Some(1_710_000_050_000);
    snapshot.last_sequence = 4;
    snapshot
        .accepted_event_ids
        .insert("event-snapshot".to_string());
    state_store
        .write_trigger_snapshot(&snapshot)
        .expect("trigger snapshot should persist for status output");

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

    let mut state_store =
        RuntimeStateStore::open(&storage_config_for_root(&basic_root()), 1_710_000_060_000)
            .expect("runtime state store should initialize for list-runs read-only test");

    state_store
        .write_run_summary(&RunRecordSummary {
            schema_version: "1.0.0".to_string(),
            run_id: "run-committed".to_string(),
            workflow_id: "wf-alpha".to_string(),
            status: RunStatus::Running,
            started_at_ms: 1_710_000_060_000,
            finished_at_ms: None,
        })
        .expect("committed run summary should persist");

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
    assert!(stderr.contains("Root is missing required root directory:"));
    assert!(!stderr.contains("panicked at"));
    assert!(!stderr.contains("thread 'main'"));
}

#[test]
fn run_and_serve_have_bounded_runtime_outcomes() {
    let _lock = acquire_fixture_lock();
    ensure_basic_root_fixture();

    let run_output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", basic_root())
        .arg("run")
        .output()
        .expect("run should execute");
    assert_eq!(run_output.status.code(), Some(5));
    let run_stderr = String::from_utf8(run_output.stderr).expect("stderr should be UTF-8");
    assert!(run_stderr.contains("Run manual-"));
    assert!(run_stderr.contains("failed"));

    // Clean up trigger records created by run before serve to ensure deterministic outcome
    let state_root = basic_root().join("state");
    let _ = fs::remove_dir_all(state_root.join("triggers"));
    let _ = fs::create_dir_all(state_root.join("triggers"));

    let serve_output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", basic_root())
        .arg("serve")
        .output()
        .expect("serve should execute");
    assert!(serve_output.status.success());
    let serve_stdout = String::from_utf8(serve_output.stdout).expect("stdout should be UTF-8");
    let serve_stderr = String::from_utf8(serve_output.stderr).expect("stderr should be UTF-8");
    assert!(serve_stdout.contains("serve completed: no accepted trigger events"));
    assert!(serve_stderr.is_empty());
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

fn storage_config_for_root(root: &Path) -> RuntimeStorageConfig {
    RuntimeStorageConfig {
        backend: RuntimeStorageBackend::Local {
            database_path: root.join("state").join("runtime.sqlite3"),
        },
        raw_debug_enabled: false,
        raw_debug_artifacts_dir: None,
    }
}

fn fixture_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}
