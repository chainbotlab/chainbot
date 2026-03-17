/*
[INPUT]:  External node plugin manifests, host root fixtures, and executable plugin scripts.
[OUTPUT]: Deterministic integration coverage for node plugin manifest validation, roundtrip execution, and capability restrictions.
[POS]:    Integration test boundary for external node plugin host contracts.
[UPDATE]: 2026-03-16 - Add external node plugin host validation and execution contract tests.
[UPDATE]: 2026-03-17 - Add regression coverage for default-deny plugin host environment isolation.
*/

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use chainbot::errors::ContractError;
use chainbot::plugin::{
    ExternalNodePluginHost, ExternalNodePluginRequest, PluginManifest, PLUGIN_HOST_ENV_ALLOWLIST,
    PLUGIN_KIND_EXTERNAL_NODE,
};
use serde_json::json;

#[test]
fn node_plugin_manifest_validation() {
    let root = unique_test_root("node-plugin-manifest");
    let plugins_root = root.join("plugins");
    let executable = plugins_root.join("bin").join("node_ok.sh");
    write_plugin_script(
        &executable,
        "{\"contract_version\":\"1.0.0\",\"success\":true,\"output\":{\"decision\":\"buy\"}}",
        None,
    );

    let manifest = external_node_manifest("node-quote", "bin/node_ok.sh");
    manifest
        .validate()
        .expect("valid external node plugin manifest should pass");

    let host = ExternalNodePluginHost::new(plugins_root.clone());
    let request = valid_request("node-quote", "node-1");
    let result = host
        .execute(&manifest, &request)
        .expect("manifest and executable contract should be valid");
    assert_eq!(result.output.get("decision"), Some(&json!("buy")));

    let mut escaped_manifest = manifest.clone();
    escaped_manifest.executable = Some("../escape.sh".to_owned());
    let escaped_error = host
        .execute(&escaped_manifest, &request)
        .expect_err("executable path escaping plugin root must be rejected");
    assert!(matches!(
        escaped_error,
        ContractError::NodePluginInvalidExecutablePath {
            plugin_id,
            executable,
            ..
        } if plugin_id == "node-quote" && executable == "../escape.sh"
    ));
}

#[test]
fn external_node_plugin_roundtrip() {
    let root = unique_test_root("node-plugin-roundtrip");
    let plugins_root = root.join("plugins");
    let executable = plugins_root.join("bin").join("node_roundtrip.sh");
    write_plugin_script(
        &executable,
        "{\"contract_version\":\"1.0.0\",\"success\":true,\"output\":{\"decision\":\"sell\"}}",
        None,
    );

    let host = ExternalNodePluginHost::new(plugins_root);
    let manifest = external_node_manifest("node-roundtrip", "bin/node_roundtrip.sh");
    let mut request = valid_request("node-roundtrip", "node-2");
    request.input.insert("symbol".to_owned(), json!("ETHUSDT"));

    let result = host
        .execute(&manifest, &request)
        .expect("external node plugin should execute with a stable stdin/stdout contract");
    assert_eq!(result.output.get("decision"), Some(&json!("sell")));
}

#[test]
fn node_plugin_capability_restrictions() {
    let root = unique_test_root("node-plugin-capability");
    let plugins_root = root.join("plugins");
    let marker_path = root.join("spawned.marker");
    let executable = plugins_root.join("bin").join("node_restricted.sh");
    write_plugin_script(
        &executable,
        "{\"contract_version\":\"1.0.0\",\"success\":true,\"output\":{\"decision\":\"hold\"}}",
        Some(&marker_path),
    );

    let host = ExternalNodePluginHost::new(plugins_root);
    let manifest = external_node_manifest("node-restricted", "bin/node_restricted.sh");

    let mut capability_request = valid_request("node-restricted", "node-3");
    capability_request.requested_capabilities = vec!["node:admin".to_owned()];
    let capability_error = host
        .execute(&manifest, &capability_request)
        .expect_err("undeclared capability requests must be rejected before spawn");
    assert!(matches!(
        capability_error,
        ContractError::NodePluginCapabilityNotDeclared {
            plugin_id,
            capability
        } if plugin_id == "node-restricted" && capability == "node:admin"
    ));
    assert!(!marker_path.exists());

    let mut version_request = valid_request("node-restricted", "node-3");
    version_request.contract_version = "2.0.0".to_owned();
    let version_error = host
        .execute(&manifest, &version_request)
        .expect_err("unsupported request contract version must be rejected before spawn");
    assert!(matches!(
        version_error,
        ContractError::UnsupportedFutureMajorVersion {
            field: "node_plugin_request.contract_version",
            major: 2,
            max_supported_major: 1,
        }
    ));
    assert!(!marker_path.exists());
}

