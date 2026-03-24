//! [INPUT]
//! CLI binary invocations, curated example roots, and plugin manifests with richer metadata or legacy fallback shapes.
//!
//! [OUTPUT]
//! Verifies catalog list/show command behavior, machine-readable payloads, and plugin metadata fallback coverage.
//!
//! [ROLE]
//! Covers the capability-discovery CLI surface independently from the broader CLI integration suite.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn catalog_list_reports_builtin_sections_without_a_root() {
    let _lock = acquire_fixture_lock();
    let root = unique_root("catalog-no-root");
    let _ = fs::remove_dir_all(&root);

    let output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", &root)
        .args(["catalog", "list"])
        .output()
        .expect("catalog list should execute without a root");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    assert!(stdout.contains("Builtin nodes"));
    assert!(stdout.contains("builtin_node:builtin.data.merge"));
    assert!(stdout.contains("Builtin triggers"));
    assert!(stdout.contains("builtin_trigger:webhook"));
    assert!(stdout.contains("Installed plugins"));
}

#[test]
fn catalog_list_json_can_filter_to_plugins() {
    let _lock = acquire_fixture_lock();

    let output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", plugin_integrations_root())
        .args(["catalog", "list", "--json", "--kind", "plugin"])
        .output()
        .expect("catalog list --json --kind plugin should execute");

    assert!(output.status.success());
    let payload = serde_json::from_slice::<serde_json::Value>(&output.stdout)
        .expect("catalog list json should decode");
    assert!(payload.get("builtin_nodes").is_none());
    assert!(payload.get("builtin_triggers").is_none());
    assert_eq!(payload["plugins"].as_array().map(Vec::len), Some(2));
}

#[test]
fn catalog_list_text_filter_only_renders_requested_section() {
    let _lock = acquire_fixture_lock();

    let output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", plugin_integrations_root())
        .args(["catalog", "list", "--kind", "plugin"])
        .output()
        .expect("catalog list --kind plugin should execute");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    assert!(!stdout.contains("Builtin nodes"));
    assert!(!stdout.contains("Builtin triggers"));
    assert!(stdout.contains("Installed plugins"));
    assert!(stdout.contains("plugin:quote-node-plugin"));
}

#[test]
fn catalog_show_reports_external_node_operations() {
    let _lock = acquire_fixture_lock();

    let output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", plugin_integrations_root())
        .args(["catalog", "show", "plugin:quote-node-plugin", "--json"])
        .output()
        .expect("catalog show plugin should execute");

    assert!(output.status.success());
    let payload = serde_json::from_slice::<serde_json::Value>(&output.stdout)
        .expect("catalog show json should decode");
    assert_eq!(payload["reference"], "plugin:quote-node-plugin");
    assert_eq!(payload["detail"]["plugin_kind"], "external_node");
    assert_eq!(payload["detail"]["schema_status"], "declared");
    assert_eq!(payload["detail"]["operations"][0]["name"], "normalize");
}

#[test]
fn catalog_builtin_list_succeeds_even_when_root_is_invalid() {
    let _lock = acquire_fixture_lock();
    let root = unique_root("catalog-invalid-root");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("root directory should be creatable");
    fs::write(
        root.join("chainbot.toml"),
        "manifest_version = \"2.0.0\"\nchainbot_version = \"2.3.2\"\nprofile = \"broken\"\n",
    )
    .expect("invalid root config should be writable");

    let output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", &root)
        .args(["catalog", "list", "--kind", "builtin_node"])
        .output()
        .expect("builtin catalog list should execute even with invalid root");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    assert!(stdout.contains("Builtin nodes"));
    assert!(stdout.contains("builtin_node:builtin.data.merge"));
}

#[test]
fn catalog_builtin_show_succeeds_even_when_root_is_invalid() {
    let _lock = acquire_fixture_lock();
    let root = unique_root("catalog-invalid-root-show");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("root directory should be creatable");
    fs::write(
        root.join("chainbot.toml"),
        "manifest_version = \"2.0.0\"\nchainbot_version = \"2.3.2\"\nprofile = \"broken\"\n",
    )
    .expect("invalid root config should be writable");

    let output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", &root)
        .args(["catalog", "show", "builtin_trigger:webhook", "--json"])
        .output()
        .expect("builtin catalog show should execute even with invalid root");

    assert!(output.status.success());
    let payload = serde_json::from_slice::<serde_json::Value>(&output.stdout)
        .expect("builtin show json should decode");
    assert_eq!(payload["reference"], "builtin_trigger:webhook");
    assert_eq!(payload["detail"]["kind"], "builtin_trigger");
}

