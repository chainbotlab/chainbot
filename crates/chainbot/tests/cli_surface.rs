/*
[INPUT]:  CLI binary invocations plus fixture roots under target/test-roots/basic.
[OUTPUT]: Integration coverage for help output, success cases, and stable user-facing CLI failures.
[POS]:    Integration test boundary for task-5 CLI surface and exit behavior.
[UPDATE]: 2026-03-16 - Add end-to-end CLI command surface coverage.
[UPDATE]: 2026-03-16 - Serialize shared basic-root fixture setup within the CLI surface test binary.
[UPDATE]: 2026-03-18 - Cover trigger help and persisted trigger enable/disable CLI mutations.
*/

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use chainbot::config::RootLayout;
use chainbot::state::{
    CoordinationStore, FileBackedStateStore, RunRecordSummary, RunStatus, StateLayout,
    SERVE_OWNER_ID_PREFIX,
};

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
    assert!(stdout.contains("status"));
    assert!(stdout.contains("trigger"));
    assert!(stdout.contains("serve"));
    assert!(stdout.contains("run"));
    assert!(stdout.contains("list-runs"));
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

    assert!(stdout.contains("enable or disable a trigger package"));
    assert!(stdout.contains("chainbot trigger enable tr-market"));
    assert!(stderr.is_empty());
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
    assert!(stderr.is_empty());
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
    let state_layout = StateLayout::from_root_layout(&RootLayout::from_root(basic_root()));
    let mut coordination = CoordinationStore::open(&state_layout, now_ms)
        .expect("coordination store should open for status fixture");
    let owner_id = format!("{SERVE_OWNER_ID_PREFIX}{}", std::process::id());
    coordination
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

    let state_layout = StateLayout::from_root_layout(&RootLayout::from_root(basic_root()));
    let state_store = FileBackedStateStore::new(state_layout.clone());
    state_store
        .initialize()
        .expect("state tree should initialize for status mutation test");
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

    let summary = state_store
        .read_run_summary("run-incomplete")
        .expect("run summary should remain readable after status");
    assert_eq!(summary.status, RunStatus::Running);
    assert!(!state_layout
        .workflow_logs_dir
        .join("run-incomplete")
        .exists());
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
    let _ = fs::remove_dir_all(state_root.join("trigger-records"));
    let _ = fs::create_dir_all(state_root.join("trigger-records"));

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
    let _ = fs::remove_dir_all(state_root.join("workflow-logs"));
    let _ = fs::remove_dir_all(state_root.join("trigger-records"));
    let _ = fs::remove_file(state_root.join("coordination.sqlite3"));

    fs::create_dir_all(root.join("config")).expect("config directory should be creatable");
    fs::create_dir_all(root.join("workflows")).expect("workflows directory should be creatable");
    fs::create_dir_all(root.join("triggers")).expect("triggers directory should be creatable");
    fs::create_dir_all(root.join("plugins").join("manifests"))
        .expect("plugin manifests directory should be creatable");
    fs::create_dir_all(root.join("secrets")).expect("secrets directory should be creatable");
    fs::create_dir_all(&state_root).expect("state directory should be creatable");
    fs::create_dir_all(root.join("workflows").join("wf-alpha"))
        .expect("workflow package directory should be creatable");
    fs::create_dir_all(root.join("triggers").join("tr-market"))
        .expect("trigger package directory should be creatable");

    fs::write(
        root.join("config").join("root.toml"),
        "manifest_version = \"2.0.0\"\nprofile = \"basic\"\nsecret_refs = [\"secret://ops/slack/webhook\"]\n",
    )
    .expect("root config fixture should be writable");

    fs::write(
        root.join("workflows").join("wf-alpha").join("config.toml"),
        "[workflow]\nmanifest_version = \"2.0.0\"\nid = \"wf-alpha\"\nname = \"alpha\"\n\n[[nodes]]\nmanifest_version = \"2.0.0\"\nid = \"node-1\"\nkind = \"plugin\"\nplugin = \"quote-plugin\"\noperation = \"normalize\"\ndepends_on = []\n",
    )
    .expect("workflow fixture should be writable");

    fs::write(
        root.join("triggers").join("tr-market").join("config.toml"),
        "manifest_version = \"2.0.0\"\ntrigger_id = \"tr-market\"\nkind = \"market_tick\"\nsource = \"market-feed\"\nworkflow_id = \"wf-alpha\"\nenabled = false\n",
    )
    .expect("trigger fixture should be writable");

    fs::write(
        root.join("plugins").join("manifests").join("quote_plugin.toml"),
        "manifest_version = \"2.0.0\"\nplugin_id = \"quote-plugin\"\nkind = \"builtin\"\nentrypoint = \"plugins.quote\"\ncapabilities = [\"normalize\"]\n",
    )
    .expect("plugin fixture should be writable");
}

fn fixture_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}
