//! [INPUT]
//! MCP external-node manifests, local stdio and streamable HTTP fixtures, plugin host roots, and request payloads.
//!
//! [OUTPUT]
//! Verifies stdio-backed and streamable-HTTP-backed MCP session startup, tool discovery/call success, and deterministic fail-closed host errors.
//!
//! [ROLE]
//! Covers the MCP transport boundaries for the external node plugin host.

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderName, HeaderValue, Response, StatusCode};
use axum::response::sse::KeepAlive;
use axum::response::{IntoResponse, Sse};
use axum::routing::post;
use axum::{Json, Router};
use chainbot::errors::ContractError;
use chainbot::plugin::{
    ExternalNodePluginHost, ExternalNodePluginRequest, McpAuthConfig, McpPluginContract,
    McpStdioTransportConfig, McpStreamableHttpTransportConfig, McpTransportKind,
    PluginHostSecretMode, PluginManifest, PluginOperationDescriptor,
    EXTERNAL_NODE_ENTRYPOINT_MCP_TOOL_V1, NODE_PLUGIN_EXECUTE_CAPABILITY,
    PLUGIN_KIND_EXTERNAL_NODE,
};
use futures_util::stream;
use serde_json::json;
use tokio::sync::oneshot;

const MCP_STDIO_FIXTURE_SCRIPT: &str = r#"#!/usr/bin/env python3
import json
import sys

MODE = sys.argv[1] if len(sys.argv) > 1 else "happy"


def read_message():
    line = sys.stdin.readline()
    if not line:
        return None
    return json.loads(line)


def send_message(message):
    sys.stdout.write(json.dumps(message, separators=(",", ":")) + "\n")
    sys.stdout.flush()


def send_initialize_response(message):
    protocol_version = message.get("params", {}).get("protocolVersion", "2025-11-25")
    send_message(
        {
            "jsonrpc": "2.0",
            "id": message["id"],
            "result": {
                "protocolVersion": protocol_version,
                "capabilities": {"tools": {}},
                "serverInfo": {
                    "name": "chainbot-stdio-fixture",
                    "version": "1.0.0",
                },
            },
        }
    )


while True:
    message = read_message()
    if message is None:
        break

    method = message.get("method")

    if method == "initialize":
        if MODE == "initialize_failure":
            sys.exit(17)
        send_initialize_response(message)
        continue

    if method == "notifications/initialized":
        continue

    if method == "tools/list":
        if MODE == "malformed_payload":
            sys.stdout.write(
                '{"jsonrpc":"2.0","id":' + str(message["id"]) + ',"result":'
            )
            sys.stdout.flush()
            sys.exit(0)

        send_message(
            {
                "jsonrpc": "2.0",
                "id": message["id"],
                "result": {
                    "tools": [
                        {
                            "name": "echo",
                            "description": "Echo the incoming message",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "message": {"type": "string"}
                                },
                                "required": ["message"],
                            },
                        }
                    ]
                },
            }
        )
        continue

    if method == "tools/call":
        arguments = message.get("params", {}).get("arguments", {})
        send_message(
            {
                "jsonrpc": "2.0",
                "id": message["id"],
                "result": {
                    "content": [],
                    "structuredContent": {"message": arguments.get("message")},
                    "isError": False,
                },
            }
        )
        continue

    if message.get("id") is not None:
        send_message(
            {
                "jsonrpc": "2.0",
                "id": message["id"],
                "error": {"code": -32601, "message": "unsupported method"},
            }
        )
"#;

#[test]
fn stdio_tool_call_happy_path() {
    let root = unique_test_root("mcp-stdio-happy");
    let plugin_root = create_plugin_root(&root, "mcp-echo");
    write_mcp_stdio_fixture(&plugin_root.join("bin").join("mcp_stdio_fixture.py"));

    let host = ExternalNodePluginHost::new(root.join("plugins"));
    let manifest = stdio_manifest(
        "mcp-echo",
        &plugin_root,
        "bin/mcp_stdio_fixture.py",
        vec!["happy".to_owned()],
    );

    let result = host
        .execute_node_invocation(&manifest, &echo_request("mcp-echo", "hello"))
        .expect("stdio MCP tool invocation should normalize structured output");

    assert_eq!(result.output.get("message"), Some(&json!("hello")));
}

