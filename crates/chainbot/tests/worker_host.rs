//! [INPUT]
//! Worker host process specs, protocol envelopes, fixture worker scripts, and host safety limits.
//!
//! [OUTPUT]
//! Verifies subprocess worker-host protocol handling, failure semantics, and cleanup behavior.
//!
//! [ROLE]
//! Covers the script worker-host boundary as an integration test.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use chainbot::builtins::nodes::script_worker::{
    ScriptRuntime, WorkerHost, WorkerHostError, WorkerHostLimits, WorkerProcessSpec,
};
use chainbot::errors::ContractError;
use chainbot::script_protocol::WorkerRequestEnvelope;

#[test]
fn worker_protocol_version_negotiation() {
    let request_error = WorkerRequestEnvelope::from_json_str(
        r#"{
          "protocol_version": "2.0.0",
          "request_id": "req-future",
          "worker_id": "python-worker",
          "workflow_id": "wf-worker",
          "payload": {}
        }"#,
    )
    .expect_err("future-major request protocol must be rejected");
    assert!(matches!(
        request_error,
        ContractError::UnsupportedFutureMajorVersion {
            field: "worker_request.protocol_version",
            major: 2,
            max_supported_major: 1
        }
    ));

    let Some(python_interpreter) = resolve_interpreter(&["python3", "python"]) else {
        return;
    };

    let host = WorkerHost::new(WorkerHostLimits {
        timeout: Duration::from_secs(1),
        max_stdout_bytes: 64 * 1024,
        max_stderr_bytes: 64 * 1024,
    });

    let process = WorkerProcessSpec::new(
        ScriptRuntime::Python,
        python_interpreter,
        fixture_script_path("python_worker.py"),
    );
    let request = base_request(serde_json::json!({"mode": "future_protocol"}));

    let error = host
        .execute(&process, &request)
        .expect_err("future-major response protocol must be rejected");

    assert!(matches!(
        error,
        WorkerHostError::InvalidEnvelope(ContractError::UnsupportedFutureMajorVersion {
            field: "worker_response.protocol_version",
            major: 9,
            max_supported_major: 1
        })
    ));
}

#[test]
fn python_and_javascript_worker_roundtrip() {
    let host = WorkerHost::new(WorkerHostLimits {
        timeout: Duration::from_secs(1),
        max_stdout_bytes: 64 * 1024,
        max_stderr_bytes: 64 * 1024,
    });

    let Some(python_interpreter) = resolve_interpreter(&["python3", "python"]) else {
        return;
    };
    let Some(node_interpreter) = resolve_interpreter(&["node", "nodejs"]) else {
        return;
    };

    let python_request = base_request(serde_json::json!({
        "mode": "echo",
        "language": "python",
        "value": 17
    }));
    let python_process = WorkerProcessSpec::new(
        ScriptRuntime::Python,
        python_interpreter,
        fixture_script_path("python_worker.py"),
    );
    let python_response = host
        .execute(&python_process, &python_request)
        .expect("python worker roundtrip should succeed");

    assert!(python_response.success);
    assert_eq!(python_response.protocol_version, "1.0.0");
    assert_eq!(python_response.request_id, python_request.request_id);
    assert_eq!(
        python_response.output["runtime"],
        serde_json::json!("python")
    );
    assert_eq!(python_response.output["payload"]["language"], "python");
    assert_eq!(python_response.output["payload"]["value"], 17);

    let javascript_request = base_request(serde_json::json!({
        "mode": "echo",
        "language": "javascript",
        "value": 29
    }));
    let javascript_process = WorkerProcessSpec::new(
        ScriptRuntime::JavaScript,
        node_interpreter,
        fixture_script_path("javascript_worker.js"),
    );
    let javascript_response = host
        .execute(&javascript_process, &javascript_request)
        .expect("javascript worker roundtrip should succeed");

    assert!(javascript_response.success);
    assert_eq!(javascript_response.protocol_version, "1.0.0");
    assert_eq!(
        javascript_response.request_id,
        javascript_request.request_id
    );
    assert_eq!(
        javascript_response.output["runtime"],
        serde_json::json!("javascript")
    );
    assert_eq!(
        javascript_response.output["payload"]["language"],
        "javascript"
    );
    assert_eq!(javascript_response.output["payload"]["value"], 29);
}

