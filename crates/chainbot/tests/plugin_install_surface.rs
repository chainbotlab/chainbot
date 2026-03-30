//! [INPUT]
//! CLI binary invocations, temporary local git repositories, and temporary ChainBot roots.
//!
//! [OUTPUT]
//! Verifies remote plugin install success paths, build-required preparation, and overwrite protection.
//!
//! [ROLE]
//! Covers the plugin install side-effect surface as an integration boundary.

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
fn plugin_install_direct_from_git_succeeds() {
    let _lock = acquire_fixture_lock();
    let root = basic_root();
    ensure_basic_root_fixture();
    let repo = unique_root("plugin-install-direct-repo");
    create_direct_plugin_repo(&repo, "remote-node-plugin", "#!/bin/sh\necho direct\n");

    let output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", &root)
        .args(["plugin", "install", "git", repo.to_string_lossy().as_ref()])
        .output()
        .expect("plugin install should execute");

    assert!(output.status.success());
    let installed_root = root.join("plugins").join("remote-node-plugin");
    assert!(installed_root.join("config.toml").is_file());
    let installed_artifact = installed_root.join("bin").join("plugin.sh");
    assert!(installed_artifact.is_file());
    let installed_body =
        fs::read_to_string(&installed_artifact).expect("installed artifact should be readable");
    assert!(installed_body.contains("echo direct"));

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&installed_artifact)
            .expect("installed artifact metadata should be readable")
            .permissions()
            .mode();
        assert_ne!(
            mode & 0o111,
            0,
            "installed artifact should remain executable"
        );
    }

    let validate = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", &root)
        .arg("validate")
        .output()
        .expect("validate should execute");
    assert!(validate.status.success());
}

#[test]
fn plugin_install_build_required_from_git_succeeds() {
    let _lock = acquire_fixture_lock();
    let root = basic_root();
    ensure_basic_root_fixture();
    let repo = unique_root("plugin-install-build-repo");
    create_build_plugin_repo(&repo, "built-plugin");

    let output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", &root)
        .args(["plugin", "install", "git", repo.to_string_lossy().as_ref()])
        .output()
        .expect("plugin install build should execute");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let artifact = root
        .join("plugins")
        .join("built-plugin")
        .join("bin")
        .join("generated.sh");
    assert!(artifact.is_file());
    let contents = fs::read_to_string(artifact).expect("artifact should be readable");
    assert!(contents.contains("built"));
}

#[test]
fn plugin_install_from_local_git_ref_uses_committed_revision() {
    let _lock = acquire_fixture_lock();
    let root = basic_root();
    ensure_basic_root_fixture();
    let repo = unique_root("plugin-install-local-ref");
    create_direct_plugin_repo(&repo, "ref-plugin", "#!/bin/sh\necho first\n");
    let first_rev = git_output(&repo, &["rev-parse", "HEAD"]);
    fs::write(
        repo.join("bin").join("plugin.sh"),
        "#!/bin/sh\necho second\n",
    )
    .expect("updated script should be writable");
    run_git(&repo, &["add", "."]);
    run_git(&repo, &["commit", "-m", "second revision"]);
    fs::write(repo.join("DIRTY_MARKER.txt"), "dirty working tree")
        .expect("dirty marker should be writable");

    let output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", &root)
        .args([
            "plugin",
            "install",
            "git",
            repo.to_string_lossy().as_ref(),
            "--ref",
            first_rev.trim(),
        ])
        .output()
        .expect("plugin install with --ref should execute");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let contents = fs::read_to_string(
        root.join("plugins")
            .join("ref-plugin")
            .join("bin")
            .join("plugin.sh"),
    )
    .expect("installed artifact should be readable");
    assert!(contents.contains("echo first"));
    assert!(!contents.contains("echo second"));
    assert!(!root
        .join("plugins")
        .join("ref-plugin")
        .join("DIRTY_MARKER.txt")
        .exists());
}