#[test]
fn stdio_missing_executable_fails_closed() {
    let root = unique_test_root("mcp-stdio-missing-executable");
    let plugin_root = create_plugin_root(&root, "mcp-missing");

    let host = ExternalNodePluginHost::new(root.join("plugins"));
    let manifest = stdio_manifest(
        "mcp-missing",
        &plugin_root,
        "bin/missing_fixture.py",
        vec!["happy".to_owned()],
    );

    let error = host
        .execute_node_invocation(&manifest, &echo_request("mcp-missing", "hello"))
        .expect_err("missing stdio executable must fail closed before spawn");

    assert!(matches!(
        error,
        ContractError::NodePluginInvalidExecutablePath {
            plugin_id,
            executable,
            ..
        } if plugin_id == "mcp-missing" && executable == "bin/missing_fixture.py"
    ));
}

#[test]
fn stdio_initialize_failure_maps_to_plugin_error() {
    let root = unique_test_root("mcp-stdio-initialize-failure");
    let plugin_root = create_plugin_root(&root, "mcp-init-failure");
    write_mcp_stdio_fixture(&plugin_root.join("bin").join("mcp_stdio_fixture.py"));

    let host = ExternalNodePluginHost::new(root.join("plugins"));
    let manifest = stdio_manifest(
        "mcp-init-failure",
        &plugin_root,
        "bin/mcp_stdio_fixture.py",
        vec!["initialize_failure".to_owned()],
    );

    let error = host
        .execute_node_invocation(&manifest, &echo_request("mcp-init-failure", "hello"))
        .expect_err("stdio MCP initialize failure must normalize to a plugin error");

    assert!(matches!(
        error,
        ContractError::NodePluginProtocolContractViolation { plugin_id, detail }
            if plugin_id == "mcp-init-failure"
                && detail.contains("before initialization completed")
    ));
}

#[test]
fn stdio_malformed_payload_rejected() {
    let root = unique_test_root("mcp-stdio-malformed-payload");
    let plugin_root = create_plugin_root(&root, "mcp-malformed");
    write_mcp_stdio_fixture(&plugin_root.join("bin").join("mcp_stdio_fixture.py"));

    let host = ExternalNodePluginHost::new(root.join("plugins"));
    let manifest = stdio_manifest(
        "mcp-malformed",
        &plugin_root,
        "bin/mcp_stdio_fixture.py",
        vec!["malformed_payload".to_owned()],
    );

    let error = host
        .execute_node_invocation(&manifest, &echo_request("mcp-malformed", "hello"))
        .expect_err("malformed stdio MCP payload must fail closed");

    assert!(matches!(
        error,
        ContractError::NodePluginProtocolContractViolation { plugin_id, detail }
            if plugin_id == "mcp-malformed"
                && detail.contains("valid response payload was received while attempting to list tools")
    ));
}

#[test]
fn http_tool_call_happy_path() {
    let root = unique_test_root("mcp-http-happy");
    let plugin_root = create_plugin_root(&root, "mcp-http-echo");
    let server = spawn_http_fixture(HttpFixtureMode::Happy, "x-chainbot-auth", "valid-token");
    write_plaintext_secret(
        &root,
        "secret://mcp/http-auth-token",
        "valid-token\n",
    );

    let host = ExternalNodePluginHost::with_secret_runtime(
        root.join("plugins"),
        root.join("secrets"),
        PluginHostSecretMode::Plaintext,
    );
    let manifest = http_manifest(
        "mcp-http-echo",
        &plugin_root,
        &server.url,
        Some((
            "x-chainbot-auth".to_owned(),
            "secret://mcp/http-auth-token".to_owned(),
        )),
    );

    let result = host
        .execute_node_invocation(&manifest, &echo_request("mcp-http-echo", "hello"))
        .expect("streamable HTTP MCP tool invocation should normalize structured output");

    assert_eq!(result.output.get("message"), Some(&json!("hello")));

    let snapshot = server.snapshot();
    assert_eq!(snapshot.initialize_count, 1);
    assert!(snapshot
        .session_headers_seen
        .iter()
        .all(|session_id| session_id == "session-1"));
    assert!(snapshot
        .auth_headers_seen
        .iter()
        .all(|value| value == "valid-token"));
    assert!(snapshot.session_headers_seen.len() >= 3);
}

