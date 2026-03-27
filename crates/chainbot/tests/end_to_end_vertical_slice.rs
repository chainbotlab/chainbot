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

use rusqlite::{params, Connection};

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
    assert_eq!(status, "failed");
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
    upsert_run_summary(
        &root,
        "run-incomplete",
        "wf-e2e",
        "running",
        1_710_400_000_000,
        None,
    );

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

#[test]
fn disable_trigger_prevents_future_acceptance_after_restart() {
    let _guard = fixture_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());

    let root = prepare_fixture_root("success", "e2e-disable-prevents-replay");

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
    let first_trigger_records = count_trigger_records_for_trigger_id(&root, "external-trigger-e2e");
    assert!(first_trigger_records >= 1);

    let disable_output = run_chainbot(["trigger", "disable", "external-trigger-e2e"], &root, false);
    assert!(
        disable_output.status.success(),
        "trigger disable failed: stdout={} stderr={}",
        String::from_utf8_lossy(&disable_output.stdout),
        String::from_utf8_lossy(&disable_output.stderr)
    );

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
    let second_trigger_records =
        count_trigger_records_for_trigger_id(&root, "external-trigger-e2e");
    assert_eq!(second_runs.len(), first_runs.len());
    assert_eq!(second_trigger_records, first_trigger_records);
}

#[test]
fn lease_loss_teardown_rejects_future_acceptance_after_teardown() {
    let _guard = fixture_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());

    let root = prepare_fixture_root("success", "e2e-lease-loss-replay-safe");

    let first_serve = run_chainbot(["serve"], &root, true);
    assert!(
        first_serve.status.success(),
        "first serve failed: stdout={} stderr={}",
        String::from_utf8_lossy(&first_serve.stdout),
        String::from_utf8_lossy(&first_serve.stderr)
    );
    wait_for_serve_state(&root, "active");
    wait_for_run_count(&root, 1);

    let baseline_runs = read_run_summaries(&root);
    let baseline_trigger_records =
        count_trigger_records_for_trigger_id(&root, "external-trigger-e2e");
    assert!(baseline_trigger_records >= 1);

    force_replace_serve_lease_owner(&root, "lease-stolen-owner", 9_999_999_999_999_i64);
    wait_for_serve_state_any(&root, &["idle", "stale"]);

    let cleanup_stop = run_chainbot(["stop"], &root, true);
    assert!(cleanup_stop.status.success());
    wait_for_serve_state(&root, "idle");

    thread::sleep(Duration::from_millis(300));
    let after_teardown_runs = read_run_summaries(&root);
    let after_teardown_trigger_records =
        count_trigger_records_for_trigger_id(&root, "external-trigger-e2e");
    assert_eq!(after_teardown_runs.len(), baseline_runs.len());
    assert_eq!(after_teardown_trigger_records, baseline_trigger_records);
}

#[test]
fn manifest_plugin_switch_restarts_session_and_accepts_new_event_once() {
    let _guard = fixture_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());

    let root = prepare_fixture_root("success", "e2e-manifest-switch-restart");

    let first_serve = run_chainbot(["serve"], &root, true);
    assert!(
        first_serve.status.success(),
        "first serve failed: stdout={} stderr={}",
        String::from_utf8_lossy(&first_serve.stdout),
        String::from_utf8_lossy(&first_serve.stderr)
    );
    wait_for_serve_state(&root, "active");
    wait_for_run_count(&root, 1);

    install_v2_trigger_plugin_and_switch_trigger_manifest(&root);
    wait_for_run_count(&root, 2);

    let stop_output = run_chainbot(["stop"], &root, true);
    assert!(stop_output.status.success());
    wait_for_serve_state(&root, "idle");

    let trigger_records = list_recent_trigger_records(&root, 200, Some("external-trigger-e2e"));

    let v1_count = trigger_records
        .iter()
        .filter(|record| record.event_id == "external-trigger-e2e:event-external")
        .count();
    let v2_count = trigger_records
        .iter()
        .filter(|record| record.event_id == "external-trigger-e2e:event-external-v2")
        .count();

    assert_eq!(v1_count, 1);
    assert_eq!(v2_count, 1);
}