#[test]
fn plugin_install_rejects_dirty_local_git_source() {
    let _lock = acquire_fixture_lock();
    let root = basic_root();
    ensure_basic_root_fixture();
    let repo = unique_root("plugin-install-dirty-local-source");
    create_direct_plugin_repo(&repo, "dirty-plugin", "#!/bin/sh\necho clean\n");
    fs::write(
        repo.join("bin").join("plugin.sh"),
        "#!/bin/sh\necho dirty\n",
    )
    .expect("dirty script should be writable");

    let output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", &root)
        .args(["plugin", "install", "git", repo.to_string_lossy().as_ref()])
        .output()
        .expect("plugin install should execute");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(stderr.contains("has uncommitted or untracked changes"));
    assert!(!root.join("plugins").join("dirty-plugin").exists());
}

#[test]
fn plugin_install_rejects_existing_target_without_force() {
    let _lock = acquire_fixture_lock();
    let root = basic_root();
    ensure_basic_root_fixture();
    let repo = unique_root("plugin-install-conflict-repo");
    create_direct_plugin_repo(&repo, "remote-conflict-plugin", "#!/bin/sh\necho first\n");

    let first = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", &root)
        .args(["plugin", "install", "git", repo.to_string_lossy().as_ref()])
        .output()
        .expect("first install should execute");
    assert!(first.status.success());

    let second = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", &root)
        .args(["plugin", "install", "git", repo.to_string_lossy().as_ref()])
        .output()
        .expect("second install should execute");
    assert!(!second.status.success());
    let stderr = String::from_utf8(second.stderr).expect("stderr should be UTF-8");
    assert!(stderr.contains("re-run with --force"));
}

#[test]
fn plugin_install_force_replaces_existing_target() {
    let _lock = acquire_fixture_lock();
    let root = basic_root();
    ensure_basic_root_fixture();
    let repo_a = unique_root("plugin-install-force-a");
    let repo_b = unique_root("plugin-install-force-b");
    create_direct_plugin_repo(&repo_a, "force-plugin", "#!/bin/sh\necho first\n");
    create_direct_plugin_repo(&repo_b, "force-plugin", "#!/bin/sh\necho second\n");

    let first = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", &root)
        .args([
            "plugin",
            "install",
            "git",
            repo_a.to_string_lossy().as_ref(),
        ])
        .output()
        .expect("first install should execute");
    assert!(first.status.success());

    let second = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", &root)
        .args([
            "plugin",
            "install",
            "git",
            repo_b.to_string_lossy().as_ref(),
            "--force",
        ])
        .output()
        .expect("forced install should execute");
    assert!(
        second.status.success(),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );

    let artifact = root
        .join("plugins")
        .join("force-plugin")
        .join("bin")
        .join("plugin.sh");
    let contents = fs::read_to_string(artifact).expect("artifact should be readable");
    assert!(contents.contains("second"));
}