#[test]
fn http_401_maps_to_plugin_error() {
    let root = unique_test_root("mcp-http-401");
    let plugin_root = create_plugin_root(&root, "mcp-http-401");
    let server = spawn_http_fixture(HttpFixtureMode::Unauthorized, "x-chainbot-auth", "valid-token");
    write_plaintext_secret(
        &root,
        "secret://mcp/http-auth-token",
        "invalid-token\n",
    );

    let host = ExternalNodePluginHost::with_secret_runtime(
        root.join("plugins"),
        root.join("secrets"),
        PluginHostSecretMode::Plaintext,
    );
    let manifest = http_manifest(
        "mcp-http-401",
        &plugin_root,
        &server.url,
        Some((
            "x-chainbot-auth".to_owned(),
            "secret://mcp/http-auth-token".to_owned(),
        )),
    );

    let error = host
        .execute_node_invocation(&manifest, &echo_request("mcp-http-401", "hello"))
        .expect_err("401 streamable HTTP auth failures must normalize to plugin errors");

    match error {
        ContractError::NodePluginProtocolContractViolation { plugin_id, detail } => {
            assert_eq!(plugin_id, "mcp-http-401");
            assert!(
                detail.contains("401")
                    || detail.to_ascii_lowercase().contains("unauthorized")
                    || detail.to_ascii_lowercase().contains("auth")
            );
            assert!(!detail.contains("invalid-token"));
        }
        other => panic!("expected protocol violation for HTTP 401, got {other:?}"),
    }
}

#[test]
fn http_stale_session_recovers_or_fails_deterministically() {
    let root = unique_test_root("mcp-http-stale-session");
    let plugin_root = create_plugin_root(&root, "mcp-http-stale");
    let server = spawn_http_fixture(
        HttpFixtureMode::StaleSessionRecover,
        "x-chainbot-auth",
        "valid-token",
    );
    write_plaintext_secret(
        &root,
        "secret://mcp/http-auth-token",
        "valid-token\n",
    );

    let host = ExternalNodePluginHost::with_secret_runtime(
        root.join("plugins"),
        root.join("secrets"),
        PluginHostSecretMode::Plaintext,
    );
    let manifest = http_manifest(
        "mcp-http-stale",
        &plugin_root,
        &server.url,
        Some((
            "x-chainbot-auth".to_owned(),
            "secret://mcp/http-auth-token".to_owned(),
        )),
    );

    let result = host
        .execute_node_invocation(&manifest, &echo_request("mcp-http-stale", "hello"))
        .expect("stale streamable HTTP sessions should recover within one invocation");

    assert_eq!(result.output.get("message"), Some(&json!("hello")));

    let snapshot = server.snapshot();
    assert!(snapshot.initialize_count >= 2);
    assert!(snapshot.saw_stale_session_404);
    assert!(snapshot
        .session_headers_seen
        .iter()
        .any(|session_id| session_id == "session-1"));
    assert!(snapshot
        .session_headers_seen
        .iter()
        .any(|session_id| session_id == "session-2"));
}

