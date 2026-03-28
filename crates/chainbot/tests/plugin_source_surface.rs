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
fn plugin_source_list_requires_embedded_source_metadata() {
    let _lock = acquire_fixture_lock();
    let repo = unique_root("plugin-source-missing-source");
    create_multi_plugin_repo(&repo);
    fs::write(
        repo.join("official-plugins").join("alpha-plugin").join("config.toml"),
        r#"manifest_version = "2.0.0"
plugin_id = "alpha-plugin"
kind = "external_node"
entrypoint = "node.exec.v1"
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
        repo.join("official-plugins").join("alpha-plugin").join("source.toml"),
        r#"manifest_version = "1.0.0"
install_mode = "direct"
runtime = "bin"
entry_artifact = "bin/plugin.sh"
release_version = "0.1.0"
"#,
    )
    .expect("legacy source manifest should be writable");

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

fn create_multi_plugin_repo(root: &Path) {
    let _ = fs::remove_dir_all(root);
    fs::create_dir_all(root.join("official-plugins").join("alpha-plugin").join("bin"))
        .expect("alpha plugin directory should be creatable");
    fs::create_dir_all(root.join("official-plugins").join("beta-plugin").join("bin"))
        .expect("beta plugin directory should be creatable");
    fs::write(
        root.join("chainbot-plugin-index.toml"),
        "manifest_version = \"1.0.0\"\n\n[[plugins]]\nplugin_id = \"alpha-plugin\"\npath = \"official-plugins/alpha-plugin\"\nsummary = \"Alpha plugin\"\n\n[[plugins]]\nplugin_id = \"beta-plugin\"\npath = \"official-plugins/beta-plugin\"\nsummary = \"Beta plugin\"\n",
    )
    .expect("source index should be writable");
    write_external_node_plugin(root.join("official-plugins").join("alpha-plugin"), "alpha-plugin");
    write_external_node_plugin(root.join("official-plugins").join("beta-plugin"), "beta-plugin");
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
    fs::write(
        root.join("bin").join("plugin.sh"),
        "#!/bin/sh\necho '{}'\n",
    )
    .expect("plugin executable should be writable");
    set_executable(&root.join("bin").join("plugin.sh"));
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
    assert!(output.status.success(), "git {:?} failed: {}", args, String::from_utf8_lossy(&output.stderr));
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