#[test]
fn multi_trigger_process_sessions_accept_once_each_and_remain_stable_after_restart() {
    let _guard = fixture_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());

    let root = prepare_fixture_root("success", "e2e-process-matrix-multi-trigger");
    install_secondary_external_trigger_package(&root);

    let first_serve = run_chainbot(["serve"], &root, true);
    assert!(
        first_serve.status.success(),
        "first serve failed: stdout={} stderr={}",
        String::from_utf8_lossy(&first_serve.stdout),
        String::from_utf8_lossy(&first_serve.stderr)
    );
    wait_for_serve_state(&root, "active");
    wait_for_run_count(&root, 2);
    let first_stop = run_chainbot(["stop"], &root, true);
    assert!(first_stop.status.success());
    wait_for_serve_state(&root, "idle");

    let first_runs = read_run_summaries(&root);
    assert_eq!(first_runs.len(), 2);
    let first_primary_count = count_trigger_records_for_trigger_id(&root, "external-trigger-e2e");
    let first_secondary_count =
        count_trigger_records_for_trigger_id(&root, "external-trigger-e2e-secondary");
    assert_eq!(first_primary_count, 1);
    assert_eq!(first_secondary_count, 1);

    let second_serve = run_chainbot(["serve"], &root, true);
    assert!(
        second_serve.status.success(),
        "second serve failed: stdout={} stderr={}",
        String::from_utf8_lossy(&second_serve.stdout),
        String::from_utf8_lossy(&second_serve.stderr)
    );
    wait_for_serve_state(&root, "active");
    let second_stop = run_chainbot(["stop"], &root, true);
    assert!(second_stop.status.success());
    wait_for_serve_state(&root, "idle");

    let second_runs = read_run_summaries(&root);
    assert_eq!(second_runs.len(), first_runs.len());
    assert_eq!(
        count_trigger_records_for_trigger_id(&root, "external-trigger-e2e"),
        first_primary_count
    );
    assert_eq!(
        count_trigger_records_for_trigger_id(&root, "external-trigger-e2e-secondary"),
        first_secondary_count
    );
}

#[test]
fn serve_turn_bridges_pending_staged_external_rows() {
    let _guard = fixture_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());

    let root = prepare_fixture_root("success", "e2e-serve-staged-bridge");
    insert_staged_trigger_event_record(
        &root,
        "staging-e2e-bridge-1",
        "external-trigger-e2e",
        "wf-e2e",
        "external-trigger-e2e:event-staged",
        "external-trigger-plugin",
        1_710_600_000_000,
        1_710_600_000_001,
        Some("cp-staged-e2e"),
        serde_json::json!({"symbol": "ETHUSDT"}),
    );

    let serve_output = run_chainbot(["serve"], &root, true);
    assert!(
        serve_output.status.success(),
        "serve failed: stdout={} stderr={}",
        String::from_utf8_lossy(&serve_output.stdout),
        String::from_utf8_lossy(&serve_output.stderr)
    );
    wait_for_serve_state(&root, "active");
    wait_for_run_count(&root, 2);
    let stop_output = run_chainbot(["stop"], &root, true);
    assert!(stop_output.status.success());
    wait_for_serve_state(&root, "idle");

    let pending_count =
        count_pending_staged_trigger_event_records(&root, "external-trigger-e2e", 10);
    assert_eq!(pending_count, 0);

    let trigger_events = list_recent_trigger_records(&root, 20, Some("external-trigger-e2e"));
    assert!(
        trigger_events
            .iter()
            .any(|record| record.event_id == "external-trigger-e2e:event-staged"),
        "staged external event should be normalized into accepted trigger records"
    );
}

#[test]
fn serve_turn_accepts_wasm_persistent_external_trigger_via_daemon_callback_path() {
    let _guard = fixture_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner());

    let root = prepare_fixture_root("success", "e2e-serve-wasm-persistent-callback");
    install_wasm_trigger_runtime_for_primary_external_plugin(&root);

    let serve_output = run_chainbot(["serve"], &root, true);
    assert!(
        serve_output.status.success(),
        "serve failed: stdout={} stderr={}",
        String::from_utf8_lossy(&serve_output.stdout),
        String::from_utf8_lossy(&serve_output.stderr)
    );
    wait_for_serve_state(&root, "active");
    wait_for_run_count(&root, 1);
    let stop_output = run_chainbot(["stop"], &root, true);
    assert!(stop_output.status.success());
    wait_for_serve_state(&root, "idle");

    let trigger_events = list_recent_trigger_records(&root, 20, Some("external-trigger-e2e"));
    assert!(
        trigger_events.iter().any(|record| {
            record.event_id == "external-trigger-e2e:guest-callback-event"
                && record.payload["symbol"] == serde_json::json!("SOLUSDT")
        }),
        "wasm callback event should be normalized into accepted trigger records"
    );
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
    let connection = open_runtime_db(root);
    let runs = query_json_objects(
        &connection,
        "SELECT run_id, workflow_id, status, started_at_ms, finished_at_ms
         FROM run_summaries
         ORDER BY started_at_ms DESC, run_id DESC
         LIMIT 100",
    );
    let logs = query_json_objects(
        &connection,
        "SELECT run_id, sequence, event, message, occurred_at_ms
         FROM workflow_runtime_logs
         ORDER BY occurred_at_ms DESC, run_id DESC, sequence DESC
         LIMIT 100",
    );
    let trigger_events = query_json_objects(
        &connection,
        "SELECT trigger_id, sequence, workflow_id, event_id, accepted_at_ms, payload_json
         FROM trigger_event_records
         ORDER BY accepted_at_ms DESC, trigger_id DESC, sequence DESC
         LIMIT 100",
    );
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