#[test]
fn http_non_mcp_endpoint_rejected() {
    let root = unique_test_root("mcp-http-non-mcp");
    let plugin_root = create_plugin_root(&root, "mcp-http-non-mcp");
    let server = spawn_http_fixture(HttpFixtureMode::NonMcp, "x-chainbot-auth", "valid-token");
    write_plaintext_secret(
        &root,
        "secret://mcp/http-auth-token",
        "valid-token\n",
    );

    let host = ExternalNodePluginHost::with_secret_runtime(
        root.join("plugins"),
        root.join("secrets"),
        PluginHostSecretMode::Plaintext,
    );
    let manifest = http_manifest(
        "mcp-http-non-mcp",
        &plugin_root,
        &server.url,
        Some((
            "x-chainbot-auth".to_owned(),
            "secret://mcp/http-auth-token".to_owned(),
        )),
    );

    let error = host
        .execute_node_invocation(&manifest, &echo_request("mcp-http-non-mcp", "hello"))
        .expect_err("non-MCP HTTP endpoints must fail closed");

    assert!(matches!(
        error,
        ContractError::NodePluginProtocolContractViolation { plugin_id, detail }
            if plugin_id == "mcp-http-non-mcp"
                && detail.contains("valid MCP")
    ));
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HttpFixtureMode {
    Happy,
    Unauthorized,
    StaleSessionRecover,
    NonMcp,
}

#[derive(Debug, Default)]
struct HttpFixtureSnapshot {
    initialize_count: usize,
    session_headers_seen: Vec<String>,
    auth_headers_seen: Vec<String>,
    saw_stale_session_404: bool,
}

#[derive(Debug)]
struct HttpFixtureSharedState {
    mode: HttpFixtureMode,
    expected_auth_header: String,
    expected_auth_value: String,
    initialize_count: usize,
    session_headers_seen: Vec<String>,
    auth_headers_seen: Vec<String>,
    stale_session_id: Option<String>,
    saw_stale_session_404: bool,
}

impl HttpFixtureSharedState {
    fn new(mode: HttpFixtureMode, expected_auth_header: &str, expected_auth_value: &str) -> Self {
        Self {
            mode,
            expected_auth_header: expected_auth_header.to_owned(),
            expected_auth_value: expected_auth_value.to_owned(),
            initialize_count: 0,
            session_headers_seen: Vec::new(),
            auth_headers_seen: Vec::new(),
            stale_session_id: None,
            saw_stale_session_404: false,
        }
    }

    fn snapshot(&self) -> HttpFixtureSnapshot {
        HttpFixtureSnapshot {
            initialize_count: self.initialize_count,
            session_headers_seen: self.session_headers_seen.clone(),
            auth_headers_seen: self.auth_headers_seen.clone(),
            saw_stale_session_404: self.saw_stale_session_404,
        }
    }
}

#[derive(Debug, Clone)]
struct HttpFixtureAppState {
    shared: Arc<Mutex<HttpFixtureSharedState>>,
}

struct HttpFixtureServer {
    url: String,
    shared: Arc<Mutex<HttpFixtureSharedState>>,
    shutdown: Option<oneshot::Sender<()>>,
    worker: Option<thread::JoinHandle<()>>,
}

impl HttpFixtureServer {
    fn snapshot(&self) -> HttpFixtureSnapshot {
        self.shared
            .lock()
            .expect("HTTP fixture state lock should not be poisoned")
            .snapshot()
    }
}

impl Drop for HttpFixtureServer {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }

        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn spawn_http_fixture(
    mode: HttpFixtureMode,
    expected_auth_header: &str,
    expected_auth_value: &str,
) -> HttpFixtureServer {
    let shared = Arc::new(Mutex::new(HttpFixtureSharedState::new(
        mode,
        expected_auth_header,
        expected_auth_value,
    )));
    let app = Router::new()
        .route(
            "/mcp",
            post(http_fixture_post)
                .get(http_fixture_get)
                .delete(http_fixture_delete),
        )
        .with_state(HttpFixtureAppState {
            shared: shared.clone(),
        });
    let listener = TcpListener::bind("127.0.0.1:0")
        .expect("HTTP fixture TCP listener should bind to an ephemeral port");
    listener
        .set_nonblocking(true)
        .expect("HTTP fixture TCP listener should support nonblocking mode");
    let address = listener
        .local_addr()
        .expect("HTTP fixture TCP listener should have a local address");
    let (shutdown_tx, shutdown_rx) = oneshot::channel();

    let worker = thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("HTTP fixture runtime should be constructible");
        runtime.block_on(async move {
            let listener = tokio::net::TcpListener::from_std(listener)
                .expect("HTTP fixture listener should convert into Tokio listener");
            axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    let _ = shutdown_rx.await;
                })
                .await
                .expect("HTTP fixture server should shut down cleanly");
        });
    });

    HttpFixtureServer {
        url: format!("http://{address}/mcp"),
        shared,
        shutdown: Some(shutdown_tx),
        worker: Some(worker),
    }
}