#[test]
fn catalog_show_reports_external_trigger_event_schema() {
    let _lock = acquire_fixture_lock();

    let output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", plugin_integrations_root())
        .args(["catalog", "show", "plugin:market-trigger-plugin", "--json"])
        .output()
        .expect("catalog show trigger plugin should execute");

    assert!(output.status.success());
    let payload = serde_json::from_slice::<serde_json::Value>(&output.stdout)
        .expect("catalog trigger plugin json should decode");
    assert_eq!(payload["detail"]["plugin_kind"], "external_trigger");
    assert_eq!(payload["detail"]["schema_status"], "declared");
    assert_eq!(payload["detail"]["event_schema"]["fields"][0], "symbol");
}

#[test]
fn catalog_show_rejects_legacy_external_trigger_without_event_schema() {
    let _lock = acquire_fixture_lock();
    let root = unique_root("catalog-legacy-trigger-plugin");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("plugins").join("legacy-trigger"))
        .expect("plugin package directory should be creatable");
    fs::create_dir_all(root.join("workflows")).expect("workflows directory should be creatable");
    fs::create_dir_all(root.join("triggers")).expect("triggers directory should be creatable");
    fs::create_dir_all(root.join("secrets")).expect("secrets directory should be creatable");
    fs::create_dir_all(root.join("state")).expect("state directory should be creatable");
    fs::write(
        root.join("chainbot.toml"),
        format!(
            "manifest_version = \"2.0.0\"\nchainbot_version = \"{}\"\nprofile = \"catalog\"\n\n[storage]\nmode = \"local\"\n\n[storage.local]\ndatabase_path = \"state/runtime.sqlite3\"\n",
            env!("CARGO_PKG_VERSION")
        ),
    )
    .expect("root config should be writable");
    fs::write(
        root.join("plugins").join("legacy-trigger").join("config.toml"),
        "manifest_version = \"2.0.0\"\nplugin_id = \"legacy-trigger\"\nkind = \"external_trigger\"\nentrypoint = \"trigger.exec.v1\"\ncapabilities = [\"trigger.listen.event\"]\nexecutable = \"bin/external_trigger.sh\"\n",
    )
    .expect("legacy plugin config should be writable");

    let output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", &root)
        .args(["catalog", "show", "plugin:legacy-trigger", "--json"])
        .output()
        .expect("catalog show legacy plugin should execute");

    assert_eq!(output.status.code(), Some(3));
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(stderr.contains("plugin.event_schema"));
}

#[test]
fn catalog_show_rejects_unknown_reference_with_next_step_guidance() {
    let _lock = acquire_fixture_lock();
    let root = unique_root("catalog-unknown-reference");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("workflows")).expect("workflows directory should be creatable");
    fs::create_dir_all(root.join("triggers")).expect("triggers directory should be creatable");
    fs::create_dir_all(root.join("plugins")).expect("plugins directory should be creatable");
    fs::create_dir_all(root.join("secrets")).expect("secrets directory should be creatable");
    fs::create_dir_all(root.join("state")).expect("state directory should be creatable");
    fs::write(
        root.join("chainbot.toml"),
        format!(
            "manifest_version = \"2.0.0\"\nchainbot_version = \"{}\"\nprofile = \"catalog\"\n\n[storage]\nmode = \"local\"\n\n[storage.local]\ndatabase_path = \"state/runtime.sqlite3\"\n",
            env!("CARGO_PKG_VERSION")
        ),
    )
    .expect("root config should be writable");

    let output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", &root)
        .args(["catalog", "show", "plugin:missing-plugin"])
        .output()
        .expect("catalog show should execute");

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(stderr.contains("unknown catalog entry `plugin:missing-plugin`"));
    assert!(stderr.contains("chainbot catalog list"));
}

#[test]
fn catalog_list_rejects_unsupported_kind_filter() {
    let _lock = acquire_fixture_lock();

    let output = Command::new(chainbot_bin())
        .args(["catalog", "list", "--kind", "unknown"])
        .output()
        .expect("catalog list should execute");

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(stderr.contains("Unsupported --kind value"));
}

fn chainbot_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_chainbot"))
}

fn plugin_integrations_root() -> PathBuf {
    workspace_root()
        .join("examples")
        .join("plugin-integrations")
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

fn acquire_fixture_lock() -> MutexGuard<'static, ()> {
    fixture_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

fn fixture_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}
