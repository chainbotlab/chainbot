//! [INPUT]
//! Temporary fixture roots, environment overrides, and TOML package definitions.
//!
//! [OUTPUT]
//! Verifies root-layout resolution and package-loader behavior for valid, missing, and invalid manifests.
//!
//! [ROLE]
//! Covers the configuration loading boundary as an integration test.


use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use chainbot::config::{
    RootDefinitionBundle, RootLayout, CHAINBOT_CONFIG_DIR_ENV, DEFAULT_ROOT_DIR_NAME,
};
use chainbot::errors::ContractError;

#[test]
fn config_root_layout() {
    let _guard = config_env_lock();
    let home_root = unique_test_root("config-root-layout-home");
    let fake_home = home_root.join("home-user");
    let _env_guard = ChainbotConfigDirGuard::capture();
    unsafe {
        std::env::remove_var(CHAINBOT_CONFIG_DIR_ENV);
    }
    let resolved = RootLayout::resolve_with_env_home(None, Some(&fake_home))
        .expect("default root should resolve from provided home path");
    assert_eq!(resolved.root, fake_home.join(DEFAULT_ROOT_DIR_NAME));

    let env_root = home_root.join("custom-root");
    let env_layout = RootLayout::resolve_with_env_home(Some(&env_root), Some(&fake_home))
        .expect("config-dir override should bypass default ~/.chainbot resolution");
    assert_eq!(env_layout.root, env_root);
}

#[test]
fn config_root_layout_prefers_chainbot_config_dir_env() {
    let _guard = config_env_lock();
    let home_root = unique_test_root("config-root-layout-env");
    let fake_home = home_root.join("home-user");
    let env_root = home_root.join("env-root");
    let _env_guard = ChainbotConfigDirGuard::capture();

    unsafe {
        std::env::set_var(CHAINBOT_CONFIG_DIR_ENV, &env_root);
    }
    let resolved = RootLayout::resolve_with_env_home(None, Some(&fake_home))
        .expect("env root should override the HOME-based default");
    assert_eq!(resolved.root, env_root);

    unsafe {
        std::env::set_var(CHAINBOT_CONFIG_DIR_ENV, "");
    }
    let fallback = RootLayout::resolve_with_env_home(None, Some(&fake_home))
        .expect("empty env root should fall back to the HOME-based default");
    assert_eq!(fallback.root, fake_home.join(DEFAULT_ROOT_DIR_NAME));
}

fn config_env_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

struct ChainbotConfigDirGuard {
    previous: Option<std::ffi::OsString>,
}

impl ChainbotConfigDirGuard {
    fn capture() -> Self {
        Self {
            previous: std::env::var_os(CHAINBOT_CONFIG_DIR_ENV),
        }
    }
}

impl Drop for ChainbotConfigDirGuard {
    fn drop(&mut self) {
        unsafe {
            match self.previous.take() {
                Some(value) => std::env::set_var(CHAINBOT_CONFIG_DIR_ENV, value),
                None => std::env::remove_var(CHAINBOT_CONFIG_DIR_ENV),
            }
        }
    }
}

#[test]
fn toml_definition_validation() {
    let valid_root = unique_test_root("toml-valid");
    write_valid_fixture(&valid_root);
    let valid_layout = RootLayout::from_root(valid_root);
    let bundle = RootDefinitionBundle::load(&valid_layout).expect("valid fixture root should load");
    assert_eq!(bundle.root_config.schema_version, "2.0.0");
    assert_eq!(bundle.workflows.len(), 1);
    assert_eq!(bundle.triggers.len(), 1);
    assert_eq!(bundle.plugins.len(), 1);

    let missing_root = unique_test_root("toml-missing");
    let missing_layout = RootLayout::from_root(missing_root.clone());
    let missing_error =
        RootDefinitionBundle::load(&missing_layout).expect_err("missing root must fail");
    assert!(matches!(
        missing_error,
        ContractError::MissingDirectory { path, kind: "root" } if path == missing_root
    ));

    let invalid_version_root = unique_test_root("toml-invalid-version");
    write_valid_fixture(&invalid_version_root);
    fs::write(
        invalid_version_root.join("config").join("root.toml"),
        "manifest_version = \"3.0.0\"\nprofile = \"test\"\n",
    )
    .expect("invalid-version root fixture should be writable");

    let invalid_version_layout = RootLayout::from_root(invalid_version_root);
    let version_error = RootDefinitionBundle::load(&invalid_version_layout)
        .expect_err("future-major root schema must be rejected");
    assert!(matches!(
        version_error,
        ContractError::UnsupportedFutureMajorVersion {
            field: "root_config.manifest_version",
            major: 3,
            max_supported_major: 2
        }
    ));
}

#[test]
fn invalid_toml_fixture_rejected() {
    let invalid_root = unique_test_root("toml-invalid-syntax");
    write_valid_fixture(&invalid_root);
    fs::write(
        invalid_root
            .join("workflows")
            .join("wf-alpha")
            .join("config.toml"),
        "[workflow]\nmanifest_version = \"2.0.0\"\nid = \"wf-alpha\n",
    )
    .expect("invalid TOML fixture should be writable");

    let layout = RootLayout::from_root(invalid_root.join("."));
    let error = RootDefinitionBundle::load(&layout)
        .expect_err("broken workflow TOML should produce structured decode error");
    assert!(matches!(error, ContractError::TomlDecode { .. }));
}