async fn http_fixture_post(
    State(state): State<HttpFixtureAppState>,
    headers: HeaderMap,
    body: String,
) -> Response<Body> {
    let mut shared = state
        .shared
        .lock()
        .expect("HTTP fixture state lock should not be poisoned");
    record_http_fixture_headers(&mut shared, &headers);

    if shared.mode == HttpFixtureMode::NonMcp {
        return html_response(StatusCode::OK, "<html>not an mcp endpoint</html>");
    }

    if auth_header_value(&shared, &headers) != Some(shared.expected_auth_value.clone()) {
        return unauthorized_response();
    }

    let payload: serde_json::Value = serde_json::from_str(&body)
        .expect("HTTP fixture should receive JSON-RPC payloads from the client");
    let method = payload
        .get("method")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let id = payload.get("id").cloned().unwrap_or(serde_json::Value::Null);

    match method {
        "initialize" => {
            shared.initialize_count += 1;
            let session_id = format!("session-{}", shared.initialize_count);
            if shared.mode == HttpFixtureMode::StaleSessionRecover && shared.stale_session_id.is_none() {
                shared.stale_session_id = Some(session_id.clone());
            }
            json_response(
                StatusCode::OK,
                Some(&session_id),
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "protocolVersion": "2025-11-25",
                        "capabilities": { "tools": {} },
                        "serverInfo": {
                            "name": "chainbot-http-fixture",
                            "version": "1.0.0"
                        }
                    }
                }),
            )
        }
        "notifications/initialized" => StatusCode::ACCEPTED.into_response(),
        "tools/list" => {
            if shared.mode == HttpFixtureMode::StaleSessionRecover
                && !shared.saw_stale_session_404
                && session_header_value(&headers).as_deref() == shared.stale_session_id.as_deref()
            {
                shared.saw_stale_session_404 = true;
                return StatusCode::NOT_FOUND.into_response();
            }

            json_response(
                StatusCode::OK,
                None,
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "tools": [
                            {
                                "name": "echo",
                                "description": "Echo the incoming message",
                                "inputSchema": {
                                    "type": "object",
                                    "properties": {
                                        "message": { "type": "string" }
                                    },
                                    "required": ["message"]
                                }
                            }
                        ]
                    }
                }),
            )
        }
        "tools/call" => {
            let message = payload
                .get("params")
                .and_then(|params| params.get("arguments"))
                .and_then(|arguments| arguments.get("message"))
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            json_response(
                StatusCode::OK,
                None,
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "content": [],
                        "structuredContent": { "message": message },
                        "isError": false
                    }
                }),
            )
        }
        _ => json_response(
            StatusCode::OK,
            None,
            json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {
                    "code": -32601,
                    "message": "unsupported method"
                }
            }),
        ),
    }
}

async fn http_fixture_get(
    State(state): State<HttpFixtureAppState>,
    headers: HeaderMap,
) -> Response<Body> {
    let mut shared = state
        .shared
        .lock()
        .expect("HTTP fixture state lock should not be poisoned");
    record_http_fixture_headers(&mut shared, &headers);

    if shared.mode == HttpFixtureMode::NonMcp {
        return html_response(StatusCode::OK, "<html>not an mcp endpoint</html>");
    }

    if auth_header_value(&shared, &headers) != Some(shared.expected_auth_value.clone()) {
        return unauthorized_response();
    }

    Sse::new(stream::pending::<Result<axum::response::sse::Event, Infallible>>())
        .keep_alive(KeepAlive::default())
        .into_response()
}

async fn http_fixture_delete(
    State(state): State<HttpFixtureAppState>,
    headers: HeaderMap,
) -> Response<Body> {
    let mut shared = state
        .shared
        .lock()
        .expect("HTTP fixture state lock should not be poisoned");
    record_http_fixture_headers(&mut shared, &headers);

    StatusCode::NO_CONTENT.into_response()
}

fn record_http_fixture_headers(shared: &mut HttpFixtureSharedState, headers: &HeaderMap) {
    if let Some(session_id) = session_header_value(headers) {
        shared.session_headers_seen.push(session_id);
    }

    if let Some(auth_value) = auth_header_value(shared, headers) {
        shared.auth_headers_seen.push(auth_value);
    }
}

