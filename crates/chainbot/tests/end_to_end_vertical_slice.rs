//! [INPUT]
//! Full end-to-end fixture roots, CLI command invocations, and persisted runtime artifacts.
//!
//! [OUTPUT]
//! Verifies runnable validate, run, serve, and list-runs flows plus bounded failure-mode behavior.
//!
//! [ROLE]
//! Exercises the crate's vertical slice across config, trigger, execution, worker, secret, and state boundaries.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use chainbot::config::{RuntimeStorageBackend, RuntimeStorageConfig};
use chainbot::state::RunRecordSummary;
use chainbot::state::RunStatus;
use chainbot::state_db::RuntimeStateStore;

const SECRET_DECRYPT_ENV: &str = "CHAINBOT_SECRET_DECRYPTOR";
const SECRET_DECRYPT_MODE_PLAINTEXT: &str = "plaintext";

#[test]
fn end_to_end_vertical_slice() {
    let _guard = fixture_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());

    let root = prepare_fixture_root("success", "e2e");

    let validate_output = run_chainbot(["validate"], &root, true);
    assert!(validate_output.status.success());

    let run_output = run_chainbot(["run"], &root, true);
    assert!(run_output.status.success());

    let serve_output = run_chainbot(["serve"], &root, true);
    assert!(
        serve_output.status.success(),
        "serve failed: stdout={} stderr={}",
        String::from_utf8_lossy(&serve_output.stdout),
        String::from_utf8_lossy(&serve_output.stderr)
    );
    assert!(String::from_utf8_lossy(&serve_output.stdout).contains("serve started:"));
    wait_for_serve_state(&root, "active");
    wait_for_run_count(&root, 2);
    let stop_output = run_chainbot(["stop"], &root, true);
    assert!(stop_output.status.success());

    let list_runs_output = run_chainbot(["list-runs"], &root, false);
    assert!(list_runs_output.status.success());
    let runs: Vec<serde_json::Value> = serde_json::from_slice(&list_runs_output.stdout)
        .expect("list-runs output should decode as JSON array");

    assert!(runs.len() >= 2);
    let statuses = runs
        .iter()
        .filter_map(|run| run.get("status"))
        .filter_map(serde_json::Value::as_str)
        .collect::<Vec<_>>();
    assert!(statuses.iter().all(|status| *status == "succeeded"));

    assert!(count_trigger_records(&root) >= 1);

    let persisted_text = collect_runtime_text_from_db(&root);
    assert!(!persisted_text.contains("token-e2e-123"));
}

#[test]
fn end_to_end_vertical_slice_failure_modes() {
    let _guard = fixture_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());

    let root = prepare_fixture_root("failure_missing_secret", "e2e-failure");

    let validate_output = run_chainbot(["validate"], &root, true);
    assert!(validate_output.status.success());

    let run_output = run_chainbot(["run"], &root, true);
    assert!(!run_output.status.success());

    let stderr = String::from_utf8(run_output.stderr).expect("stderr should decode as UTF-8");
    assert!(stderr.contains("Run manual-"));
    assert!(stderr.contains("failed"));

    let list_runs_output = run_chainbot(["list-runs"], &root, false);
    assert!(list_runs_output.status.success());

    let runs: Vec<serde_json::Value> = serde_json::from_slice(&list_runs_output.stdout)
        .expect("list-runs output should decode as JSON array");
    assert_eq!(runs.len(), 1);

    let status = runs[0]
        .get("status")
        .and_then(serde_json::Value::as_str)
        .expect("run summary should contain status");
    assert_eq!(status, render_status(RunStatus::Failed));
}

#[test]
fn end_to_end_vertical_slice_failure_modes_redact_plugin_error_details() {
    let _guard = fixture_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());

    let root = prepare_fixture_root("success", "e2e-failure-secret-redaction");
    let secret_value = read_e2e_secret_value(&root);

    fs::write(
        root.join("plugins").join("bin").join("external_node.sh"),
        "#!/bin/sh\ncat >&2\nexit 42\n",
    )
    .expect("plugin fixture should be writable");
    make_executable(&root.join("plugins").join("bin").join("external_node.sh"));

    let run_output = run_chainbot(["run"], &root, true);
    assert!(!run_output.status.success());

    let stderr = String::from_utf8(run_output.stderr).expect("stderr should decode as UTF-8");
    assert!(stderr.contains("failed"));
    assert!(!stderr.contains(&secret_value));

    let persisted_logs = collect_runtime_text_from_db(&root);
    assert!(persisted_logs.contains("run_finished"));
    assert!(!persisted_logs.contains(&secret_value));
}

