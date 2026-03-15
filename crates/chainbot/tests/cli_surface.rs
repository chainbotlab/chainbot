/*
[INPUT]:  CLI binary invocations plus fixture roots under target/test-roots/basic.
[OUTPUT]: Integration coverage for help output, success cases, and stable user-facing CLI failures.
[POS]:    Integration test boundary for task-5 CLI surface and exit behavior.
[UPDATE]: 2026-03-16 - Add end-to-end CLI command surface coverage.
[UPDATE]: 2026-03-16 - Serialize shared basic-root fixture setup within the CLI surface test binary.
*/

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, MutexGuard, OnceLock};

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
    assert!(stdout.contains("serve"));
    assert!(stdout.contains("run"));
    assert!(stdout.contains("list-runs"));
    assert!(stderr.is_empty());
}

#[test]
fn validate_accepts_basic_root() {
    let _lock = acquire_fixture_lock();
    ensure_basic_root_fixture();

    let output = Command::new(chainbot_bin())
        .args(["validate", "--root"])
        .arg(basic_root())
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
        .args(["list-runs", "--root"])
        .arg(basic_root())
        .output()
        .expect("list-runs should execute");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");

    assert_eq!(stdout.trim(), "[]");
    assert!(stderr.is_empty());
}

#[test]
fn invalid_root_returns_stable_user_facing_error() {
    let missing_root = workspace_root()
        .join("target")
        .join("test-roots")
        .join("missing-cli-root");

    let output = Command::new(chainbot_bin())
        .args(["validate", "--root"])
        .arg(missing_root)
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
        .args(["run", "--root"])
        .arg(basic_root())
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
        .args(["serve", "--root"])
        .arg(basic_root())
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
    fs::create_dir_all(root.join("plugins")).expect("plugins directory should be creatable");
    fs::create_dir_all(root.join("secrets")).expect("secrets directory should be creatable");
    fs::create_dir_all(&state_root).expect("state directory should be creatable");

    fs::write(
        root.join("config").join("root.toml"),
        "schema_version = \"1.0.0\"\nprofile = \"basic\"\nsecret_refs = [\"secret://ops/slack/webhook\"]\n",
    )
    .expect("root config fixture should be writable");

    fs::write(
        root.join("workflows").join("wf_alpha.toml"),
        "api_version = \"1.0.0\"\nworkflow_id = \"wf-alpha\"\nname = \"alpha\"\n\n[[triggers]]\napi_version = \"1.0.0\"\ntrigger_id = \"inline-tr\"\nkind = \"manual\"\nsource = \"inline\"\nenabled = true\n\n[[nodes]]\napi_version = \"1.0.0\"\nnode_id = \"node-1\"\nkind = \"plugin\"\nplugin_id = \"quote-plugin\"\noperation = \"normalize\"\ndepends_on = []\n",
    )
    .expect("workflow fixture should be writable");

    fs::write(
        root.join("triggers").join("trigger_market.toml"),
        "api_version = \"1.0.0\"\ntrigger_id = \"tr-market\"\nkind = \"market_tick\"\nsource = \"market-feed\"\nenabled = false\n",
    )
    .expect("trigger fixture should be writable");

    fs::write(
        root.join("plugins").join("quote_plugin.toml"),
        "api_version = \"1.0.0\"\nplugin_id = \"quote-plugin\"\nkind = \"builtin\"\nentrypoint = \"plugins.quote\"\ncapabilities = [\"normalize\"]\n",
    )
    .expect("plugin fixture should be writable");
}

fn fixture_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}