fn session_header_value(headers: &HeaderMap) -> Option<String> {
    headers
        .get("mcp-session-id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

fn auth_header_value(shared: &HttpFixtureSharedState, headers: &HeaderMap) -> Option<String> {
    headers
        .get(shared.expected_auth_header.as_str())
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

fn unauthorized_response() -> Response<Body> {
    let mut response = StatusCode::UNAUTHORIZED.into_response();
    response.headers_mut().insert(
        HeaderName::from_static("www-authenticate"),
        HeaderValue::from_static("Bearer realm=\"chainbot\""),
    );
    response
}

fn html_response(status: StatusCode, body: &str) -> Response<Body> {
    let mut response = (status, body.to_owned()).into_response();
    response.headers_mut().insert(
        HeaderName::from_static("content-type"),
        HeaderValue::from_static("text/html"),
    );
    response
}

fn json_response(
    status: StatusCode,
    session_id: Option<&str>,
    body: serde_json::Value,
) -> Response<Body> {
    let mut response = (status, Json(body)).into_response();
    if let Some(session_id) = session_id {
        response.headers_mut().insert(
            HeaderName::from_static("mcp-session-id"),
            HeaderValue::from_str(session_id).expect("fixture session id should be a valid header"),
        );
    }
    response
}

fn create_plugin_root(root: &Path, plugin_id: &str) -> PathBuf {
    let plugin_root = root.join("plugins").join(plugin_id);
    fs::create_dir_all(&plugin_root).expect("plugin root should be creatable");
    plugin_root
}

fn stdio_manifest(
    plugin_id: &str,
    plugin_root: &Path,
    command: &str,
    args: Vec<String>,
) -> PluginManifest {
    PluginManifest {
        api_version: "2.0.0".to_owned(),
        plugin_id: plugin_id.to_owned(),
        kind: PLUGIN_KIND_EXTERNAL_NODE.to_owned(),
        entrypoint: EXTERNAL_NODE_ENTRYPOINT_MCP_TOOL_V1.to_owned(),
        capabilities: vec![NODE_PLUGIN_EXECUTE_CAPABILITY.to_owned()],
        executable: None,
        trigger_runtime: None,
        input_schema: Vec::new(),
        output_schema: Vec::new(),
        operations: vec![PluginOperationDescriptor {
            name: "echo".to_owned(),
            summary: Some("Echo message payloads".to_owned()),
            input_schema: vec!["message".to_owned()],
            output_schema: vec!["message".to_owned()],
        }],
        event_schema: None,
        mcp: Some(McpPluginContract {
            transport: McpTransportKind::Stdio,
            stdio: Some(McpStdioTransportConfig {
                command: command.to_owned(),
                args,
            }),
            streamable_http: None,
            auth: None,
        }),
        manifest_path: plugin_root.join("config.toml"),
    }
}

fn http_manifest(
    plugin_id: &str,
    plugin_root: &Path,
    url: &str,
    auth: Option<(String, String)>,
) -> PluginManifest {
    PluginManifest {
        api_version: "2.0.0".to_owned(),
        plugin_id: plugin_id.to_owned(),
        kind: PLUGIN_KIND_EXTERNAL_NODE.to_owned(),
        entrypoint: EXTERNAL_NODE_ENTRYPOINT_MCP_TOOL_V1.to_owned(),
        capabilities: vec![NODE_PLUGIN_EXECUTE_CAPABILITY.to_owned()],
        executable: None,
        trigger_runtime: None,
        input_schema: Vec::new(),
        output_schema: Vec::new(),
        operations: vec![PluginOperationDescriptor {
            name: "echo".to_owned(),
            summary: Some("Echo message payloads".to_owned()),
            input_schema: vec!["message".to_owned()],
            output_schema: vec!["message".to_owned()],
        }],
        event_schema: None,
        mcp: Some(McpPluginContract {
            transport: McpTransportKind::StreamableHttp,
            stdio: None,
            streamable_http: Some(McpStreamableHttpTransportConfig {
                url: url.to_owned(),
            }),
            auth: auth.map(|(header_name, token_secret_ref)| McpAuthConfig {
                header_name: Some(header_name),
                token_secret_ref: Some(token_secret_ref),
            }),
        }),
        manifest_path: plugin_root.join("config.toml"),
    }
}

fn echo_request(plugin_id: &str, message: &str) -> ExternalNodePluginRequest {
    ExternalNodePluginRequest {
        contract_version: "1.0.0".to_owned(),
        plugin_id: plugin_id.to_owned(),
        node_id: "node-stdio-1".to_owned(),
        operation: "echo".to_owned(),
        requested_capabilities: vec![NODE_PLUGIN_EXECUTE_CAPABILITY.to_owned()],
        input: BTreeMap::from_iter([("message".to_owned(), json!(message))]),
    }
}

fn write_mcp_stdio_fixture(path: &Path) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("stdio MCP fixture parent directory should be creatable");
    }

    fs::write(path, MCP_STDIO_FIXTURE_SCRIPT).expect("stdio MCP fixture script should be writable");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path)
            .expect("stdio MCP fixture metadata should exist")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).expect("stdio MCP fixture should be executable");
    }
}

fn write_plaintext_secret(root: &Path, secret_ref: &str, value: &str) {
    let reference = secret_ref
        .strip_prefix("secret://")
        .expect("test secret refs should start with secret://");
    let (path_part, _) = reference.split_once('#').unwrap_or((reference, ""));
    let secret_path = root.join("secrets").join(path_part).with_extension("gpg");
    if let Some(parent) = secret_path.parent() {
        fs::create_dir_all(parent).expect("plaintext secret parent directory should be creatable");
    }
    fs::write(&secret_path, value).expect("plaintext secret fixture should be writable");
}

fn unique_test_root(prefix: &str) -> PathBuf {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after UNIX_EPOCH")
        .as_nanos();
    workspace_root()
        .join("target")
        .join("test-roots")
        .join(format!("{prefix}-{now}"))
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