#[test]
fn serve_restart_recovery() {
    let _guard = fixture_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());

    let root = prepare_fixture_root("success", "e2e-serve-restart-recovery");
    let mut store = open_runtime_store(&root, 1_710_400_000_100);
    store
        .write_run_summary(&RunRecordSummary {
            schema_version: "1.0.0".to_string(),
            run_id: "run-incomplete".to_string(),
            workflow_id: "wf-e2e".to_string(),
            status: RunStatus::Running,
            started_at_ms: 1_710_400_000_000,
            finished_at_ms: None,
        })
        .expect("incomplete DB run summary should persist");
    drop(store);

    let serve_output = run_chainbot(["serve"], &root, true);
    assert!(
        serve_output.status.success(),
        "serve restart recovery failed: stdout={} stderr={}",
        String::from_utf8_lossy(&serve_output.stdout),
        String::from_utf8_lossy(&serve_output.stderr)
    );
    wait_for_serve_state(&root, "active");
    wait_for_run_count(&root, 2);
    let stop_output = run_chainbot(["stop"], &root, true);
    assert!(stop_output.status.success());

    let summaries = read_run_summaries(&root);
    let recovered_summary = summaries
        .iter()
        .find(|run| run.get("run_id") == Some(&serde_json::json!("run-incomplete")))
        .expect("recovered run summary should remain queryable");
    assert_eq!(recovered_summary["status"], serde_json::json!("failed"));

    assert!(summaries.iter().any(|run| {
        run.get("run_id") == Some(&serde_json::json!("run-incomplete"))
            && run.get("status") == Some(&serde_json::json!("failed"))
    }));
    assert!(summaries
        .iter()
        .any(|run| run.get("status") == Some(&serde_json::json!("succeeded"))));

    wait_for_serve_state(&root, "idle");
}

#[test]
fn duplicate_trigger_after_restart() {
    let _guard = fixture_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());

    let root = prepare_fixture_root("success", "e2e-duplicate-trigger-after-restart");

    let first_serve = run_chainbot(["serve"], &root, true);
    assert!(
        first_serve.status.success(),
        "first serve failed: stdout={} stderr={}",
        String::from_utf8_lossy(&first_serve.stdout),
        String::from_utf8_lossy(&first_serve.stderr)
    );
    wait_for_serve_state(&root, "active");
    wait_for_run_count(&root, 1);
    let first_stop = run_chainbot(["stop"], &root, true);
    assert!(first_stop.status.success());
    wait_for_serve_state(&root, "idle");

    let first_runs = read_run_summaries(&root);
    assert_eq!(first_runs.len(), 1);

    let second_serve = run_chainbot(["serve"], &root, true);
    assert!(
        second_serve.status.success(),
        "second serve failed: stdout={} stderr={}",
        String::from_utf8_lossy(&second_serve.stdout),
        String::from_utf8_lossy(&second_serve.stderr)
    );
    wait_for_serve_state(&root, "active");
    thread::sleep(Duration::from_millis(300));
    let second_stop = run_chainbot(["stop"], &root, true);
    assert!(second_stop.status.success());
    wait_for_serve_state(&root, "idle");

    let second_runs = read_run_summaries(&root);
    assert_eq!(second_runs.len(), first_runs.len());
}

fn render_status(status: RunStatus) -> &'static str {
    match status {
        RunStatus::Pending => "pending",
        RunStatus::Running => "running",
        RunStatus::Succeeded => "succeeded",
        RunStatus::Failed => "failed",
    }
}

fn run_chainbot<const N: usize>(
    args: [&str; N],
    root: &Path,
    with_plaintext_secrets: bool,
) -> std::process::Output {
    let mut command = std::process::Command::new(chainbot_bin());
    command.env("CHAINBOT_CONFIG_DIR", root).args(args);
    if with_plaintext_secrets {
        command.env(SECRET_DECRYPT_ENV, SECRET_DECRYPT_MODE_PLAINTEXT);
    }
    command.output().expect("chainbot command should execute")
}