fn wait_for_serve_state_any(root: &Path, expected_states: &[&str]) {
    for _ in 0..250 {
        let status_output = run_chainbot(["status", "--json"], root, false);
        if status_output.status.success() {
            let payload: serde_json::Value = serde_json::from_slice(&status_output.stdout)
                .expect("status json output should decode during wait");
            let serve_state = payload["serve"]["state"].as_str().unwrap_or_default();
            if expected_states.contains(&serve_state) {
                return;
            }
        }
        thread::sleep(Duration::from_millis(100));
    }

    panic!(
        "timed out waiting for serve.state in [{}]",
        expected_states.join(", ")
    );
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
    count_trigger_records_for_trigger_id(root, "")
}

fn count_trigger_records_for_trigger_id(root: &Path, trigger_id: &str) -> usize {
    let connection = open_runtime_db(root);
    if trigger_id.is_empty() {
        return connection
            .query_row("SELECT COUNT(*) FROM trigger_event_records", [], |row| {
                row.get::<_, i64>(0)
            })
            .expect("trigger record count should query") as usize;
    }

    connection
        .query_row(
            "SELECT COUNT(*) FROM trigger_event_records WHERE trigger_id = ?1",
            params![trigger_id],
            |row| row.get::<_, i64>(0),
        )
        .expect("trigger record count by trigger_id should query") as usize
}

fn force_replace_serve_lease_owner(root: &Path, owner_id: &str, expires_at_ms: i64) {
    let sqlite_path = root.join("state").join("runtime.sqlite3");
    let connection = Connection::open(sqlite_path)
        .expect("runtime sqlite database should be readable for lease loss simulation");
    let updated = connection
        .execute(
            "UPDATE serve_leases SET owner_id = ?1, expires_at_ms = ?2 WHERE lease_key = ?3",
            rusqlite::params![owner_id, expires_at_ms, "serve"],
        )
        .expect("serve lease owner replacement should execute");
    assert_eq!(updated, 1, "serve lease row should exist for replacement");
}