#[test]
fn worker_failure_channels_are_typed_errors() {
    let Some(python_interpreter) = resolve_interpreter(&["python3", "python"]) else {
        return;
    };

    let host = WorkerHost::new(WorkerHostLimits {
        timeout: Duration::from_secs(1),
        max_stdout_bytes: 64 * 1024,
        max_stderr_bytes: 64 * 1024,
    });

    let script_path = unique_tmp_python_script("worker-failure-channels");
    let script = r#"import json
import sys

request = json.loads(sys.stdin.read())
mode = request.get("payload", {}).get("mode")

if mode == "success_false":
    response = {
        "protocol_version": request.get("protocol_version"),
        "request_id": request.get("request_id"),
        "success": False,
        "output": {"error": "worker reported explicit failure"},
    }
    sys.stdout.write(json.dumps(response))
    sys.stdout.flush()
    raise SystemExit(0)

response = {
    "protocol_version": request.get("protocol_version"),
    "request_id": request.get("request_id"),
    "success": True,
    "output": {"runtime": "python"},
}
sys.stdout.write(json.dumps(response))
sys.stdout.flush()
sys.stderr.write("worker exiting with code 7\\n")
sys.stderr.flush()
raise SystemExit(7)
"#;
    write_executable_python_script(&script_path, script);

    let process = WorkerProcessSpec::new(ScriptRuntime::Python, python_interpreter, script_path);

    let reported_failure_request = base_request(serde_json::json!({"mode": "success_false"}));
    let reported_failure_error = host
        .execute(&process, &reported_failure_request)
        .expect_err("success=false response must be treated as worker failure");
    assert!(matches!(
        reported_failure_error,
        WorkerHostError::ReportedFailure {
            runtime: ScriptRuntime::Python,
            detail,
        } if detail.contains("explicit failure")
    ));

    let nonzero_exit_request = base_request(serde_json::json!({"mode": "nonzero_exit"}));
    let nonzero_exit_error = host
        .execute(&process, &nonzero_exit_request)
        .expect_err("non-zero worker exit must be treated as worker failure");
    assert!(matches!(
        nonzero_exit_error,
        WorkerHostError::NonZeroExit {
            runtime: ScriptRuntime::Python,
            exit_code: Some(7),
            stderr,
        } if stderr.contains("exiting with code 7")
    ));
}

