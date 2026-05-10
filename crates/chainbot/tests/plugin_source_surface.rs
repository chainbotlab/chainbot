//! [INPUT]
//! CLI binary invocations and temporary local git repositories that model remote plugin sources.
//!
//! [OUTPUT]
//! Verifies remote plugin source list/show behavior for single- and multi-plugin repositories.
//!
//! [ROLE]
//! Covers the read-only plugin source discoverability surface as an integration boundary.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

fn acquire_fixture_lock() -> MutexGuard<'static, ()> {
    fixture_lock()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

#[test]
fn plugin_source_list_json_reports_multi_plugin_repo() {
    let _lock = acquire_fixture_lock();
    let repo = unique_root("plugin-source-list-repo");
    create_multi_plugin_repo(&repo);

    let output = Command::new(chainbot_bin())
        .args([
            "plugin",
            "source",
            "list",
            "git",
            repo.to_string_lossy().as_ref(),
            "--json",
        ])
        .output()
        .expect("plugin source list should execute");

    assert!(output.status.success());
    let payload: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("plugin source list json should decode");
    assert_eq!(payload["plugins"].as_array().map(Vec::len), Some(2));
    assert_eq!(payload["plugins"][0]["plugin_id"], "alpha-plugin");
    assert_eq!(payload["plugins"][1]["plugin_id"], "beta-plugin");
}

#[test]
fn plugin_source_show_json_requires_and_reports_plugin_details() {
    let _lock = acquire_fixture_lock();
    let repo = unique_root("plugin-source-show-repo");
    create_multi_plugin_repo(&repo);

    let output = Command::new(chainbot_bin())
        .args([
            "plugin",
            "source",
            "show",
            "git",
            repo.to_string_lossy().as_ref(),
            "--plugin",
            "beta-plugin",
            "--json",
        ])
        .output()
        .expect("plugin source show should execute");

    assert!(output.status.success());
    let payload: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("plugin source show json should decode");
    assert_eq!(payload["plugin"]["plugin_id"], "beta-plugin");
    assert_eq!(payload["plugin"]["runtime"], "bin");
    assert_eq!(payload["plugin"]["install_mode"], "direct");
    assert_eq!(payload["plugin"]["entry_artifact"], "bin/plugin.sh");
}

#[test]
fn plugin_source_show_without_plugin_is_a_usage_error_for_multi_plugin_repo() {
    let _lock = acquire_fixture_lock();
    let repo = unique_root("plugin-source-missing-plugin");
    create_multi_plugin_repo(&repo);

    let output = Command::new(chainbot_bin())
        .args([
            "plugin",
            "source",
            "show",
            "git",
            repo.to_string_lossy().as_ref(),
        ])
        .output()
        .expect("plugin source show should execute");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(stderr.contains("requires `--plugin <plugin_id>`"));
    assert!(stderr.contains("alpha-plugin"));
    assert!(stderr.contains("beta-plugin"));
}

#[test]
fn plugin_source_list_json_reports_official_chain_packages() {
    let _lock = acquire_fixture_lock();
    let repo = unique_root("plugin-source-official-chain-repo");
    create_official_chain_repo(&repo);

    let output = Command::new(chainbot_bin())
        .args([
            "plugin",
            "source",
            "list",
            "git",
            repo.to_string_lossy().as_ref(),
            "--json",
        ])
        .output()
        .expect("plugin source list should execute");

    assert!(output.status.success());
    let payload: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("plugin source list json should decode");
    assert_eq!(payload["plugins"].as_array().map(Vec::len), Some(6));
    assert!(payload["plugins"].as_array().is_some_and(|plugins| plugins
        .iter()
        .any(|plugin| plugin["plugin_id"] == "eth-node")));
    assert!(payload["plugins"].as_array().is_some_and(|plugins| plugins
        .iter()
        .any(|plugin| plugin["plugin_id"] == "eth-trigger")));
    assert!(payload["plugins"].as_array().is_some_and(|plugins| plugins
        .iter()
        .any(|plugin| plugin["plugin_id"] == "solana-node")));
    assert!(payload["plugins"].as_array().is_some_and(|plugins| plugins
        .iter()
        .any(|plugin| plugin["plugin_id"] == "solana-trigger")));
    assert!(payload["plugins"].as_array().is_some_and(|plugins| plugins
        .iter()
        .any(|plugin| plugin["plugin_id"] == "hyperliquid-node")));
    assert!(payload["plugins"].as_array().is_some_and(|plugins| plugins
        .iter()
        .any(|plugin| plugin["plugin_id"] == "hyperliquid-trigger")));
}