#[test]
fn plugin_install_rejects_missing_embedded_source_metadata() {
    let _lock = acquire_fixture_lock();
    let root = basic_root();
    ensure_basic_root_fixture();
    let repo = unique_root("plugin-install-missing-source");
    create_direct_plugin_repo(&repo, "missing-source-plugin", "#!/bin/sh\necho direct\n");
    fs::write(
        repo.join("config.toml"),
        r#"manifest_version = "2.0.0"
plugin_id = "missing-source-plugin"
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
    .expect("plugin config without embedded source metadata should be writable");
    run_git(&repo, &["add", "."]);
    run_git(&repo, &["commit", "-m", "remove source metadata"]);

    let output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", &root)
        .args(["plugin", "install", "git", repo.to_string_lossy().as_ref()])
        .output()
        .expect("plugin install should execute");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(stderr.contains("plugin source metadata must be declared in config.toml [source]"));
}

#[test]
fn plugin_install_rejects_legacy_source_toml() {
    let _lock = acquire_fixture_lock();
    let root = basic_root();
    ensure_basic_root_fixture();
    let repo = unique_root("plugin-install-legacy-source");
    create_direct_plugin_repo(&repo, "legacy-source-plugin", "#!/bin/sh\necho direct\n");
    fs::write(
        repo.join("source.toml"),
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
        .env("CHAINBOT_CONFIG_DIR", &root)
        .args(["plugin", "install", "git", repo.to_string_lossy().as_ref()])
        .output()
        .expect("plugin install should execute");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(stderr.contains("legacy source.toml is no longer supported"));
}

#[test]
fn plugin_install_allows_mcp_streamable_http_without_plugin_executable() {
    let _lock = acquire_fixture_lock();
    let root = basic_root();
    ensure_basic_root_fixture();
    let repo = unique_root("plugin-install-mcp-http");
    create_mcp_http_plugin_repo(&repo, "mcp-http-plugin");

    let output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", &root)
        .args(["plugin", "install", "git", repo.to_string_lossy().as_ref()])
        .output()
        .expect("plugin install should execute");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let installed = root
        .join("plugins")
        .join("mcp-http-plugin")
        .join("config.toml");
    assert!(installed.is_file());
}

#[test]
fn plugin_install_rejects_mcp_streamable_http_missing_anchor_file() {
    let _lock = acquire_fixture_lock();
    let root = basic_root();
    ensure_basic_root_fixture();
    let repo = unique_root("plugin-install-mcp-http-missing-anchor");
    create_mcp_http_plugin_repo_with_missing_anchor(&repo, "mcp-http-missing-anchor");

    let output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", &root)
        .args(["plugin", "install", "git", repo.to_string_lossy().as_ref()])
        .output()
        .expect("plugin install should execute");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(stderr.contains("entry artifact missing"));
}

#[test]
fn plugin_install_build_required_uses_scrubbed_environment() {
    let _lock = acquire_fixture_lock();
    let root = basic_root();
    ensure_basic_root_fixture();
    let repo = unique_root("plugin-install-build-scrubbed-env");
    create_build_plugin_repo_reading_env(&repo, "built-env-plugin");

    let output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", &root)
        .env("CHAINBOT_BUILD_SECRET", "should-not-leak")
        .args(["plugin", "install", "git", repo.to_string_lossy().as_ref()])
        .output()
        .expect("plugin install build should execute");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let contents = fs::read_to_string(
        root.join("plugins")
            .join("built-env-plugin")
            .join("bin")
            .join("generated.sh"),
    )
    .expect("generated artifact should be readable");
    assert!(contents.contains("missing"));
    assert!(!contents.contains("should-not-leak"));
}

#[test]
fn plugin_install_build_required_times_out() {
    let _lock = acquire_fixture_lock();
    let root = basic_root();
    ensure_basic_root_fixture();
    let repo = unique_root("plugin-install-build-timeout");
    create_build_plugin_repo_that_hangs(&repo, "built-timeout-plugin");

    let output = Command::new(chainbot_bin())
        .env("CHAINBOT_CONFIG_DIR", &root)
        .env("CHAINBOT_SOURCE_BUILD_TIMEOUT_MS", "50")
        .args(["plugin", "install", "git", repo.to_string_lossy().as_ref()])
        .output()
        .expect("plugin install build should execute");

    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");
    assert!(stderr.contains("timed out"));
}

fn ensure_basic_root_fixture() {
    let root = basic_root();
    let state_root = root.join("state");
    let _ = fs::remove_dir_all(root.join("plugins").join("remote-node-plugin"));
    let _ = fs::remove_dir_all(root.join("plugins").join("remote-conflict-plugin"));
    let _ = fs::remove_dir_all(root.join("plugins").join("force-plugin"));
    let _ = fs::remove_dir_all(root.join("plugins").join("built-plugin"));
    let _ = fs::remove_dir_all(root.join("plugins").join("ref-plugin"));
    let _ = fs::remove_dir_all(root.join("plugins").join("dirty-plugin"));
    let _ = fs::remove_dir_all(root.join("plugins").join("built-env-plugin"));
    let _ = fs::remove_dir_all(root.join("plugins").join("built-timeout-plugin"));
    let _ = fs::remove_dir_all(root.join("plugins").join("mcp-http-plugin"));
    let _ = fs::remove_dir_all(root.join("plugins").join("mcp-http-missing-anchor"));
    let _ = fs::remove_dir_all(state_root.join("runs"));
    let _ = fs::remove_dir_all(state_root.join("triggers"));
    let _ = fs::remove_file(state_root.join("coordination.sqlite3"));
    let _ = fs::remove_file(state_root.join("runtime.sqlite3"));

    fs::create_dir_all(root.join("workflows").join("wf-alpha")).expect("workflow dir should exist");
    fs::create_dir_all(root.join("triggers").join("tr-market")).expect("trigger dir should exist");
    fs::create_dir_all(root.join("plugins").join("quote-plugin")).expect("plugin dir should exist");
    fs::create_dir_all(root.join("secrets")).expect("secrets dir should exist");
    fs::create_dir_all(&state_root).expect("state dir should exist");

    fs::write(
        root.join("chainbot.toml"),
        format!(
            "manifest_version = \"2.0.0\"\nchainbot_version = \"{}\"\nprofile = \"basic\"\nsecret_refs = []\n\n[storage]\nmode = \"local\"\n\n[storage.local]\ndatabase_path = \"state/runtime.sqlite3\"\n",
            env!("CARGO_PKG_VERSION")
        ),
    )
    .expect("root config should be writable");
    fs::write(
        root.join("workflows").join("wf-alpha").join("config.toml"),
        "[workflow]\nmanifest_version = \"2.0.0\"\nid = \"wf-alpha\"\nname = \"alpha\"\n\n[[nodes]]\nmanifest_version = \"2.0.0\"\nid = \"node-1\"\nkind = \"plugin\"\nplugin = \"quote-plugin\"\noperation = \"normalize\"\ndepends_on = []\n",
    )
    .expect("workflow config should be writable");
    fs::write(
        root.join("triggers").join("tr-market").join("config.toml"),
        "manifest_version = \"2.0.0\"\ntrigger_id = \"tr-market\"\nkind = \"builtin\"\nsource = \"market_tick\"\nworkflow_id = \"wf-alpha\"\nenabled = false\n",
    )
    .expect("trigger config should be writable");
    fs::write(
        root.join("plugins").join("quote-plugin").join("config.toml"),
        "manifest_version = \"2.0.0\"\nplugin_id = \"quote-plugin\"\nkind = \"builtin\"\nentrypoint = \"plugins.quote\"\ncapabilities = [\"normalize\"]\n",
    )
    .expect("builtin plugin should be writable");
}

fn create_direct_plugin_repo(root: &Path, plugin_id: &str, script_body: &str) {
    let _ = fs::remove_dir_all(root);
    fs::create_dir_all(root.join("bin")).expect("bin dir should be creatable");
    fs::write(
        root.join("config.toml"),
        format!(
            "manifest_version = \"2.0.0\"\nplugin_id = \"{plugin_id}\"\nkind = \"external_node\"\nentrypoint = \"node.exec.v1\"\ncapabilities = [\"node:execute\"]\nexecutable = \"bin/plugin.sh\"\n\n[[operations]]\nname = \"normalize\"\nsummary = \"Normalize payload\"\ninput_schema = [\"symbol\"]\noutput_schema = [\"decision\"]\n\n[source]\nmanifest_version = \"1.0.0\"\ninstall_mode = \"direct\"\nruntime = \"bin\"\nentry_artifact = \"bin/plugin.sh\"\nrelease_version = \"0.1.0\"\n"
        ),
    )
    .expect("plugin config should be writable");
    fs::write(root.join("bin").join("plugin.sh"), script_body).expect("script should be writable");
    set_executable(&root.join("bin").join("plugin.sh"));
    init_git_repo(root);
}

fn create_build_plugin_repo(root: &Path, plugin_id: &str) {
    let _ = fs::remove_dir_all(root);
    fs::create_dir_all(root.join("bin")).expect("bin dir should be creatable");
    fs::write(
        root.join("config.toml"),
        format!(
            "manifest_version = \"2.0.0\"\nplugin_id = \"{plugin_id}\"\nkind = \"external_node\"\nentrypoint = \"node.exec.v1\"\ncapabilities = [\"node:execute\"]\nexecutable = \"bin/generated.sh\"\n\n[[operations]]\nname = \"normalize\"\nsummary = \"Normalize payload\"\ninput_schema = [\"symbol\"]\noutput_schema = [\"decision\"]\n\n[source]\nmanifest_version = \"1.0.0\"\ninstall_mode = \"build_required\"\nruntime = \"bin\"\nentry_artifact = \"bin/generated.sh\"\n\n[source.build]\nkind = \"cargo\"\ncommand = [\"sh\", \"build.sh\"]\nworkdir = \".\"\n\n[[source.build.outputs]]\nfrom = \"dist/generated.sh\"\nto = \"bin/generated.sh\"\n"
        ),
    )
    .expect("plugin config should be writable");
    fs::write(
        root.join("build.sh"),
        "#!/bin/sh\nset -eu\nmkdir -p dist\nprintf '#!/bin/sh\\necho built\\n' > dist/generated.sh\nchmod +x dist/generated.sh\n",
    )
    .expect("build script should be writable");
    set_executable(&root.join("build.sh"));
    init_git_repo(root);
}

fn create_mcp_http_plugin_repo(root: &Path, plugin_id: &str) {
    let _ = fs::remove_dir_all(root);
    fs::create_dir_all(root).expect("repo root should be creatable");
    fs::write(
        root.join("config.toml"),
        format!(
            "manifest_version = \"2.0.0\"\nplugin_id = \"{plugin_id}\"\nkind = \"external_node\"\nentrypoint = \"mcp.tool.v1\"\ncapabilities = [\"node:execute\"]\n\n[[operations]]\nname = \"echo\"\nsummary = \"Echo tool\"\ninput_schema = [\"message\"]\noutput_schema = [\"message\"]\n\n[mcp]\ntransport = \"streamable_http\"\n\n[mcp.streamable_http]\nurl = \"https://example.test/mcp\"\n\n[source]\nmanifest_version = \"1.0.0\"\ninstall_mode = \"direct\"\nruntime = \"bin\"\nentry_artifact = \"config.toml\"\nrelease_version = \"0.1.0\"\n"
        ),
    )
    .expect("mcp plugin config should be writable");
    init_git_repo(root);
}

fn create_mcp_http_plugin_repo_with_missing_anchor(root: &Path, plugin_id: &str) {
    let _ = fs::remove_dir_all(root);
    fs::create_dir_all(root).expect("repo root should be creatable");
    fs::write(
        root.join("config.toml"),
        format!(
            "manifest_version = \"2.0.0\"\nplugin_id = \"{plugin_id}\"\nkind = \"external_node\"\nentrypoint = \"mcp.tool.v1\"\ncapabilities = [\"node:execute\"]\n\n[[operations]]\nname = \"echo\"\nsummary = \"Echo tool\"\ninput_schema = [\"message\"]\noutput_schema = [\"message\"]\n\n[mcp]\ntransport = \"streamable_http\"\n\n[mcp.streamable_http]\nurl = \"https://example.test/mcp\"\n\n[source]\nmanifest_version = \"1.0.0\"\ninstall_mode = \"direct\"\nruntime = \"bin\"\nentry_artifact = \"artifacts/anchor.txt\"\nrelease_version = \"0.1.0\"\n"
        ),
    )
    .expect("mcp plugin config should be writable");
    init_git_repo(root);
}

fn create_build_plugin_repo_reading_env(root: &Path, plugin_id: &str) {
    create_build_plugin_repo(root, plugin_id);
    fs::write(
        root.join("build.sh"),
        "#!/bin/sh\nset -eu\nmkdir -p dist\nprintf '#!/bin/sh\\necho %s\\n' \"${CHAINBOT_BUILD_SECRET:-missing}\" > dist/generated.sh\nchmod +x dist/generated.sh\n",
    )
    .expect("env-reading build script should be writable");
    set_executable(&root.join("build.sh"));
    run_git(root, &["add", "."]);
    run_git(root, &["commit", "-m", "read env in build"]);
}

fn create_build_plugin_repo_that_hangs(root: &Path, plugin_id: &str) {
    create_build_plugin_repo(root, plugin_id);
    fs::write(root.join("build.sh"), "#!/bin/sh\nset -eu\nsleep 1\n")
        .expect("hanging build script should be writable");
    set_executable(&root.join("build.sh"));
    run_git(root, &["add", "."]);
    run_git(root, &["commit", "-m", "hang build"]);
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

fn git_output(root: &Path, args: &[&str]) -> String {
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
    String::from_utf8(output.stdout).expect("git stdout should be UTF-8")
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

fn basic_root() -> PathBuf {
    workspace_root()
        .join("target")
        .join("test-roots")
        .join("basic-plugin-install")
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