#[test]
fn worker_timeout_and_oversized_output() {
    let Some(python_interpreter) = resolve_interpreter(&["python3", "python"]) else {
        return;
    };

    let host = WorkerHost::new(WorkerHostLimits {
        timeout: Duration::from_millis(120),
        max_stdout_bytes: 256,
        max_stderr_bytes: 128,
    });

    let process = WorkerProcessSpec::new(
        ScriptRuntime::Python,
        python_interpreter,
        fixture_script_path("python_worker.py"),
    );

    let pid_file = unique_tmp_file("worker-timeout-pid");
    if let Some(parent) = pid_file.parent() {
        fs::create_dir_all(parent).expect("timeout pid directory should be creatable");
    }
    let timeout_request = base_request(serde_json::json!({
        "mode": "sleep_with_pid",
        "duration_ms": 1500,
        "pid_file": pid_file
    }));
    let timeout_error = host
        .execute(&process, &timeout_request)
        .expect_err("sleeping worker must time out");
    assert!(matches!(
        timeout_error,
        WorkerHostError::Timeout {
            runtime: ScriptRuntime::Python,
            ..
        }
    ));

    let pid = wait_for_pid_file(&pid_file).expect("pid file should be written before timeout");
    assert!(wait_until_process_exits(pid, Duration::from_secs(2)));

    let oversized_stdout_request = base_request(serde_json::json!({
        "mode": "oversized_stdout",
        "size": 2048
    }));
    let oversized_stdout_error = host
        .execute(&process, &oversized_stdout_request)
        .expect_err("oversized stdout should be rejected");
    assert!(matches!(
        oversized_stdout_error,
        WorkerHostError::OversizedOutput {
            stream: "stdout",
            max_bytes: 256,
            actual_bytes
        } if actual_bytes > 256
    ));

    let oversized_stderr_request = base_request(serde_json::json!({
        "mode": "oversized_stderr",
        "size": 1024
    }));
    let oversized_stderr_error = host
        .execute(&process, &oversized_stderr_request)
        .expect_err("oversized stderr should be rejected");
    assert!(matches!(
        oversized_stderr_error,
        WorkerHostError::OversizedOutput {
            stream: "stderr",
            max_bytes: 128,
            actual_bytes
        } if actual_bytes > 128
    ));

    let malformed_request = base_request(serde_json::json!({"mode": "malformed"}));
    let malformed_error = host
        .execute(&process, &malformed_request)
        .expect_err("malformed stdout must be rejected");
    assert!(matches!(
        malformed_error,
        WorkerHostError::MalformedResponse(_)
    ));
}

fn base_request(payload: serde_json::Value) -> WorkerRequestEnvelope {
    WorkerRequestEnvelope {
        protocol_version: "1.0.0".to_string(),
        request_id: format!("req-{}", unique_suffix()),
        worker_id: "worker-test".to_string(),
        workflow_id: "wf-worker-test".to_string(),
        payload,
    }
}

fn fixture_script_path(script_name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("workers")
        .join(script_name)
}

fn resolve_interpreter(candidates: &[&str]) -> Option<PathBuf> {
    for candidate in candidates {
        let path = PathBuf::from(candidate);
        if path.is_absolute() && path.is_file() {
            return Some(path);
        }

        if candidate.contains(std::path::MAIN_SEPARATOR) {
            if path.is_file() {
                return Some(path);
            }
            continue;
        }

        let Some(path_env) = env::var_os("PATH") else {
            continue;
        };

        for dir in env::split_paths(&path_env) {
            let full = dir.join(candidate);
            if full.is_file() {
                return Some(full);
            }
        }
    }

    None
}

fn unique_tmp_file(prefix: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("test-roots")
        .join(format!("{prefix}-{}.tmp", unique_suffix()))
}

fn unique_tmp_python_script(prefix: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("test-roots")
        .join(format!("{prefix}-{}.py", unique_suffix()))
}

fn write_executable_python_script(path: &Path, script: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("temporary script directory should be creatable");
    }
    fs::write(path, script).expect("temporary worker script should be writable");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path)
            .expect("temporary worker script metadata should exist")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions)
            .expect("temporary worker script should be executable");
    }
}

fn wait_for_pid_file(path: &Path) -> Option<u32> {
    let deadline = Instant::now() + Duration::from_millis(500);
    while Instant::now() < deadline {
        if let Ok(contents) = fs::read_to_string(path) {
            if let Ok(pid) = contents.trim().parse::<u32>() {
                return Some(pid);
            }
        }
        thread::sleep(Duration::from_millis(10));
    }
    None
}

fn wait_until_process_exits(pid: u32, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if !process_exists(pid) {
            return true;
        }
        thread::sleep(Duration::from_millis(20));
    }
    !process_exists(pid)
}

fn process_exists(pid: u32) -> bool {
    let Ok(output) = Command::new("ps").arg("-p").arg(pid.to_string()).output() else {
        return false;
    };

    output.status.success() && String::from_utf8_lossy(&output.stdout).lines().count() > 1
}

fn unique_suffix() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after UNIX_EPOCH")
        .as_nanos();
    format!("{nanos}")
}
