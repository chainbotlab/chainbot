//! [INPUT]
//! Full e2e fixture roots, daemon lifecycle commands, live webhook/websocket clients, and persisted runtime artifacts.
//!
//! [OUTPUT]
//! Verifies listener-backed ingress triggers accept live network input and persist resulting runs through the normal serve path.
//!
//! [ROLE]
//! Covers webhook/websocket ingress as an integration boundary above the trigger plane.

use std::fs;
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::Duration;

use chainbot::domain::state::TriggerEventRecord;
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};

#[test]
fn webhook_trigger_accepts_live_post() {
    let _guard = fixture_lock().lock().unwrap_or_else(|error| error.into_inner());
    let port = reserve_free_port();
    let root = prepare_ingress_root("ingress-webhook", None, None);
    write_webhook_trigger(&root, port);

    let serve_output = run_chainbot(["serve"], &root, true);
    assert!(serve_output.status.success(), "serve failed: {}", String::from_utf8_lossy(&serve_output.stderr));
    wait_for_serve_state(&root, "active");
    wait_for_listener_ready(port);

    let baseline_runs = read_run_count(&root);
    let response = reqwest::blocking::Client::new()
        .post(format!("http://127.0.0.1:{port}/ingress/webhook"))
        .header("content-type", "application/json")
        .header("x-chainbot-token", "dev-webhook-token")
        .header("x-event-id", "evt-webhook-1")
        .body(
            serde_json::json!({"symbol": "SOLUSDT", "price": 123.45, "id": "evt-webhook-1"})
                .to_string(),
        )
        .send()
        .expect("webhook request should succeed");
    assert_eq!(response.status(), reqwest::StatusCode::ACCEPTED);

    wait_for_run_count(&root, baseline_runs + 1);
    assert_eq!(read_run_count(&root), baseline_runs + 1);
    assert_eq!(latest_trigger_id(&root).as_deref(), Some("webhook-ingress"));
    let stop_output = run_chainbot(["stop"], &root, true);
    assert!(stop_output.status.success());
}

#[test]
fn websocket_trigger_accepts_live_message_and_enforces_connection_limit() {
    let _guard = fixture_lock().lock().unwrap_or_else(|error| error.into_inner());
    let port = reserve_free_port();
    let root = prepare_ingress_root("ingress-websocket", None, None);
    write_websocket_trigger(&root, port, 1, 30_000);

    let serve_output = run_chainbot(["serve"], &root, true);
    assert!(serve_output.status.success(), "serve failed: {}", String::from_utf8_lossy(&serve_output.stderr));
    wait_for_serve_state(&root, "active");
    wait_for_listener_ready(port);

    let baseline_runs = read_run_count(&root);
    with_async_runtime(async {
        let mut request = format!("ws://127.0.0.1:{port}/ingress/ws")
            .into_client_request()
            .expect("websocket request should build");
        request.headers_mut().insert(
            "x-chainbot-token",
            "dev-websocket-token".parse().expect("token header should parse"),
        );
        let (mut stream, _) = tokio_tungstenite::connect_async(request)
            .await
            .expect("first websocket connection should succeed");
        let mut second_request = format!("ws://127.0.0.1:{port}/ingress/ws")
            .into_client_request()
            .expect("second websocket request should build");
        second_request.headers_mut().insert(
            "x-chainbot-token",
            "dev-websocket-token"
                .parse()
                .expect("second token header should parse"),
        );
        let second = tokio_tungstenite::connect_async(second_request)
            .await;
        assert!(second.is_err(), "second connection should be rejected when max_connections=1");

        use futures_util::SinkExt;
        stream
            .send(Message::Text(
                serde_json::json!({"symbol": "DOGEUSDT", "price": 0.42, "event": "ws-open"})
                    .to_string()
                    .into(),
            ))
            .await
            .expect("websocket text frame should send");
        let _ = stream.close(None).await;
    });

    wait_for_run_count(&root, baseline_runs + 1);
    assert_eq!(read_run_count(&root), baseline_runs + 1);
    assert_eq!(latest_trigger_id(&root).as_deref(), Some("websocket-ingress"));
    let stop_output = run_chainbot(["stop"], &root, true);
    assert!(stop_output.status.success());
}

