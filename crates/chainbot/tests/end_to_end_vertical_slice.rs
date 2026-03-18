/*
[INPUT]:  Full e2e fixture roots, CLI command invocations, and persisted state artifacts.
[OUTPUT]: Deterministic success and failure-mode evidence for the runnable MVP vertical slice.
[POS]:    Integration test boundary for task-11 end-to-end run/serve/list-runs behavior.
[UPDATE]: 2026-03-16 - Add end_to_end_vertical_slice and end_to_end_vertical_slice_failure_modes tests.
[UPDATE]: 2026-03-16 - Keep staged trigger-record fixtures aligned with persisted coordination metadata.
[UPDATE]: 2026-03-17 - Add runtime failure regression proving plugin stderr and run_failed logs redact resolved secrets.
*/

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use chainbot::config::RootLayout;
use chainbot::state::RunStatus;
use chainbot::state::{
    CoordinationStore, FileBackedStateStore, LeaseAcquireResult, RunRecordSummary, StateLayout,
    TriggerEventRecord, WorkflowRuntimeLogEntry, SERVE_OWNER_ID_PREFIX,
};

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

    assert!(directory_contains_files(
        &root.join("state").join("workflow-logs")
    ));
    assert!(directory_contains_files(
        &root.join("state").join("trigger-records")
    ));

    let persisted_text = collect_text_files(&root.join("state"));
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

    let persisted_logs = collect_text_files(&root.join("state").join("workflow-logs"));
    assert!(persisted_logs.contains("run_finished"));
    assert!(!persisted_logs.contains(&secret_value));
}