fn install_v2_trigger_plugin_and_switch_trigger_manifest(root: &Path) {
    let plugin_dir = root.join("plugins").join("trigger-e2e-plugin-v2");
    fs::create_dir_all(&plugin_dir)
        .expect("v2 trigger plugin package directory should be creatable");
    fs::write(
        plugin_dir.join("config.toml"),
        "manifest_version = \"2.0.0\"\nplugin_id = \"trigger-e2e-plugin-v2\"\nkind = \"external_trigger\"\nentrypoint = \"trigger.exec.v1\"\ncapabilities = [\"trigger.listen.event\"]\nexecutable = \"../bin/external_trigger_v2.sh\"\n\n[trigger_runtime]\nlifecycle = \"process_short_lived\"\npush_callback = \"inline_response\"\ndurable_ack = \"caller_scope\"\nhost_error_categories = [\"transport\", \"protocol_contract\", \"plugin_fatal\"]\n\n[event_schema]\nsummary = \"External trigger payload\"\nfields = [\"symbol\", \"price\"]\n",
    )
    .expect("v2 trigger plugin config should be writable");

    let v2_script = root
        .join("plugins")
        .join("bin")
        .join("external_trigger_v2.sh");
    fs::write(
        &v2_script,
        "#!/bin/sh\nIFS= read -r start_line\nsymbol=\"BTCUSDT\"\ncase \"$start_line\" in\n  *'\"symbol\":\"ETHUSDT\"'*) symbol=\"ETHUSDT\" ;;\nesac\nprintf '%s\\n' '{\"type\":\"ready\",\"protocol_version\":\"2.0.0\"}'\nprintf '%s\\n' '{\"type\":\"event\",\"checkpoint\":\"cp-e2e-v2\",\"event_key\":\"event-external-v2\",\"occurred_at_ms\":1711200001000,\"payload\":{\"source\":\"external-v2\",\"symbol\":\"'\"$symbol\"'\"}}'\n",
    )
    .expect("v2 trigger plugin script should be writable");
    make_executable(&v2_script);

    let trigger_config_path = root
        .join("triggers")
        .join("external-trigger-e2e")
        .join("config.toml");
    let trigger_config = fs::read_to_string(&trigger_config_path)
        .expect("trigger config should be readable for plugin switch");
    let updated_trigger_config = trigger_config
        .lines()
        .map(|line| {
            if line.trim_start().starts_with("plugin = ") {
                String::from("plugin = \"trigger-e2e-plugin-v2\"")
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(&trigger_config_path, format!("{updated_trigger_config}\n"))
        .expect("trigger config should be updated to v2 plugin id");
}

fn install_secondary_external_trigger_package(root: &Path) {
    let trigger_dir = root.join("triggers").join("external-trigger-e2e-secondary");
    fs::create_dir_all(&trigger_dir)
        .expect("secondary trigger package directory should be creatable");
    fs::write(
        trigger_dir.join("config.toml"),
        "manifest_version = \"2.0.0\"\ntrigger_id = \"external-trigger-e2e-secondary\"\nkind = \"external_plugin\"\nplugin = \"trigger-e2e-plugin\"\nsource = \"external-trigger-plugin-secondary\"\nworkflow_id = \"wf-e2e\"\nenabled = true\n\n[params]\nsymbol = \"BTCUSDT\"\n\n[input_mapping]\nsymbol = \"payload.symbol\"\n",
    )
    .expect("secondary trigger config should be writable");
}

fn install_wasm_trigger_runtime_for_primary_external_plugin(root: &Path) {
    let plugin_config_path = root
        .join("plugins")
        .join("trigger-e2e-plugin")
        .join("config.toml");
    fs::write(
        plugin_config_path,
        "manifest_version = \"2.0.0\"\nplugin_id = \"trigger-e2e-plugin\"\nkind = \"external_trigger\"\nentrypoint = \"trigger.exec.v1\"\ncapabilities = [\"trigger.listen.event\"]\n\n[trigger_runtime]\nlifecycle = \"wasm_daemon_persistent_session\"\npush_callback = \"host_callback\"\ndurable_ack = \"after_store_persist\"\nhost_error_categories = [\"transport\", \"protocol_contract\", \"plugin_fatal\"]\nmodule = \"../bin/external_trigger.wat\"\n\n[event_schema]\nsummary = \"External trigger payload\"\nfields = [\"symbol\", \"price\"]\n",
    )
    .expect("wasm trigger plugin config should be writable");

    let guest_envelope = serde_json::json!({
        "event_key": "guest-callback-event",
        "occurred_at_ms": 1_710_600_100_111_i64,
        "checkpoint": "cp-guest-callback",
        "payload": {
            "symbol": "SOLUSDT",
            "price": 204.75,
            "producer": "guest"
        }
    });
    let guest_envelope_json =
        serde_json::to_string(&guest_envelope).expect("guest callback envelope should serialize");
    fs::write(
        root.join("plugins")
            .join("bin")
            .join("external_trigger.wat"),
        render_guest_callback_session_module_wat(&guest_envelope_json),
    )
    .expect("guest-envelope wasm module should be writable for fixture fidelity");
}

fn render_guest_callback_session_module_wat(guest_envelope_json: &str) -> String {
    let escaped_bytes = guest_envelope_json
        .as_bytes()
        .iter()
        .map(|byte| format!("\\{:02x}", byte))
        .collect::<String>();
    format!(
        "(module\n  (import \"trigger-host\" \"push-trigger-event\" (func $push-trigger-event (param i32 i32) (result i32)))\n  (memory (export \"memory\") 1)\n  (data (i32.const 0) \"{escaped_bytes}\")\n  (func (export \"run-session\")\n    i32.const 0\n    i32.const {}\n    call $push-trigger-event\n    drop\n  )\n)\n",
        guest_envelope_json.as_bytes().len()
    )
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

#[derive(Debug, Clone)]
struct TriggerEventRow {
    event_id: String,
    payload: serde_json::Value,
}

fn runtime_db_path(root: &Path) -> PathBuf {
    root.join("state").join("runtime.sqlite3")
}

fn ensure_runtime_db_ready(root: &Path) {
    let output = run_chainbot(["status", "--json"], root, false);
    assert!(
        output.status.success(),
        "status --json should initialize runtime DB before direct sqlite writes"
    );
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

fn insert_staged_trigger_event_record(
    root: &Path,
    staging_id: &str,
    trigger_id: &str,
    workflow_id: &str,
    event_id: &str,
    source: &str,
    occurred_at_ms: i64,
    staged_at_ms: i64,
    checkpoint: Option<&str>,
    payload: serde_json::Value,
) {
    let connection = open_runtime_db(root);
    connection
        .execute(
            "INSERT INTO staged_trigger_event_records (
                staging_id, schema_version, trigger_id, workflow_id, event_id, source,
                occurred_at_ms, staged_at_ms, checkpoint, payload_json,
                dedup_key, dedup_window_ms, cooldown_key, cooldown_ms,
                accepted_at_ms, last_error
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, NULL, NULL, NULL, NULL, NULL, NULL)
             ON CONFLICT(trigger_id, event_id) DO NOTHING",
            params![
                staging_id,
                "1.0.0",
                trigger_id,
                workflow_id,
                event_id,
                source,
                occurred_at_ms,
                staged_at_ms,
                checkpoint,
                serde_json::to_string(&payload).expect("staged payload should serialize"),
            ],
        )
        .expect("staged trigger event should insert");
}

fn count_pending_staged_trigger_event_records(root: &Path, trigger_id: &str, limit: i64) -> usize {
    let connection = open_runtime_db(root);
    connection
        .query_row(
            "SELECT COUNT(*) FROM (
                SELECT 1
                FROM staged_trigger_event_records
                WHERE trigger_id = ?1 AND accepted_at_ms IS NULL
                ORDER BY staged_at_ms ASC, staging_id ASC
                LIMIT ?2
             )",
            params![trigger_id, limit.max(1)],
            |row| row.get::<_, i64>(0),
        )
        .expect("pending staged trigger event count should query") as usize
}

fn list_recent_trigger_records(
    root: &Path,
    limit: i64,
    trigger_id: Option<&str>,
) -> Vec<TriggerEventRow> {
    let connection = open_runtime_db(root);
    let limit = limit.max(1);
    let mut rows = Vec::new();

    if let Some(trigger_id) = trigger_id {
        let mut statement = connection
            .prepare(
                "SELECT event_id, payload_json
                 FROM trigger_event_records
                 WHERE trigger_id = ?1
                 ORDER BY accepted_at_ms DESC, sequence DESC
                 LIMIT ?2",
            )
            .expect("prepare trigger record query by trigger_id should succeed");
        let iter = statement
            .query_map(params![trigger_id, limit], |row| {
                let payload_json: String = row.get(1)?;
                Ok(TriggerEventRow {
                    event_id: row.get(0)?,
                    payload: serde_json::from_str(&payload_json).unwrap_or(serde_json::Value::Null),
                })
            })
            .expect("query trigger records by trigger_id should succeed");
        for row in iter {
            rows.push(row.expect("trigger record row should decode"));
        }
        return rows;
    }

    let mut statement = connection
        .prepare(
            "SELECT event_id, payload_json
             FROM trigger_event_records
             ORDER BY accepted_at_ms DESC, trigger_id DESC, sequence DESC
             LIMIT ?1",
        )
        .expect("prepare trigger record query should succeed");
    let iter = statement
        .query_map(params![limit], |row| {
            let payload_json: String = row.get(1)?;
            Ok(TriggerEventRow {
                event_id: row.get(0)?,
                payload: serde_json::from_str(&payload_json).unwrap_or(serde_json::Value::Null),
            })
        })
        .expect("query trigger records should succeed");
    for row in iter {
        rows.push(row.expect("trigger record row should decode"));
    }
    rows
}

fn query_json_objects(connection: &Connection, query: &str) -> Vec<serde_json::Value> {
    let mut statement = connection
        .prepare(query)
        .expect("prepare sqlite JSON projection query should succeed");
    let column_names = statement
        .column_names()
        .into_iter()
        .map(|name| name.to_owned())
        .collect::<Vec<_>>();
    let rows = statement
        .query_map([], |row| {
            let mut object = serde_json::Map::new();
            for (index, name) in column_names.iter().enumerate() {
                let value = row
                    .get::<_, Option<String>>(index)
                    .ok()
                    .flatten()
                    .map(serde_json::Value::String)
                    .unwrap_or(serde_json::Value::Null);
                object.insert(name.clone(), value);
            }
            Ok(serde_json::Value::Object(object))
        })
        .expect("query sqlite JSON projection rows should succeed");
    rows.map(|row| row.expect("sqlite JSON row should decode"))
        .collect::<Vec<_>>()
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