#[test]
fn websocket_trigger_closes_idle_connections() {
    let _guard = fixture_lock().lock().unwrap_or_else(|error| error.into_inner());
    let port = reserve_free_port();
    let root = prepare_ingress_root("ingress-websocket-idle", None, None);
    write_websocket_trigger(&root, port, 2, 200);

    let serve_output = run_chainbot(["serve"], &root, true);
    assert!(serve_output.status.success(), "serve failed: {}", String::from_utf8_lossy(&serve_output.stderr));
    wait_for_serve_state(&root, "active");
    wait_for_listener_ready(port);

    with_async_runtime(async {
        let mut request = format!("ws://127.0.0.1:{port}/ingress/ws")
            .into_client_request()
            .expect("websocket request should build");
        request.headers_mut().insert(
            "x-chainbot-token",
            "dev-websocket-token".parse().expect("token header should parse"),
        );
        let (mut stream, _) = tokio_tungstenite::connect_async(request)
            .await
            .expect("websocket connection should succeed");
        tokio::time::sleep(Duration::from_millis(450)).await;
        use futures_util::StreamExt;
        let next = tokio::time::timeout(Duration::from_secs(1), stream.next())
            .await
            .expect("idle websocket should close within timeout");
        assert!(next.is_none() || matches!(next, Some(Ok(Message::Close(_))) | Some(Err(_))));
    });

    let stop_output = run_chainbot(["stop"], &root, true);
    assert!(stop_output.status.success());
}

#[test]
fn accepted_trigger_record_without_run_summary_replays_on_restart() {
    let _guard = fixture_lock().lock().unwrap_or_else(|error| error.into_inner());
    let root = prepare_ingress_root("ingress-replay", None, None);

    let mut store = open_runtime_store(&root, 1_710_500_000_000);
    store
        .write_trigger_record(&TriggerEventRecord {
            schema_version: String::from("1.0.0"),
            run_id: String::from("replay-run-1"),
            sequence: 1,
            trigger_id: String::from("replay-trigger"),
            workflow_id: String::from("wf-e2e"),
            event_id: String::from("accepted-event-1"),
            checkpoint: None,
            source: String::from("webhook"),
            accepted_at_ms: 1_710_500_000_000,
            payload: serde_json::json!({"symbol": "ADAUSDT"}),
            dedup_key: None,
            dedup_expires_at_ms: None,
            cooldown_key: None,
            cooldown_expires_at_ms: None,
        })
        .expect("accepted trigger record should persist for replay test");

    let serve_output = run_chainbot(["serve"], &root, true);
    assert!(serve_output.status.success(), "serve failed: {}", String::from_utf8_lossy(&serve_output.stderr));
    wait_for_run_count(&root, 1);
    assert_eq!(read_run_count(&root), 1);
    assert!(read_run_ids(&root).contains(&String::from("replay-run-1")));

    let stop_output = run_chainbot(["stop"], &root, true);
    assert!(stop_output.status.success());
}

fn run_chainbot<const N: usize>(args: [&str; N], root: &Path, with_plaintext_secrets: bool) -> std::process::Output {
    let mut command = Command::new(chainbot_bin());
    command.env("CHAINBOT_CONFIG_DIR", root).args(args);
    if with_plaintext_secrets {
        command.env("CHAINBOT_SECRET_DECRYPTOR", "plaintext");
    }
    command.output().expect("chainbot command should execute")
}

fn prepare_ingress_root(root_name: &str, _host: Option<&str>, _port: Option<u16>) -> PathBuf {
    let root = workspace_root().join("target").join("test-roots").join(root_name);
    if root.exists() {
        fs::remove_dir_all(&root).expect("existing ingress root should be removable");
    }
    let source = fixture_root().join("e2e").join("success");
    copy_directory_recursive(&source, &root);
    let root_config_path = root.join("chainbot.toml");
    let root_config = fs::read_to_string(&root_config_path).expect("root config should be readable");
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
    fs::write(&root_config_path, format!("{updated_root_config}\n")).expect("root config should be rewritten");
    make_executable(&root.join("plugins").join("bin").join("external_trigger.sh"));
    make_executable(&root.join("plugins").join("bin").join("external_node.sh"));

    let external_trigger_path = root.join("triggers").join("external-trigger-e2e").join("config.toml");
    let external_trigger = fs::read_to_string(&external_trigger_path).expect("external trigger config should be readable");
    fs::write(&external_trigger_path, external_trigger.replace("enabled = true", "enabled = false"))
        .expect("external trigger fixture should be disableable");
    root
}

fn write_webhook_trigger(root: &Path, port: u16) {
    let trigger_dir = root.join("triggers").join("webhook-ingress");
    fs::create_dir_all(&trigger_dir).expect("webhook trigger dir should be creatable");
    fs::write(
        trigger_dir.join("config.toml"),
        format!(
            "manifest_version = \"2.0.0\"\ntrigger_id = \"webhook-ingress\"\nkind = \"builtin\"\nsource = \"webhook\"\nworkflow_id = \"wf-e2e\"\nenabled = true\n\n[params]\nbind = \"127.0.0.1:{port}\"\npath = \"/ingress/webhook\"\nmethod = \"POST\"\nmax_body_bytes = 65536\ncontent_type = \"application/json\"\nidempotency_header = \"x-event-id\"\n\n[params.auth]\nkind = \"header_token\"\nheader_name = \"x-chainbot-token\"\ntoken = \"dev-webhook-token\"\n\n[input_mapping]\nsymbol = \"payload.symbol\"\n"
        ),
    )
    .expect("webhook trigger config should be writable");
}