#[test]
fn root_paths_overrides_and_plugin_discovery_are_applied() {
    let root = unique_test_root("root-path-overrides");
    write_valid_fixture_with_overrides(&root);

    let layout = RootLayout::from_root(root);
    let bundle = RootDefinitionBundle::load(&layout).expect("override fixture root should load");

    assert_eq!(bundle.workflows.len(), 1);
    assert_eq!(bundle.triggers.len(), 1);
    assert_eq!(bundle.plugins.len(), 1);
    assert_eq!(bundle.plugins[0].plugin_id, "quote-plugin");
    assert_eq!(bundle.workflows[0].workflow_id, "wf-alpha");
    assert_eq!(bundle.triggers[0].workflow_id, "wf-alpha");
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

fn write_valid_fixture(root: &Path) {
    fs::create_dir_all(root.join("config")).expect("config directory should be creatable");
    fs::create_dir_all(root.join("workflows")).expect("workflows directory should be creatable");
    fs::create_dir_all(root.join("triggers")).expect("triggers directory should be creatable");
    fs::create_dir_all(root.join("plugins").join("manifests"))
        .expect("plugin manifests directory should be creatable");
    fs::create_dir_all(root.join("secrets")).expect("secrets directory should be creatable");
    fs::create_dir_all(root.join("state")).expect("state directory should be creatable");
    fs::create_dir_all(root.join("workflows").join("wf-alpha"))
        .expect("workflow package directory should be creatable");
    fs::create_dir_all(root.join("triggers").join("tr-market"))
        .expect("trigger package directory should be creatable");

    fs::write(
        root.join("config").join("root.toml"),
        "manifest_version = \"2.0.0\"\nprofile = \"basic\"\nsecret_refs = [\"secret://ops/slack/webhook\"]\n",
    )
    .expect("root config fixture should be writable");

    fs::write(
        root.join("workflows").join("wf-alpha").join("config.toml"),
        "[workflow]\nmanifest_version = \"2.0.0\"\nid = \"wf-alpha\"\nname = \"alpha\"\n\n[runtime.defaults]\nregion = \"us\"\n\n[[nodes]]\nmanifest_version = \"2.0.0\"\nid = \"node-1\"\nkind = \"plugin\"\nplugin = \"quote-plugin\"\noperation = \"normalize\"\ndepends_on = []\n",
    )
    .expect("workflow fixture should be writable");

    fs::write(
        root.join("triggers").join("tr-market").join("config.toml"),
        "manifest_version = \"2.0.0\"\ntrigger_id = \"tr-market\"\nkind = \"market_tick\"\nsource = \"market-feed\"\nworkflow_id = \"wf-alpha\"\nenabled = true\n\n[input_mapping]\nregion = \"payload.region\"\n",
    )
    .expect("trigger fixture should be writable");

    fs::write(
        root.join("plugins").join("manifests").join("quote_plugin.toml"),
        "manifest_version = \"2.0.0\"\nplugin_id = \"quote-plugin\"\nkind = \"builtin\"\nentrypoint = \"plugins.quote\"\ncapabilities = [\"normalize\"]\n",
    )
    .expect("plugin fixture should be writable");
}

fn write_valid_fixture_with_overrides(root: &Path) {
    fs::create_dir_all(root.join("config")).expect("config directory should be creatable");
    fs::create_dir_all(root.join("defs").join("workflow-pkgs").join("wf-alpha"))
        .expect("workflow override directory should be creatable");
    fs::create_dir_all(root.join("defs").join("trigger-pkgs").join("tr-market"))
        .expect("trigger override directory should be creatable");
    fs::create_dir_all(root.join("shared").join("plugins").join("catalog"))
        .expect("plugin discovery directory should be creatable");
    fs::create_dir_all(root.join("vault")).expect("secrets override directory should be creatable");
    fs::create_dir_all(root.join("runtime-state"))
        .expect("state override directory should be creatable");

    fs::write(
        root.join("config").join("root.toml"),
        "manifest_version = \"2.0.0\"\nprofile = \"override\"\n\n[paths]\nworkflows_dir = \"defs/workflow-pkgs\"\ntriggers_dir = \"defs/trigger-pkgs\"\nplugins_dir = \"shared/plugins\"\nsecrets_dir = \"vault\"\nstate_dir = \"runtime-state\"\n\n[plugins]\nmanifest_globs = [\"shared/plugins/catalog/*.toml\"]\n",
    )
    .expect("override root config fixture should be writable");

    fs::write(
        root.join("defs")
            .join("workflow-pkgs")
            .join("wf-alpha")
            .join("config.toml"),
        "[workflow]\nmanifest_version = \"2.0.0\"\nid = \"wf-alpha\"\nname = \"alpha\"\n\n[[nodes]]\nmanifest_version = \"2.0.0\"\nid = \"node-1\"\nkind = \"plugin\"\nplugin = \"quote-plugin\"\noperation = \"normalize\"\ndepends_on = []\n",
    )
    .expect("override workflow fixture should be writable");

    fs::write(
        root.join("defs")
            .join("trigger-pkgs")
            .join("tr-market")
            .join("config.toml"),
        "manifest_version = \"2.0.0\"\ntrigger_id = \"tr-market\"\nkind = \"market_tick\"\nsource = \"market-feed\"\nworkflow_id = \"wf-alpha\"\nenabled = true\n",
    )
    .expect("override trigger fixture should be writable");

    fs::write(
        root.join("shared")
            .join("plugins")
            .join("catalog")
            .join("quote_plugin.toml"),
        "manifest_version = \"2.0.0\"\nplugin_id = \"quote-plugin\"\nkind = \"builtin\"\nentrypoint = \"plugins.quote\"\ncapabilities = [\"normalize\"]\n",
    )
    .expect("override plugin fixture should be writable");
}
