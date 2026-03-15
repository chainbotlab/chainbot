/*
[INPUT]:  Pass-style secret fixture paths, fake decryptors, and runtime-state persistence targets.
[OUTPUT]: Integration coverage for runtime secret resolution and redaction/persistence guarantees.
[POS]:    Integration test boundary for task-10 secret-provider runtime contracts.
[UPDATE]: 2026-03-16 - Add pass-style secret resolution and redaction persistence tests.
*/

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use chainbot::config::RootLayout;
use chainbot::errors::UserFacingError;
use chainbot::secrets::{
    redact_json_snapshot, redact_text, SecretDecryptError, SecretDecryptor, SecretProvider,
    SecretReference, SECRET_REDACTION_TOKEN,
};
use chainbot::state::{
    CoordinationStore, FileBackedStateStore, RunRecordSummary, RunStatus, StateLayout,
    WorkflowRuntimeLogEntry,
};

#[derive(Clone)]
struct RecordingDecryptor {
    expected_path: PathBuf,
    plaintext: String,
    calls: Arc<Mutex<Vec<PathBuf>>>,
}

impl RecordingDecryptor {
    fn new(expected_path: PathBuf, plaintext: String) -> Self {
        Self {
            expected_path,
            plaintext,
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn call_log(&self) -> Vec<PathBuf> {
        self.calls
            .lock()
            .expect("decrypt call log mutex should not be poisoned")
            .clone()
    }
}

impl SecretDecryptor for RecordingDecryptor {
    fn decrypt(&self, encrypted_file: &Path) -> Result<String, SecretDecryptError> {
        self.calls
            .lock()
            .expect("decrypt call log mutex should not be poisoned")
            .push(encrypted_file.to_path_buf());
        if encrypted_file != self.expected_path {
            return Err(SecretDecryptError::Io);
        }
        Ok(self.plaintext.clone())
    }
}

struct FailingDecryptor;

impl SecretDecryptor for FailingDecryptor {
    fn decrypt(&self, _encrypted_file: &Path) -> Result<String, SecretDecryptError> {
        Err(SecretDecryptError::ProcessFailed { exit_code: Some(1) })
    }
}

#[test]
fn secret_reference_resolution() {
    let fixture_path = secret_fixture_path();
    assert!(fixture_path.exists());

    let decryptor = RecordingDecryptor::new(
        fixture_path.clone(),
        "line-password\napi_token=token-alpha\nregion: us-east-1\n".to_string(),
    );
    let provider = SecretProvider::new(secret_fixture_root(), decryptor.clone());

    let keyed_reference =
        SecretReference::parse("secret://ops/slack/webhook#api_token").expect("ref should parse");
    let keyed_secret = provider
        .resolve_reference(&keyed_reference)
        .expect("keyed secret should resolve");
    assert_eq!(keyed_secret.expose(), "token-alpha");

    let plain_reference =
        SecretReference::parse("secret://ops/slack/webhook").expect("ref should parse");
    let plain_secret = provider
        .resolve_reference(&plain_reference)
        .expect("plain secret should resolve");
    assert_eq!(plain_secret.expose(), "line-password");

    assert_eq!(
        decryptor.call_log(),
        vec![fixture_path.clone(), fixture_path]
    );
}

#[test]
fn secret_decryption_failure_redaction() {
    let provider = SecretProvider::new(secret_fixture_root(), FailingDecryptor);
    let reference = SecretReference::parse("secret://ops/slack/webhook").expect("ref should parse");

    let error = provider
        .resolve_reference(&reference)
        .expect_err("decryption failure should be surfaced");
    let contract_render = error.to_string();
    let user_render = UserFacingError::from_contract(error).to_string();

    assert!(contract_render.contains("plaintext redacted"));
    assert!(user_render.contains("plaintext redacted"));
    assert!(!contract_render.contains("line-password"));
    assert!(!contract_render.contains("token-alpha"));
    assert!(!user_render.contains("line-password"));
    assert!(!user_render.contains("token-alpha"));
}

#[test]
fn secret_values_never_persisted() {
    let fixture_path = secret_fixture_path();
    let decryptor = RecordingDecryptor::new(
        fixture_path,
        "line-password\napi_token=token-alpha\n".to_string(),
    );
    let provider = SecretProvider::new(secret_fixture_root(), decryptor);

    let reference =
        SecretReference::parse("secret://ops/slack/webhook#api_token").expect("ref should parse");
    let secret = provider
        .resolve_reference(&reference)
        .expect("secret should resolve");

    let secret_values = vec![secret.clone()];
    let raw_log_message = format!("dispatching worker payload with token {}", secret.expose());
    let redacted_log_message = redact_text(&raw_log_message, &secret_values);
    assert!(redacted_log_message.contains(SECRET_REDACTION_TOKEN));
    assert!(!redacted_log_message.contains(secret.expose()));

    let worker_payload = serde_json::json!({
        "token": secret.expose(),
        "headers": {
            "authorization": format!("Bearer {}", secret.expose())
        },
        "items": [secret.expose()]
    });
    let payload_snapshot = redact_json_snapshot(&worker_payload, &secret_values);
    let payload_snapshot_text =
        serde_json::to_string(&payload_snapshot).expect("snapshot json should serialize");
    assert!(payload_snapshot_text.contains(SECRET_REDACTION_TOKEN));
    assert!(!payload_snapshot_text.contains(secret.expose()));

    let layout = unique_state_layout("secret-values-never-persisted");
    let state_store = FileBackedStateStore::new(layout.clone());
    state_store
        .initialize()
        .expect("state layout should initialize");

    state_store
        .write_run_summary(&RunRecordSummary {
            schema_version: "1.0.0".to_string(),
            run_id: "run-secret-boundary".to_string(),
            workflow_id: "wf-secret-boundary".to_string(),
            status: RunStatus::Running,
            started_at_ms: 1_711_000_000_000,
            finished_at_ms: None,
        })
        .expect("run summary should persist");

    state_store
        .write_workflow_log_entry(&WorkflowRuntimeLogEntry {
            run_id: "run-secret-boundary".to_string(),
            sequence: 1,
            event: "payload_snapshot_redacted".to_string(),
            message: redacted_log_message,
            occurred_at_ms: 1_711_000_000_100,
        })
        .expect("workflow log should persist");

    let _coordination =
        CoordinationStore::open(&layout, 1_711_000_000_200).expect("sqlite should initialize");

    let mut file_paths = Vec::new();
    collect_all_files(&layout.state_root, &mut file_paths);
    for file_path in file_paths {
        let raw = fs::read(&file_path).expect("persisted state file should be readable");
        let text = String::from_utf8_lossy(&raw);
        assert!(
            !text.contains(secret.expose()),
            "secret leaked into persisted artifact: {}",
            file_path.display()
        );
    }
}

fn secret_fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

fn secret_fixture_path() -> PathBuf {
    secret_fixture_root()
        .join("ops")
        .join("slack")
        .join("webhook.gpg")
}

fn unique_state_layout(prefix: &str) -> StateLayout {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after UNIX_EPOCH")
        .as_nanos();
    let root = workspace_root()
        .join("target")
        .join("test-roots")
        .join(format!("{prefix}-{now}"));
    let root_layout = RootLayout::from_root(root);
    StateLayout::from_root_layout(&root_layout)
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

fn collect_all_files(root: &Path, file_paths: &mut Vec<PathBuf>) {
    if !root.exists() {
        return;
    }

    let entries = fs::read_dir(root).expect("directory entries should be readable");
    for entry in entries {
        let entry = entry.expect("directory entry should decode");
        let path = entry.path();
        if path.is_dir() {
            collect_all_files(&path, file_paths);
        } else {
            file_paths.push(path);
        }
    }
}
