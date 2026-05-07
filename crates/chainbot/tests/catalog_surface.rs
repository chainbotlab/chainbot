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
    assert_eq!(payload["plugins"].as_array().map(Vec::len), Some(3));
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
fn catalog_list_reports_installed_mcp_plugins_under_installed_plugins() {
    let _lock = acquire_fixture_lock();
    let root = unique_root("catalog-installed-mcp-plugins");
    let _ = fs::remove_dir_all(&root);
    write_catalog_root(
        &root,
        &[
            (
                "mcp-stdio-plugin",
                r#"manifest_version = "2.0.0"
plugin_id = "mcp-stdio-plugin"
kind = "external_node"
entrypoint = "mcp.tool.v1"
capabilities = ["node:execute"]

[[operations]]
name = "echo"
summary = "Echo tool"
input_schema = ["message"]
output_schema = ["message"]

[mcp]
transport = "stdio"

[mcp.stdio]
command = "bin/mcp_stdio_fixture.py"
args = ["--mode", "echo"]
"#,
            ),
            (
                "mcp-http-plugin",
                r#"manifest_version = "2.0.0"
plugin_id = "mcp-http-plugin"
kind = "external_node"
entrypoint = "mcp.tool.v1"
capabilities = ["node:execute"]

[[operations]]
name = "echo"
summary = "Echo tool"
input_schema = ["message"]
output_schema = ["message"]

[mcp]
transport = "streamable_http"

[mcp.streamable_http]
url = "https://example.test/mcp"
"#,
            ),
        ],
    );

    let text_output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", &root)
        .args(["catalog", "list", "--kind", "plugin"])
        .output()
        .expect("catalog list with installed MCP plugins should execute");

    assert!(text_output.status.success());
    let stdout = String::from_utf8(text_output.stdout).expect("stdout should be UTF-8");
    assert!(stdout.contains("Installed plugins"));
    assert!(stdout.contains("plugin:mcp-stdio-plugin"));
    assert!(stdout.contains("plugin:mcp-http-plugin"));
    assert!(stdout.contains("transport=stdio runtime=per_invocation_stdio_session"));
    assert!(
        stdout.contains("transport=streamable_http runtime=per_invocation_streamable_http_session")
    );
    assert!(!stdout.contains("MCP plugins"));

    let json_output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", &root)
        .args(["catalog", "list", "--kind", "plugin", "--json"])
        .output()
        .expect("catalog list json with installed MCP plugins should execute");

    assert!(json_output.status.success());
    let payload = serde_json::from_slice::<serde_json::Value>(&json_output.stdout)
        .expect("catalog list json should decode");
    assert_eq!(payload["plugins"].as_array().map(Vec::len), Some(2));
    assert_eq!(payload["plugins"][0]["kind"], "external_node");
    assert!(payload["plugins"]
        .as_array()
        .is_some_and(|plugins| plugins
            .iter()
            .any(|plugin| plugin["plugin_id"] == "mcp-stdio-plugin"
                && plugin["transport"] == "stdio"
                && plugin["runtime"] == "per_invocation_stdio_session")));
    assert!(payload["plugins"]
        .as_array()
        .is_some_and(|plugins| plugins
            .iter()
            .any(|plugin| plugin["plugin_id"] == "mcp-http-plugin"
                && plugin["transport"] == "streamable_http"
                && plugin["runtime"] == "per_invocation_streamable_http_session")));
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
fn catalog_show_reports_mcp_transport_runtime_in_json_and_text() {
    let _lock = acquire_fixture_lock();
    let root = unique_root("catalog-show-mcp-runtime");
    let _ = fs::remove_dir_all(&root);
    write_catalog_root(
        &root,
        &[
            (
                "mcp-stdio-plugin",
                r#"manifest_version = "2.0.0"
plugin_id = "mcp-stdio-plugin"
kind = "external_node"
entrypoint = "mcp.tool.v1"
capabilities = ["node:execute"]

[[operations]]
name = "echo"
summary = "Echo tool"
input_schema = ["message"]
output_schema = ["message"]

[mcp]
transport = "stdio"

[mcp.stdio]
command = "bin/mcp_stdio_fixture.py"
"#,
            ),
            (
                "mcp-http-plugin",
                r#"manifest_version = "2.0.0"
plugin_id = "mcp-http-plugin"
kind = "external_node"
entrypoint = "mcp.tool.v1"
capabilities = ["node:execute"]

[[operations]]
name = "echo"
summary = "Echo tool"
input_schema = ["message"]
output_schema = ["message"]

[mcp]
transport = "streamable_http"

[mcp.streamable_http]
url = "https://example.test/mcp"
"#,
            ),
        ],
    );

    let stdio_json_output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", &root)
        .args(["catalog", "show", "plugin:mcp-stdio-plugin", "--json"])
        .output()
        .expect("catalog show stdio MCP plugin should execute");

    assert!(stdio_json_output.status.success());
    let stdio_payload = serde_json::from_slice::<serde_json::Value>(&stdio_json_output.stdout)
        .expect("catalog show stdio MCP plugin json should decode");
    assert_eq!(stdio_payload["detail"]["plugin_kind"], "external_node");
    assert_eq!(stdio_payload["detail"]["transport"], "stdio");
    assert_eq!(
        stdio_payload["detail"]["runtime"],
        "per_invocation_stdio_session"
    );

    let stdio_text_output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", &root)
        .args(["catalog", "show", "plugin:mcp-stdio-plugin"])
        .output()
        .expect("catalog show stdio MCP plugin text should execute");

    assert!(stdio_text_output.status.success());
    let stdio_stdout = String::from_utf8(stdio_text_output.stdout).expect("stdout should be UTF-8");
    assert!(stdio_stdout.contains("transport: stdio"));
    assert!(stdio_stdout.contains("runtime: per_invocation_stdio_session"));
    assert!(stdio_stdout.contains("starts a stdio client session for each invocation"));

    let http_json_output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", &root)
        .args(["catalog", "show", "plugin:mcp-http-plugin", "--json"])
        .output()
        .expect("catalog show streamable HTTP MCP plugin should execute");

    assert!(http_json_output.status.success());
    let http_payload = serde_json::from_slice::<serde_json::Value>(&http_json_output.stdout)
        .expect("catalog show streamable HTTP MCP plugin json should decode");
    assert_eq!(http_payload["detail"]["plugin_kind"], "external_node");
    assert_eq!(http_payload["detail"]["transport"], "streamable_http");
    assert_eq!(
        http_payload["detail"]["runtime"],
        "per_invocation_streamable_http_session"
    );

    let http_text_output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", &root)
        .args(["catalog", "show", "plugin:mcp-http-plugin"])
        .output()
        .expect("catalog show streamable HTTP MCP plugin text should execute");

    assert!(http_text_output.status.success());
    let http_stdout = String::from_utf8(http_text_output.stdout).expect("stdout should be UTF-8");
    assert!(http_stdout.contains("transport: streamable_http"));
    assert!(http_stdout.contains("runtime: per_invocation_streamable_http_session"));
    assert!(http_stdout.contains("opens a Streamable HTTP client session for each invocation"));
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
fn catalog_list_reports_installed_official_chain_node_and_trigger_plugins() {
    let _lock = acquire_fixture_lock();
    let root = unique_root("catalog-official-chain-plugins");
    let _ = fs::remove_dir_all(&root);
    write_catalog_root(
        &root,
        &[
            (
                "eth-node",
                r#"manifest_version = "2.0.0"
plugin_id = "eth-node"
kind = "external_node"
entrypoint = "node.exec.v2"
capabilities = ["node:execute"]
executable = "bin/plugin.sh"

[[operations]]
name = "eth_raw_read"
summary = "Raw read"
input_schema = ["endpoint", "method", "params"]
output_schema = ["result"]
kind = "raw_read"

[[operations]]
name = "eth_raw_write"
summary = "Raw write"
input_schema = ["endpoint", "method", "params", "signer_ref", "confirmation_mode"]
output_schema = ["status", "transaction_hash"]
kind = "raw_write"
requires_managed_signing = true
default_confirmation = "safe"
"#,
            ),
            (
                "solana-trigger",
                r#"manifest_version = "2.0.0"
plugin_id = "solana-trigger"
kind = "external_trigger"
entrypoint = "trigger.exec.v1"
capabilities = ["trigger.listen.event"]
executable = "bin/external_trigger.sh"

[trigger_runtime]
lifecycle = "process_short_lived"
push_callback = "inline_response"
durable_ack = "caller_scope"
host_error_categories = ["transport", "protocol_contract", "plugin_fatal"]

[event_schema]
summary = "Solana trigger payload"
fields = ["chain", "listener_kind", "slot_ref", "event_id", "payload"]
listener_modes = ["event_log", "state_change"]
"#,
            ),
        ],
    );

    let list_output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", &root)
        .args(["catalog", "list", "--kind", "plugin", "--json"])
        .output()
        .expect("catalog list should execute");

    assert!(list_output.status.success());
    let payload = serde_json::from_slice::<serde_json::Value>(&list_output.stdout)
        .expect("catalog list json should decode");
    assert_eq!(payload["plugins"].as_array().map(Vec::len), Some(2));
    assert!(payload["plugins"].as_array().is_some_and(|plugins| plugins
        .iter()
        .any(|plugin| plugin["plugin_id"] == "eth-node")));
    assert!(payload["plugins"].as_array().is_some_and(|plugins| plugins
        .iter()
        .any(|plugin| plugin["plugin_id"] == "solana-trigger")));

    let show_output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", &root)
        .args(["catalog", "show", "plugin:eth-node", "--json"])
        .output()
        .expect("catalog show should execute");

    assert!(show_output.status.success());
    let show_payload = serde_json::from_slice::<serde_json::Value>(&show_output.stdout)
        .expect("catalog show json should decode");
    assert_eq!(show_payload["detail"]["plugin_kind"], "external_node");
    assert_eq!(
        show_payload["detail"]["operations"][0]["name"],
        "eth_raw_read"
    );
    assert_eq!(show_payload["detail"]["operations"][0]["kind"], "raw_read");
    assert_eq!(show_payload["detail"]["operations"][1]["kind"], "raw_write");
    assert_eq!(
        show_payload["detail"]["operations"][1]["requires_managed_signing"],
        true
    );
    assert_eq!(
        show_payload["detail"]["operations"][1]["default_confirmation"],
        "safe"
    );

    let trigger_show_output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", &root)
        .args(["catalog", "show", "plugin:solana-trigger", "--json"])
        .output()
        .expect("catalog show trigger plugin should execute");

    assert!(trigger_show_output.status.success());
    let trigger_payload = serde_json::from_slice::<serde_json::Value>(&trigger_show_output.stdout)
        .expect("catalog show trigger plugin json should decode");
    assert_eq!(trigger_payload["detail"]["plugin_kind"], "external_trigger");
    assert_eq!(
        trigger_payload["detail"]["event_schema"]["listener_modes"][0],
        "event_log"
    );
    assert_eq!(
        trigger_payload["detail"]["event_schema"]["listener_modes"][1],
        "state_change"
    );
}

#[test]
fn catalog_list_does_not_infer_chain_surface_from_noncanonical_plugin_prefix() {
    let _lock = acquire_fixture_lock();
    let root = unique_root("catalog-noncanonical-chain-prefix");
    let _ = fs::remove_dir_all(&root);
    write_catalog_root(
        &root,
        &[(
            "eth-analytics-plugin",
            r#"manifest_version = "2.0.0"
plugin_id = "eth-analytics-plugin"
kind = "external_node"
entrypoint = "node.exec.v2"
capabilities = ["node:execute"]
executable = "bin/plugin.sh"

[[operations]]
name = "normalize"
summary = "Normalize payload"
input_schema = ["symbol"]
output_schema = ["decision"]
"#,
        )],
    );

    let output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", &root)
        .args(["catalog", "list", "--kind", "plugin", "--json"])
        .output()
        .expect("catalog list should execute");

    assert!(output.status.success());
    let payload = serde_json::from_slice::<serde_json::Value>(&output.stdout)
        .expect("catalog list json should decode");
    let plugin = payload["plugins"]
        .as_array()
        .and_then(|plugins| {
            plugins
                .iter()
                .find(|plugin| plugin["plugin_id"] == "eth-analytics-plugin")
        })
        .expect("noncanonical plugin should be present");
    assert_eq!(
        plugin["surfaces"],
        serde_json::json!(["operations=normalize"])
    );
}

#[test]
fn catalog_show_reports_wasm_trigger_lifecycle_in_json_and_text() {
    let _lock = acquire_fixture_lock();
    let root = unique_root("catalog-wasm-trigger-lifecycle");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("plugins").join("wasm-trigger-plugin"))
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
        root.join("plugins")
            .join("wasm-trigger-plugin")
            .join("config.toml"),
        r#"manifest_version = "2.0.0"
plugin_id = "wasm-trigger-plugin"
kind = "external_trigger"
entrypoint = "trigger.exec.v1"
capabilities = ["trigger.listen.event"]

[trigger_runtime]
lifecycle = "wasm_daemon_persistent_session"
push_callback = "host_callback"
durable_ack = "after_store_persist"
host_error_categories = ["transport", "protocol_contract", "plugin_fatal"]
module = "bin/trigger.wasm"

[event_schema]
summary = "Wasm trigger payload"
fields = ["symbol", "price"]
"#,
    )
    .expect("wasm trigger plugin config should be writable");

    let json_output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", &root)
        .args(["catalog", "show", "plugin:wasm-trigger-plugin", "--json"])
        .output()
        .expect("catalog show wasm trigger plugin should execute");

    assert!(json_output.status.success());
    let payload = serde_json::from_slice::<serde_json::Value>(&json_output.stdout)
        .expect("wasm trigger catalog json should decode");
    assert_eq!(payload["detail"]["plugin_kind"], "external_trigger");
    assert_eq!(
        payload["detail"]["lifecycle"],
        "wasm_daemon_persistent_session"
    );

    let text_output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", &root)
        .args(["catalog", "show", "plugin:wasm-trigger-plugin"])
        .output()
        .expect("catalog show wasm trigger plugin text should execute");

    assert!(text_output.status.success());
    let stdout = String::from_utf8(text_output.stdout).expect("stdout should be UTF-8");
    assert!(stdout.contains("lifecycle: wasm_daemon_persistent_session"));
    assert!(stdout.contains("Daemon-persistent wasm session"));
}