#[test]
fn serve_restart_recovery() {
    let _guard = fixture_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());

    let root = prepare_fixture_root("success", "e2e-serve-restart-recovery");
    let root_layout = RootLayout::from_root(root.clone());
    let state_layout = StateLayout::from_root_layout(&root_layout);
    let store = FileBackedStateStore::new(state_layout.clone());
    store
        .initialize()
        .expect("state store should initialize for recovery test");

    store
        .write_run_summary(&RunRecordSummary {
            schema_version: "1.0.0".to_string(),
            run_id: "run-incomplete".to_string(),
            workflow_id: "wf-e2e".to_string(),
            status: RunStatus::Running,
            started_at_ms: 1_710_400_000_000,
            finished_at_ms: None,
        })
        .expect("incomplete run summary should persist");
    store
        .write_workflow_log_entry(&WorkflowRuntimeLogEntry {
            run_id: "run-incomplete".to_string(),
            sequence: 1,
            event: "run_started".to_string(),
            message: "execution started".to_string(),
            occurred_at_ms: 1_710_400_000_010,
        })
        .expect("initial workflow log should persist");

    write_json_file(
        &state_layout.staged_workflow_log_entry_path("run-incomplete", 2),
        &WorkflowRuntimeLogEntry {
            run_id: "run-incomplete".to_string(),
            sequence: 2,
            event: "node_running".to_string(),
            message: "node was still running before restart".to_string(),
            occurred_at_ms: 1_710_400_000_020,
        },
    );
    write_json_file(
        &state_layout.staged_trigger_record_path(
            "run-incomplete",
            99,
            "recovery-trigger",
            "recovery-event",
        ),
        &TriggerEventRecord {
            run_id: "run-incomplete".to_string(),
            sequence: 99,
            trigger_id: "recovery-trigger".to_string(),
            event_id: "recovery-event".to_string(),
            source: "restart-recovery".to_string(),
            accepted_at_ms: 1_710_400_000_030,
            dedup_key: None,
            dedup_expires_at_ms: None,
            cooldown_key: None,
            cooldown_expires_at_ms: None,
        },
    );

    let mut coordination = CoordinationStore::open(&state_layout, 1_710_400_000_100)
        .expect("coordination store should open");
    let stale_owner = format!("{SERVE_OWNER_ID_PREFIX}999999");
    assert!(matches!(
        coordination
            .try_acquire_serve_lease(&stale_owner, 1_710_400_000_100, 60_000)
            .expect("stale owner lease should persist"),
        LeaseAcquireResult::Acquired
    ));
    drop(coordination);

    let serve_output = run_chainbot(["serve"], &root, true);
    assert!(
        serve_output.status.success(),
        "serve restart recovery failed: stdout={} stderr={}",
        String::from_utf8_lossy(&serve_output.stdout),
        String::from_utf8_lossy(&serve_output.stderr)
    );

    let recovered_summary = store
        .read_run_summary("run-incomplete")
        .expect("incomplete run should be rewritten after restart");
    assert_eq!(recovered_summary.status, RunStatus::Failed);
    assert!(recovered_summary.finished_at_ms.is_some());

    let promoted_log_path = state_layout.workflow_log_entry_path("run-incomplete", 2);
    let recovery_log_path = state_layout.workflow_log_entry_path("run-incomplete", 3);
    let promoted_trigger_path = state_layout.trigger_record_path(
        "run-incomplete",
        99,
        "recovery-trigger",
        "recovery-event",
    );
    assert!(promoted_log_path.exists());
    assert!(recovery_log_path.exists());
    assert!(promoted_trigger_path.exists());

    let recovery_log: WorkflowRuntimeLogEntry = serde_json::from_str(
        &fs::read_to_string(&recovery_log_path).expect("recovery log file should be readable"),
    )
    .expect("recovery log should decode");
    assert_eq!(recovery_log.event, "run_recovered_after_restart");

    let runs = read_run_summaries(&root);
    assert!(runs.iter().any(|run| {
        run.get("run_id") == Some(&serde_json::json!("run-incomplete"))
            && run.get("status") == Some(&serde_json::json!("failed"))
    }));
    assert!(runs
        .iter()
        .any(|run| run.get("status") == Some(&serde_json::json!("succeeded"))));

    let mut post_serve_coordination = CoordinationStore::open(&state_layout, 1_710_400_120_000)
        .expect("coordination store should reopen after serve");
    assert!(matches!(
        post_serve_coordination
            .try_acquire_serve_lease("owner-after-restart", 1_710_400_120_000, 5_000)
            .expect("lease should be free after serve completes"),
        LeaseAcquireResult::Acquired
    ));
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

    let first_trigger_count = count_json_files(&root.join("state").join("trigger-records"));
    let first_runs = read_run_summaries(&root);
    assert!(first_trigger_count >= 1);
    assert_eq!(first_runs.len(), 1);

    let second_serve = run_chainbot(["serve"], &root, true);
    assert!(
        second_serve.status.success(),
        "second serve failed: stdout={} stderr={}",
        String::from_utf8_lossy(&second_serve.stdout),
        String::from_utf8_lossy(&second_serve.stderr)
    );
    assert!(String::from_utf8_lossy(&second_serve.stdout)
        .contains("serve completed: no accepted trigger events"));

    let second_trigger_count = count_json_files(&root.join("state").join("trigger-records"));
    let second_runs = read_run_summaries(&root);
    assert_eq!(second_trigger_count, first_trigger_count);
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

fn directory_contains_files(path: &Path) -> bool {
    if !path.exists() {
        return false;
    }

    let entries = fs::read_dir(path).expect("directory entries should be readable");
    for entry in entries {
        let entry = entry.expect("directory entry should decode");
        let path = entry.path();
        if path.is_file() {
            return true;
        }
        if path.is_dir() && directory_contains_files(&path) {
            return true;
        }
    }

    false
}

fn collect_text_files(path: &Path) -> String {
    if !path.exists() {
        return String::new();
    }

    let mut combined = String::new();
    let entries = fs::read_dir(path).expect("state directory should be readable");
    for entry in entries {
        let entry = entry.expect("state directory entry should decode");
        let entry_path = entry.path();
        if entry_path.is_dir() {
            combined.push_str(&collect_text_files(&entry_path));
            continue;
        }

        if let Ok(contents) = fs::read_to_string(&entry_path) {
            combined.push_str(&contents);
        }
    }

    combined
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