fn write_websocket_trigger(root: &Path, port: u16, max_connections: usize, idle_timeout_ms: i64) {
    let trigger_dir = root.join("triggers").join("websocket-ingress");
    fs::create_dir_all(&trigger_dir).expect("websocket trigger dir should be creatable");
    fs::write(
        trigger_dir.join("config.toml"),
        format!(
            "manifest_version = \"2.0.0\"\ntrigger_id = \"websocket-ingress\"\nkind = \"builtin\"\nsource = \"websocket\"\nworkflow_id = \"wf-e2e\"\nenabled = true\n\n[params]\nbind = \"127.0.0.1:{port}\"\npath = \"/ingress/ws\"\nmax_connections = {max_connections}\nmax_message_bytes = 65536\nidle_timeout_ms = {idle_timeout_ms}\n\n[params.auth]\nkind = \"header_token\"\nheader_name = \"x-chainbot-token\"\ntoken = \"dev-websocket-token\"\n\n[input_mapping]\nsymbol = \"payload.symbol\"\n"
        ),
    )
    .expect("websocket trigger config should be writable");
}

fn read_run_count(root: &Path) -> usize {
    let list_runs_output = run_chainbot(["list-runs"], root, false);
    assert!(list_runs_output.status.success(), "list-runs failed: {}", String::from_utf8_lossy(&list_runs_output.stderr));
    let runs: Vec<serde_json::Value> = serde_json::from_slice(&list_runs_output.stdout).expect("list-runs output should decode");
    runs.len()
}

fn read_run_ids(root: &Path) -> Vec<String> {
    let list_runs_output = run_chainbot(["list-runs"], root, false);
    assert!(list_runs_output.status.success(), "list-runs failed: {}", String::from_utf8_lossy(&list_runs_output.stderr));
    let runs: Vec<serde_json::Value> = serde_json::from_slice(&list_runs_output.stdout).expect("list-runs output should decode");
    runs.into_iter()
        .filter_map(|run| run.get("run_id").and_then(|value| value.as_str()).map(ToOwned::to_owned))
        .collect()
}

fn wait_for_run_count(root: &Path, expected_min_runs: usize) {
    for _ in 0..50 {
        if read_run_count(root) >= expected_min_runs {
            return;
        }
        thread::sleep(Duration::from_millis(100));
    }
    panic!("timed out waiting for at least {expected_min_runs} persisted runs");
}

fn latest_trigger_id(root: &Path) -> Option<String> {
    let mut store = open_runtime_store(root, 1_710_500_000_000);
    store
        .list_recent_trigger_records(1, None)
        .expect("runtime store should list latest trigger record")
        .into_iter()
        .next()
        .map(|record| record.trigger_id)
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

fn wait_for_listener_ready(port: u16) {
    for _ in 0..50 {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return;
        }
        thread::sleep(Duration::from_millis(100));
    }
    panic!("timed out waiting for listener on 127.0.0.1:{port}");
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
            fs::copy(&source_path, &destination_path).expect("fixture file should copy");
        }
    }
}

fn with_async_runtime<F>(future: F)
where
    F: std::future::Future<Output = ()>,
{
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime should build")
        .block_on(future);
}

fn open_runtime_store(root: &Path, now_ms: i64) -> chainbot::infrastructure::state::RuntimeStateStore {
    chainbot::infrastructure::state::RuntimeStateStore::open(
        &chainbot::infrastructure::config::RuntimeStorageConfig {
            backend: chainbot::infrastructure::config::RuntimeStorageBackend::Local {
                database_path: root.join("state").join("runtime.sqlite3"),
            },
            history_retention: None,
            raw_debug_enabled: false,
            raw_debug_artifacts_dir: None,
        },
        now_ms,
    )
    .expect("runtime store should open for ingress assertions")
}

fn reserve_free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("ephemeral test port should bind");
    let port = listener
        .local_addr()
        .expect("ephemeral test port should have local addr")
        .port();
    drop(listener);
    port
}

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures")
}

fn chainbot_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_chainbot"))
}

fn workspace_root() -> PathBuf {
    let crate_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    crate_root.parent().expect("crates dir").parent().expect("workspace root").to_path_buf()
}

fn make_executable(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path).expect("fixture script metadata should exist").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).expect("fixture script should be executable");
    }
}

fn fixture_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}