#[test]
fn plugin_source_show_json_reports_official_trigger_package_details() {
    let _lock = acquire_fixture_lock();
    let repo = unique_root("plugin-source-official-trigger-show");
    create_official_chain_repo(&repo);

    let output = Command::new(chainbot_bin())
        .args([
            "plugin",
            "source",
            "show",
            "git",
            repo.to_string_lossy().as_ref(),
            "--plugin",
            "eth-trigger",
            "--json",
        ])
        .output()
        .expect("plugin source show should execute");

    assert!(output.status.success());
    let payload: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("plugin source show json should decode");
    assert_eq!(payload["plugin"]["plugin_id"], "eth-trigger");
    assert_eq!(payload["plugin"]["plugin_kind"], "external_trigger");
    assert_eq!(payload["plugin"]["runtime"], "bin");
    assert_eq!(
        payload["plugin"]["entry_artifact"],
        "bin/external_trigger.sh"
    );
}

#[test]
fn plugin_source_list_does_not_infer_chain_surface_from_noncanonical_plugin_prefix() {
    let _lock = acquire_fixture_lock();
    let repo = unique_root("plugin-source-noncanonical-chain-prefix");
    let _ = fs::remove_dir_all(&repo);
    fs::create_dir_all(
        repo.join("official-plugins")
            .join("eth-analytics-plugin")
            .join("bin"),
    )
    .expect("plugin directory should be creatable");
    fs::write(
        repo.join("chainbot-plugin-index.toml"),
        "manifest_version = \"1.0.0\"\n\n[[plugins]]\nplugin_id = \"eth-analytics-plugin\"\npath = \"official-plugins/eth-analytics-plugin\"\nsummary = \"Analytics plugin\"\n",
    )
    .expect("source index should be writable");
    write_external_node_plugin(
        repo.join("official-plugins").join("eth-analytics-plugin"),
        "eth-analytics-plugin",
    );
    init_git_repo(&repo);

    let output = Command::new(chainbot_bin())
        .args([
            "plugin",
            "source",
            "list",
            "git",
            repo.to_string_lossy().as_ref(),
            "--json",
        ])
        .output()
        .expect("plugin source list should execute");

    assert!(output.status.success());
    let payload: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("plugin source list json should decode");
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
fn plugin_source_list_requires_embedded_source_metadata() {
    let _lock = acquire_fixture_lock();
    let repo = unique_root("plugin-source-missing-source");
    create_multi_plugin_repo(&repo);
    fs::write(
        repo.join("official-plugins")
            .join("alpha-plugin")
            .join("config.toml"),
        r#"manifest_version = "2.0.0"
plugin_id = "alpha-plugin"
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
    )
    .expect("plugin config without source metadata should be writable");
    run_git(&repo, &["add", "."]);
    run_git(&repo, &["commit", "-m", "remove source metadata"]);

    let output = Command::new(chainbot_bin())
        .args([
            "plugin",
            "source",
            "list",
            "git",
            repo.to_string_lossy().as_ref(),
        ])
        .output()
        .expect("plugin source list should execute");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(stderr.contains("plugin source metadata must be declared in config.toml [source]"));
}

#[test]
fn plugin_source_list_rejects_legacy_source_toml() {
    let _lock = acquire_fixture_lock();
    let repo = unique_root("plugin-source-legacy-source-toml");
    create_multi_plugin_repo(&repo);
    fs::write(
        repo.join("official-plugins")
            .join("alpha-plugin")
            .join("source.toml"),
        r#"manifest_version = "1.0.0"
install_mode = "direct"
runtime = "bin"
entry_artifact = "bin/plugin.sh"
release_version = "0.1.0"
"#,
    )
    .expect("legacy source manifest should be writable");
    run_git(&repo, &["add", "."]);
    run_git(&repo, &["commit", "-m", "add legacy source manifest"]);

    let output = Command::new(chainbot_bin())
        .args([
            "plugin",
            "source",
            "list",
            "git",
            repo.to_string_lossy().as_ref(),
        ])
        .output()
        .expect("plugin source list should execute");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(stderr.contains("legacy source.toml is no longer supported"));
}

#[test]
fn plugin_source_list_rejects_raw_server_reference_in_source_entry_artifact() {
    let _lock = acquire_fixture_lock();
    let repo = unique_root("plugin-source-raw-server-reference");
    create_mcp_http_repo_with_raw_source_entry_artifact(&repo);

    let output = Command::new(chainbot_bin())
        .args([
            "plugin",
            "source",
            "list",
            "git",
            repo.to_string_lossy().as_ref(),
        ])
        .output()
        .expect("plugin source list should execute");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(stderr.contains(
        "source.entry_artifact must reference a package-relative artifact path; raw server URLs are not allowed"
    ));
}

#[test]
fn plugin_source_list_rejects_mcp_http_missing_anchor_file() {
    let _lock = acquire_fixture_lock();
    let repo = unique_root("plugin-source-mcp-http-missing-anchor");
    create_mcp_http_repo_with_missing_anchor(&repo);

    let output = Command::new(chainbot_bin())
        .args([
            "plugin",
            "source",
            "list",
            "git",
            repo.to_string_lossy().as_ref(),
        ])
        .output()
        .expect("plugin source list should execute");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(stderr.contains("entry artifact missing"));
}

fn create_multi_plugin_repo(root: &Path) {
    let _ = fs::remove_dir_all(root);
    fs::create_dir_all(
        root.join("official-plugins")
            .join("alpha-plugin")
            .join("bin"),
    )
    .expect("alpha plugin directory should be creatable");
    fs::create_dir_all(
        root.join("official-plugins")
            .join("beta-plugin")
            .join("bin"),
    )
    .expect("beta plugin directory should be creatable");
    fs::write(
        root.join("chainbot-plugin-index.toml"),
        "manifest_version = \"1.0.0\"\n\n[[plugins]]\nplugin_id = \"alpha-plugin\"\npath = \"official-plugins/alpha-plugin\"\nsummary = \"Alpha plugin\"\n\n[[plugins]]\nplugin_id = \"beta-plugin\"\npath = \"official-plugins/beta-plugin\"\nsummary = \"Beta plugin\"\n",
    )
    .expect("source index should be writable");
    write_external_node_plugin(
        root.join("official-plugins").join("alpha-plugin"),
        "alpha-plugin",
    );
    write_external_node_plugin(
        root.join("official-plugins").join("beta-plugin"),
        "beta-plugin",
    );
    init_git_repo(root);
}

fn create_official_chain_repo(root: &Path) {
    let _ = fs::remove_dir_all(root);
    for plugin_id in [
        "eth-node",
        "eth-trigger",
        "solana-node",
        "solana-trigger",
        "hyperliquid-node",
        "hyperliquid-trigger",
    ] {
        fs::create_dir_all(root.join("official-plugins").join(plugin_id).join("bin"))
            .expect("official plugin directory should be creatable");
    }
    fs::write(
        root.join("chainbot-plugin-index.toml"),
        "manifest_version = \"1.0.0\"\n\n[[plugins]]\nplugin_id = \"eth-node\"\npath = \"official-plugins/eth-node\"\nsummary = \"Official Ethereum node toolkit plugin\"\n\n[[plugins]]\nplugin_id = \"eth-trigger\"\npath = \"official-plugins/eth-trigger\"\nsummary = \"Official Ethereum trigger toolkit plugin\"\n\n[[plugins]]\nplugin_id = \"solana-node\"\npath = \"official-plugins/solana-node\"\nsummary = \"Official Solana node toolkit plugin\"\n\n[[plugins]]\nplugin_id = \"solana-trigger\"\npath = \"official-plugins/solana-trigger\"\nsummary = \"Official Solana trigger toolkit plugin\"\n\n[[plugins]]\nplugin_id = \"hyperliquid-node\"\npath = \"official-plugins/hyperliquid-node\"\nsummary = \"Official Hyperliquid info API node plugin\"\n\n[[plugins]]\nplugin_id = \"hyperliquid-trigger\"\npath = \"official-plugins/hyperliquid-trigger\"\nsummary = \"Official Hyperliquid market-stream trigger plugin\"\n",
    )
    .expect("source index should be writable");
    write_official_chain_node_plugin(
        root.join("official-plugins").join("eth-node"),
        "eth-node",
        "eth_raw_read",
        "eth_raw_write",
        "eth_transfer_native",
    );
    write_official_chain_trigger_plugin(
        root.join("official-plugins").join("eth-trigger"),
        "eth-trigger",
        "Ethereum trigger payload",
        &["chain", "listener_kind", "block_ref", "event_id", "payload"],
    );
    write_official_chain_node_plugin(
        root.join("official-plugins").join("solana-node"),
        "solana-node",
        "solana_raw_read",
        "solana_raw_write",
        "solana_transfer_native",
    );
    write_official_chain_trigger_plugin(
        root.join("official-plugins").join("solana-trigger"),
        "solana-trigger",
        "Solana trigger payload",
        &["chain", "listener_kind", "slot_ref", "event_id", "payload"],
    );
    write_official_market_data_node_plugin(
        root.join("official-plugins").join("hyperliquid-node"),
        "hyperliquid-node",
        &[
            ("hyperliquid_get_all_mids", &[], &["dex", "base_url"], "all_mids"),
            (
                "hyperliquid_get_l2_book",
                &["coin"],
                &["nSigFigs", "mantissa", "base_url"],
                "l2_book",
            ),
            (
                "hyperliquid_get_candle_snapshot",
                &["coin", "interval", "startTime", "endTime"],
                &["base_url"],
                "candles",
            ),
        ],
    );
    write_official_market_trigger_plugin(
        root.join("official-plugins").join("hyperliquid-trigger"),
        "hyperliquid-trigger",
        "Hyperliquid market listener payload",
        &["exchange", "listener_kind", "channel", "coin", "event_id", "payload"],
    );
    init_git_repo(root);
}

fn write_external_node_plugin(root: PathBuf, plugin_id: &str) {
    fs::write(
        root.join("config.toml"),
        format!(
            "manifest_version = \"2.0.0\"\nplugin_id = \"{plugin_id}\"\nkind = \"external_node\"\nentrypoint = \"node.exec.v1\"\ncapabilities = [\"node:execute\"]\nexecutable = \"bin/plugin.sh\"\n\n[[operations]]\nname = \"normalize\"\nsummary = \"Normalize payload\"\ninput_schema = [\"symbol\"]\noutput_schema = [\"decision\"]\n\n[source]\nmanifest_version = \"1.0.0\"\ninstall_mode = \"direct\"\nruntime = \"bin\"\nentry_artifact = \"bin/plugin.sh\"\nrelease_version = \"0.1.0\"\n"
        ),
    )
    .expect("plugin config should be writable");
    fs::write(root.join("bin").join("plugin.sh"), "#!/bin/sh\necho '{}'\n")
        .expect("plugin executable should be writable");
    set_executable(&root.join("bin").join("plugin.sh"));
}

fn write_official_chain_node_plugin(
    root: PathBuf,
    plugin_id: &str,
    raw_read_operation: &str,
    raw_write_operation: &str,
    transfer_operation: &str,
) {
    fs::write(
        root.join("config.toml"),
        format!(
            "manifest_version = \"2.0.0\"\nplugin_id = \"{plugin_id}\"\nkind = \"external_node\"\nentrypoint = \"node.exec.v1\"\ncapabilities = [\"node:execute\"]\nexecutable = \"bin/plugin.sh\"\n\n[activation]\nrequired_secret_slots = [\"signer\"]\n\n[[operations]]\nname = \"{raw_read_operation}\"\nsummary = \"Raw read\"\ninput_schema = [\"endpoint\", \"method\", \"params\"]\noutput_schema = [\"result\"]\nkind = \"raw_read\"\n\n[[operations]]\nname = \"{raw_write_operation}\"\nsummary = \"Raw write\"\ninput_schema = [\"endpoint\", \"method\", \"params\", \"confirmation_mode\"]\noutput_schema = [\"status\", \"transaction_id\"]\nkind = \"raw_write\"\nrequires_managed_signing = true\ndefault_confirmation = \"safe\"\n\n[[operations]]\nname = \"{transfer_operation}\"\nsummary = \"Native transfer\"\ninput_schema = [\"endpoint\", \"from\", \"to\", \"amount\", \"confirmation_mode\"]\noutput_schema = [\"status\", \"transaction_id\"]\nkind = \"transfer\"\nrequires_managed_signing = true\ndefault_confirmation = \"safe\"\n\n[source]\nmanifest_version = \"1.0.0\"\ninstall_mode = \"direct\"\nruntime = \"bin\"\nentry_artifact = \"bin/plugin.sh\"\nrelease_version = \"0.1.0\"\n"
        ),
    )
    .expect("plugin config should be writable");
    fs::write(root.join("bin").join("plugin.sh"), "#!/bin/sh\necho '{}'\n")
        .expect("plugin executable should be writable");
    set_executable(&root.join("bin").join("plugin.sh"));
}

fn write_official_chain_trigger_plugin(
    root: PathBuf,
    plugin_id: &str,
    summary: &str,
    fields: &[&str],
) {
    let quoted_fields = fields
        .iter()
        .map(|field| format!("\"{field}\""))
        .collect::<Vec<_>>()
        .join(", ");
    fs::write(
        root.join("config.toml"),
        format!(
            "manifest_version = \"2.0.0\"\nplugin_id = \"{plugin_id}\"\nkind = \"external_trigger\"\nentrypoint = \"trigger.exec.v1\"\ncapabilities = [\"trigger.listen.event\"]\nexecutable = \"bin/external_trigger.sh\"\n\n[activation]\noptional_secret_slots = [\"rpc_token\"]\nrequires_allowed_origins = true\n\n[trigger_runtime]\nlifecycle = \"process_short_lived\"\npush_callback = \"inline_response\"\ndurable_ack = \"caller_scope\"\nhost_error_categories = [\"transport\", \"protocol_contract\", \"plugin_fatal\"]\n\n[event_schema]\nsummary = \"{summary}\"\nfields = [{quoted_fields}]\nlistener_modes = [\"event_log\", \"state_change\"]\n\n[source]\nmanifest_version = \"1.0.0\"\ninstall_mode = \"direct\"\nruntime = \"bin\"\nentry_artifact = \"bin/external_trigger.sh\"\nrelease_version = \"0.1.0\"\n"
        ),
    )
    .expect("trigger config should be writable");
    fs::write(
        root.join("bin").join("external_trigger.sh"),
        "#!/bin/sh\nexit 0\n",
    )
    .expect("trigger executable should be writable");
    set_executable(&root.join("bin").join("external_trigger.sh"));
}

fn write_official_market_data_node_plugin(
    root: PathBuf,
    plugin_id: &str,
    operations: &[(&str, &[&str], &[&str], &str)],
) {
    let operations_block = operations
        .iter()
        .map(|(name, required_inputs, optional_inputs, output)| {
            let required = required_inputs
                .iter()
                .map(|field| format!("\"{field}\""))
                .collect::<Vec<_>>()
                .join(", ");
            let optional = optional_inputs
                .iter()
                .map(|field| format!("\"{field}\""))
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "[[operations]]\nname = \"{name}\"\nsummary = \"{name}\"\ninput_schema = [{required}]\noptional_input_schema = [{optional}]\noutput_schema = [\"{output}\"]\nkind = \"read\"\n"
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(
        root.join("config.toml"),
        format!(
            "manifest_version = \"2.0.0\"\nplugin_id = \"{plugin_id}\"\nkind = \"external_node\"\nentrypoint = \"node.exec.v2\"\ncapabilities = [\"node:execute\"]\nexecutable = \"bin/hyperliquid-node\"\n\n[activation]\noptional_secret_slots = [\"origin_binding\"]\nrequires_allowed_origins = true\n\n{operations_block}\n\n[source]\nmanifest_version = \"1.0.0\"\ninstall_mode = \"direct\"\nruntime = \"bin\"\nentry_artifact = \"bin/hyperliquid-node\"\nrelease_version = \"0.1.0\"\n"
        ),
    )
    .expect("plugin config should be writable");
    fs::write(root.join("bin").join("hyperliquid-node"), "#!/bin/sh\necho '{}'\n")
        .expect("plugin executable should be writable");
    set_executable(&root.join("bin").join("hyperliquid-node"));
}

fn write_official_market_trigger_plugin(
    root: PathBuf,
    plugin_id: &str,
    summary: &str,
    fields: &[&str],
) {
    let quoted_fields = fields
        .iter()
        .map(|field| format!("\"{field}\""))
        .collect::<Vec<_>>()
        .join(", ");
    fs::write(
        root.join("config.toml"),
        format!(
            "manifest_version = \"2.0.0\"\nplugin_id = \"{plugin_id}\"\nkind = \"external_trigger\"\nentrypoint = \"trigger.exec.v1\"\ncapabilities = [\"trigger.listen.event\"]\nexecutable = \"bin/hyperliquid-trigger\"\n\n[activation]\noptional_secret_slots = [\"origin_binding\"]\nrequires_allowed_origins = true\n\n[trigger_runtime]\nlifecycle = \"process_short_lived\"\npush_callback = \"inline_response\"\ndurable_ack = \"caller_scope\"\nhost_error_categories = [\"transport\", \"protocol_contract\", \"plugin_fatal\"]\n\n[event_schema]\nsummary = \"{summary}\"\nfields = [{quoted_fields}]\nlistener_modes = [\"event_log\", \"state_change\"]\n\n[source]\nmanifest_version = \"1.0.0\"\ninstall_mode = \"direct\"\nruntime = \"bin\"\nentry_artifact = \"bin/hyperliquid-trigger\"\nrelease_version = \"0.1.0\"\n"
        ),
    )
    .expect("trigger config should be writable");
    fs::write(root.join("bin").join("hyperliquid-trigger"), "#!/bin/sh\nexit 0\n")
        .expect("trigger executable should be writable");
    set_executable(&root.join("bin").join("hyperliquid-trigger"));
}

fn create_mcp_http_repo_with_raw_source_entry_artifact(root: &Path) {
    let _ = fs::remove_dir_all(root);
    fs::create_dir_all(root).expect("repo root should be creatable");
    fs::write(
        root.join("config.toml"),
        "manifest_version = \"2.0.0\"\nplugin_id = \"mcp-http-raw-source-plugin\"\nkind = \"external_node\"\nentrypoint = \"mcp.tool.v1\"\ncapabilities = [\"node:execute\"]\n\n[[operations]]\nname = \"echo\"\nsummary = \"Echo tool\"\ninput_schema = [\"message\"]\noutput_schema = [\"message\"]\n\n[mcp]\ntransport = \"streamable_http\"\n\n[mcp.streamable_http]\nurl = \"https://example.test/mcp\"\n\n[source]\nmanifest_version = \"1.0.0\"\ninstall_mode = \"direct\"\nruntime = \"bin\"\nentry_artifact = \"https://example.test/direct-server\"\nrelease_version = \"0.1.0\"\n",
    )
    .expect("mcp plugin config should be writable");
    init_git_repo(root);
}

fn create_mcp_http_repo_with_missing_anchor(root: &Path) {
    let _ = fs::remove_dir_all(root);
    fs::create_dir_all(root).expect("repo root should be creatable");
    fs::write(
        root.join("config.toml"),
        "manifest_version = \"2.0.0\"\nplugin_id = \"mcp-http-missing-anchor\"\nkind = \"external_node\"\nentrypoint = \"mcp.tool.v1\"\ncapabilities = [\"node:execute\"]\n\n[[operations]]\nname = \"echo\"\nsummary = \"Echo tool\"\ninput_schema = [\"message\"]\noutput_schema = [\"message\"]\n\n[mcp]\ntransport = \"streamable_http\"\n\n[mcp.streamable_http]\nurl = \"https://example.test/mcp\"\n\n[source]\nmanifest_version = \"1.0.0\"\ninstall_mode = \"direct\"\nruntime = \"bin\"\nentry_artifact = \"artifacts/anchor.txt\"\nrelease_version = \"0.1.0\"\n",
    )
    .expect("mcp plugin config should be writable");
    init_git_repo(root);
}

fn init_git_repo(root: &Path) {
    run_git(root, &["init"]);
    run_git(root, &["config", "user.email", "tests@example.com"]);
    run_git(root, &["config", "user.name", "ChainBot Tests"]);
    run_git(root, &["add", "."]);
    run_git(root, &["commit", "-m", "fixture"]);
}

fn run_git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .expect("git command should execute");
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
}

fn set_executable(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path)
            .expect("metadata should load")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).expect("permissions should update");
    }
}

fn chainbot_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_chainbot"))
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

fn fixture_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}