fn prepare_fixture_root(case_name: &str, root_name: &str) -> PathBuf {
    let root = workspace_root()
        .join("target")
        .join("test-roots")
        .join(root_name);
    if root.exists() {
        fs::remove_dir_all(&root).expect("existing e2e root should be removable");
    }

    let source = fixture_root().join("e2e").join(case_name);
    copy_directory_recursive(&source, &root);
    let root_config_path = root.join("chainbot.toml");
    let root_config =
        fs::read_to_string(&root_config_path).expect("copied e2e root config should be readable");
    let updated_root_config = root_config
        .lines()
        .map(|line| {
            if line.trim_start().starts_with("chainbot_version = ") {
                format!("chainbot_version = \"{}\"", env!("CARGO_PKG_VERSION"))
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(&root_config_path, format!("{updated_root_config}\n"))
        .expect("e2e root config should be rewritten with the running version");

    make_executable(&root.join("plugins").join("bin").join("external_trigger.sh"));
    make_executable(&root.join("plugins").join("bin").join("external_node.sh"));

    root
}

fn copy_directory_recursive(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).expect("fixture destination directory should be creatable");

    let entries = fs::read_dir(source).expect("fixture source directory should be readable");
    for entry in entries {
        let entry = entry.expect("fixture source entry should decode");
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());

        if source_path.is_dir() {
            copy_directory_recursive(&source_path, &destination_path);
        } else {
            if let Some(parent) = destination_path.parent() {
                fs::create_dir_all(parent).expect("fixture destination parent should be creatable");
            }
            fs::copy(&source_path, &destination_path)
                .expect("fixture file should be copied into destination");
        }
    }
}

fn collect_runtime_text_from_db(root: &Path) -> String {
    let mut store = open_runtime_store(root, 1_710_500_000_000);
    let runs = store
        .list_run_summaries()
        .expect("runtime store should list run summaries for text collection");
    let logs = store
        .list_recent_workflow_log_entries(100, None)
        .expect("runtime store should list workflow logs for text collection");
    let trigger_events = store
        .list_recent_trigger_records(100, None)
        .expect("runtime store should list trigger events for text collection");
    format!(
        "{}\n{}\n{}",
        serde_json::to_string(&runs).expect("runs should serialize"),
        serde_json::to_string(&logs).expect("logs should serialize"),
        serde_json::to_string(&trigger_events).expect("trigger events should serialize"),
    )
}

fn read_run_summaries(root: &Path) -> Vec<serde_json::Value> {
    let list_runs_output = run_chainbot(["list-runs"], root, false);
    assert!(
        list_runs_output.status.success(),
        "list-runs failed: stdout={} stderr={}",
        String::from_utf8_lossy(&list_runs_output.stdout),
        String::from_utf8_lossy(&list_runs_output.stderr)
    );

    serde_json::from_slice(&list_runs_output.stdout)
        .expect("list-runs output should decode as JSON array")
}

fn wait_for_run_count(root: &Path, expected_min_runs: usize) {
    for _ in 0..50 {
        let runs = read_run_summaries(root);
        if runs.len() >= expected_min_runs {
            return;
        }
        thread::sleep(Duration::from_millis(100));
    }

    panic!("timed out waiting for at least {expected_min_runs} persisted runs");
}

fn wait_for_serve_state(root: &Path, expected_state: &str) {
    for _ in 0..50 {
        let status_output = run_chainbot(["status", "--json"], root, false);
        if status_output.status.success() {
            let payload: serde_json::Value = serde_json::from_slice(&status_output.stdout)
                .expect("status json output should decode during wait");
            if payload["serve"]["state"] == expected_state {
                return;
            }
        }
        thread::sleep(Duration::from_millis(100));
    }

    panic!("timed out waiting for serve.state={expected_state}");
}

fn count_json_files(path: &Path) -> usize {
    if !path.exists() {
        return 0;
    }

    let mut count = 0;
    let entries = fs::read_dir(path).expect("directory entries should be readable");
    for entry in entries {
        let entry = entry.expect("directory entry should decode");
        let entry_path = entry.path();
        if entry_path.is_dir() {
            count += count_json_files(&entry_path);
        } else if entry_path.extension().and_then(|value| value.to_str()) == Some("json") {
            count += 1;
        }
    }

    count
}

fn count_trigger_records(root: &Path) -> usize {
    let mut store = open_runtime_store(root, 1_710_500_000_000);
    store
        .list_recent_trigger_records(100, None)
        .expect("runtime store should list trigger records")
        .len()
}

fn read_e2e_secret_value(root: &Path) -> String {
    let payload = fs::read_to_string(
        root.join("secrets")
            .join("ops")
            .join("slack")
            .join("webhook.gpg"),
    )
    .expect("fixture secret payload should be readable");
    payload
        .lines()
        .find_map(|line| line.trim().strip_prefix("api_token="))
        .expect("fixture secret payload should contain api_token entry")
        .to_owned()
}

fn write_json_file<T>(path: &Path, value: &T)
where
    T: serde::Serialize,
{
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("staged file parent directory should be creatable");
    }
    fs::write(
        path,
        serde_json::to_vec_pretty(value).expect("staged JSON payload should serialize"),
    )
    .expect("staged JSON payload should be writable");
}

fn open_runtime_store(root: &Path, now_ms: i64) -> RuntimeStateStore {
    RuntimeStateStore::open(
        &RuntimeStorageConfig {
            backend: RuntimeStorageBackend::Local {
                database_path: root.join("state").join("runtime.sqlite3"),
            },
            history_retention: None,
            raw_debug_enabled: false,
            raw_debug_artifacts_dir: None,
        },
        now_ms,
    )
    .expect("runtime store should open for e2e assertions")
}

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

fn chainbot_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_chainbot"))
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

fn make_executable(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path)
            .expect("fixture script metadata should exist")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).expect("fixture script should be executable");
    }
}

fn fixture_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}