#[test]
fn node_plugin_host_uses_default_deny_environment() {
    let probe_key = select_non_allowlisted_host_env_key();
    let root = unique_test_root("node-plugin-default-deny-env");
    let plugins_root = root.join("plugins");
    let marker_path = root.join("env-leak.marker");
    let executable = plugins_root.join("bin").join("node_env_probe.sh");
    write_env_probe_script(&executable, &probe_key, &marker_path);

    let host = ExternalNodePluginHost::new(plugins_root);
    let manifest = external_node_manifest("node-env-probe", "bin/node_env_probe.sh");
    let request = valid_request("node-env-probe", "node-4");

    host.execute(&manifest, &request)
        .expect("external node plugin should still execute under default-deny env");

    assert!(
        !marker_path.exists(),
        "plugin inherited unexpected host environment variable {probe_key}"
    );
}

fn external_node_manifest(plugin_id: &str, executable: &str) -> PluginManifest {
    PluginManifest {
        api_version: "1.0.0".to_owned(),
        plugin_id: plugin_id.to_owned(),
        kind: PLUGIN_KIND_EXTERNAL_NODE.to_owned(),
        entrypoint: "node.exec.v1".to_owned(),
        capabilities: vec!["node:execute".to_owned(), "normalize".to_owned()],
        executable: Some(executable.to_owned()),
        input_schema: vec!["symbol".to_owned()],
        output_schema: vec!["decision".to_owned()],
    }
}

fn valid_request(plugin_id: &str, node_id: &str) -> ExternalNodePluginRequest {
    ExternalNodePluginRequest {
        contract_version: "1.0.0".to_owned(),
        plugin_id: plugin_id.to_owned(),
        node_id: node_id.to_owned(),
        operation: "normalize".to_owned(),
        requested_capabilities: vec!["node:execute".to_owned()],
        input: BTreeMap::from_iter([("symbol".to_owned(), json!("BTCUSDT"))]),
    }
}

fn write_plugin_script(path: &Path, json_output: &str, marker: Option<&Path>) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("plugin script parent directory should be creatable");
    }

    let mut script = String::from("#!/bin/sh\n");
    if let Some(marker_path) = marker {
        script.push_str(&format!(
            "printf 'spawned' > \"{}\"\n",
            marker_path.display()
        ));
    }
    script.push_str("cat >/dev/null\n");
    script.push_str(&format!("printf '%s' '{json_output}'\n"));
    fs::write(path, script).expect("plugin script fixture should be writable");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path)
            .expect("plugin script metadata should exist")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions)
            .expect("plugin script permissions should be set executable");
    }
}

fn write_env_probe_script(path: &Path, probe_key: &str, marker_path: &Path) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("plugin script parent directory should be creatable");
    }

    let script = format!(
        "#!/bin/sh\nif [ -n \"$(printenv '{probe_key}' 2>/dev/null)\" ]; then\n  printf 'leaked' > \"{}\"\nfi\ncat >/dev/null\nprintf '%s' '{{\"contract_version\":\"1.0.0\",\"success\":true,\"output\":{{\"decision\":\"hold\"}}}}'\n",
        marker_path.display()
    );
    fs::write(path, script).expect("plugin env-probe fixture should be writable");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path)
            .expect("plugin script metadata should exist")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions)
            .expect("plugin script permissions should be set executable");
    }
}

fn select_non_allowlisted_host_env_key() -> String {
    let mut candidates = std::env::vars_os()
        .filter_map(|(key, value)| {
            let key = key.to_string_lossy().to_string();
            if value.is_empty() || PLUGIN_HOST_ENV_ALLOWLIST.contains(&key.as_str()) {
                return None;
            }
            Some(key)
        })
        .collect::<Vec<_>>();
    candidates.sort();
    candidates
        .into_iter()
        .next()
        .expect("test process should expose at least one non-allowlisted environment variable")
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