#[test]
fn catalog_show_reports_process_trigger_lifecycle_in_json_and_text() {
    let _lock = acquire_fixture_lock();
    let root = unique_root("catalog-process-trigger-lifecycle");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("plugins").join("process-trigger-plugin"))
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
        root.join("plugins")
            .join("process-trigger-plugin")
            .join("config.toml"),
        r#"manifest_version = "2.0.0"
plugin_id = "process-trigger-plugin"
kind = "external_trigger"
entrypoint = "trigger.exec.v1"
capabilities = ["trigger.listen.event"]
executable = "bin/external_trigger.sh"

[trigger_runtime]
lifecycle = "process_short_lived"
push_callback = "inline_response"
durable_ack = "caller_scope"
host_error_categories = ["transport", "protocol_contract", "plugin_fatal"]

[event_schema]
summary = "Process trigger payload"
fields = ["symbol", "price"]
"#,
    )
    .expect("process trigger plugin config should be writable");

    let json_output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", &root)
        .args(["catalog", "show", "plugin:process-trigger-plugin", "--json"])
        .output()
        .expect("catalog show process trigger plugin should execute");

    assert!(json_output.status.success());
    let payload = serde_json::from_slice::<serde_json::Value>(&json_output.stdout)
        .expect("process trigger catalog json should decode");
    assert_eq!(payload["detail"]["plugin_kind"], "external_trigger");
    assert_eq!(payload["detail"]["lifecycle"], "process_short_lived");

    let text_output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", &root)
        .args(["catalog", "show", "plugin:process-trigger-plugin"])
        .output()
        .expect("catalog show process trigger plugin text should execute");

    assert!(text_output.status.success());
    let stdout = String::from_utf8(text_output.stdout).expect("stdout should be UTF-8");
    assert!(stdout.contains("lifecycle: process_short_lived"));
    assert!(stdout.contains("Short-lived process adapter"));
}

#[test]
fn catalog_show_rejects_legacy_external_trigger_without_runtime_lifecycle() {
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
        "manifest_version = \"2.0.0\"\nplugin_id = \"legacy-trigger\"\nkind = \"external_trigger\"\nentrypoint = \"trigger.exec.v1\"\ncapabilities = [\"trigger.listen.event\"]\nexecutable = \"bin/external_trigger.sh\"\n\n[event_schema]\nsummary = \"Legacy trigger payload\"\nfields = [\"symbol\", \"price\"]\n",
    )
    .expect("legacy plugin config should be writable");

    let output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", &root)
        .args(["catalog", "show", "plugin:legacy-trigger", "--json"])
        .output()
        .expect("catalog show legacy plugin should execute");

    assert_eq!(output.status.code(), Some(3));
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(stderr.contains("plugin.trigger_runtime.lifecycle"));
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

fn write_catalog_root(root: &Path, plugins: &[(&str, &str)]) {
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

    for (plugin_id, manifest) in plugins {
        let plugin_dir = root.join("plugins").join(plugin_id);
        fs::create_dir_all(&plugin_dir).expect("plugin package directory should be creatable");
        fs::write(plugin_dir.join("config.toml"), manifest)
            .expect("plugin config should be writable");
    }
}
