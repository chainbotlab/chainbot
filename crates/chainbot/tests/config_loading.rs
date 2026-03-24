//! [INPUT]
//! Temporary fixture roots, environment overrides, and TOML package definitions.
//!
//! [OUTPUT]
//! Verifies root-layout resolution, startup-time root config version gating, and package-loader behavior for valid, missing, and invalid manifests.
//!
//! [ROLE]
//! Covers the configuration loading boundary as an integration test.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use chainbot::config::{
    load_effective_root_layout, RootDefinitionBundle, RootLayout, CHAINBOT_CONFIG_DIR_ENV,
    DEFAULT_ROOT_DIR_NAME,
};
use chainbot::errors::ContractError;
use chainbot::workflow::RuntimeVariableNamespace;

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
        invalid_version_root.join("chainbot.toml"),
        "manifest_version = \"4.0.0\"\nprofile = \"test\"\n\n[storage]\nmode = \"local\"\n\n[storage.local]\ndatabase_path = \"state/runtime.sqlite3\"\n",
    )
    .expect("invalid-version root fixture should be writable");

    let invalid_version_layout = RootLayout::from_root(invalid_version_root);
    let version_error = RootDefinitionBundle::load(&invalid_version_layout)
        .expect_err("future-major root schema must be rejected");
    assert!(matches!(
        version_error,
        ContractError::UnsupportedMajorVersion {
            field: "root_config.manifest_version",
            major: 4,
            supported_major: 2
        }
    ));
}

#[test]
fn storage_mode_requires_backend_specific_fields() {
    let local_missing_root = unique_test_root("storage-local-missing-path");
    write_valid_fixture(&local_missing_root);
    fs::write(
        local_missing_root.join("chainbot.toml"),
        &format!(
            "manifest_version = \"2.0.0\"\nchainbot_version = \"{}\"\nprofile = \"local-missing\"\n\n[storage]\nmode = \"local\"\n",
            env!("CARGO_PKG_VERSION")
        ),
    )
    .expect("local-missing root config should be writable");
    let local_error = RootDefinitionBundle::load(&RootLayout::from_root(local_missing_root))
        .expect_err("local mode without storage.local.database_path should fail");
    assert!(matches!(
        local_error,
        ContractError::InvalidRootConfigField {
            field: "root_config.storage.local.database_path",
            ..
        }
    ));

    let postgres_missing_root = unique_test_root("storage-postgres-missing-url");
    write_valid_fixture(&postgres_missing_root);
    fs::write(
        postgres_missing_root.join("chainbot.toml"),
        &format!(
            "manifest_version = \"2.0.0\"\nchainbot_version = \"{}\"\nprofile = \"postgres-missing\"\n\n[storage]\nmode = \"postgres\"\n",
            env!("CARGO_PKG_VERSION")
        ),
    )
    .expect("postgres-missing root config should be writable");
    let postgres_error = RootDefinitionBundle::load(&RootLayout::from_root(postgres_missing_root))
        .expect_err("postgres mode without storage.postgres.database_url should fail");
    assert!(matches!(
        postgres_error,
        ContractError::InvalidRootConfigField {
            field: "root_config.storage.postgres.database_url",
            ..
        }
    ));
}

#[test]
fn storage_retention_requires_at_least_one_window_when_enabled() {
    let root = unique_test_root("storage-retention-missing-window");
    write_valid_fixture(&root);
    fs::write(
        root.join("chainbot.toml"),
        &format!(
            "manifest_version = \"2.0.0\"\nchainbot_version = \"{}\"\nprofile = \"retention\"\n\n[storage]\nmode = \"local\"\n\n[storage.local]\ndatabase_path = \"state/runtime.sqlite3\"\n\n[storage.retention]\nenabled = true\n",
            env!("CARGO_PKG_VERSION")
        ),
    )
    .expect("retention root config should be writable");

    let error = RootDefinitionBundle::load(&RootLayout::from_root(root))
        .expect_err("enabled retention without a window should fail");
    assert!(matches!(
        error,
        ContractError::InvalidRootConfigField {
            field: "root_config.storage.retention",
            ..
        }
    ));
}

#[test]
fn plugin_manifest_loading_preserves_optional_richer_metadata() {
    let root = unique_test_root("plugin-metadata-loading");
    write_valid_fixture_with_plugin_package(&root);
    fs::write(
        root.join("plugins").join("quote-plugin").join("config.toml"),
        "manifest_version = \"2.0.0\"\nplugin_id = \"quote-plugin\"\nkind = \"external_node\"\nentrypoint = \"node.exec.v1\"\ncapabilities = [\"node:execute\"]\nexecutable = \"bin/external_node.sh\"\n\n[[operations]]\nname = \"normalize\"\nsummary = \"Normalize quote payload\"\ninput_schema = [\"symbol\", \"token\"]\noutput_schema = [\"decision\"]\n",
    )
    .expect("plugin metadata fixture should be writable");

    let bundle = RootDefinitionBundle::load(&RootLayout::from_root(root))
        .expect("root with richer plugin metadata should load");
    assert_eq!(bundle.plugins.len(), 1);
    assert_eq!(bundle.plugins[0].operations.len(), 1);
    assert_eq!(bundle.plugins[0].operations[0].name, "normalize");
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

#[test]
fn missing_chainbot_version_is_backfilled_during_startup_load() {
    let root = unique_test_root("config-version-backfill");
    write_valid_fixture(&root);
    fs::write(
        root.join("chainbot.toml"),
        "manifest_version = \"2.0.0\"\nprofile = \"basic\"\nsecret_refs = [\"secret://ops/slack/webhook\"]\n\n[storage]\nmode = \"local\"\n\n[storage.local]\ndatabase_path = \"state/runtime.sqlite3\"\n",
    )
    .expect("root config without chainbot_version should be writable");

    let layout = RootLayout::from_root(root.clone());
    let effective_layout =
        load_effective_root_layout(&layout).expect("startup load should backfill missing version");
    assert_eq!(effective_layout.root, root);

    let bundle = RootDefinitionBundle::load(&layout)
        .expect("root bundle should load after missing version backfill");
    assert_eq!(
        bundle.root_config.chainbot_version.as_deref(),
        Some(env!("CARGO_PKG_VERSION"))
    );

    let rewritten_root_config =
        fs::read_to_string(root.join("chainbot.toml")).expect("rewritten root config should exist");
    assert!(rewritten_root_config.contains(&format!(
        "chainbot_version = \"{}\"",
        env!("CARGO_PKG_VERSION")
    )));
}

#[test]
fn matching_chainbot_version_loads_successfully() {
    let root = unique_test_root("config-version-match");
    write_valid_fixture(&root);
    let root_config_path = root.join("chainbot.toml");
    let original_root_config =
        fs::read_to_string(&root_config_path).expect("matching root config should be readable");

    let bundle = RootDefinitionBundle::load(&RootLayout::from_root(root))
        .expect("matching chainbot_version should load without migration");
    assert_eq!(
        bundle.root_config.chainbot_version.as_deref(),
        Some(env!("CARGO_PKG_VERSION"))
    );

    let reloaded_root_config =
        fs::read_to_string(&root_config_path).expect("matching root config should stay readable");
    assert_eq!(reloaded_root_config, original_root_config);
}

#[test]
fn mismatched_chainbot_version_requires_migration() {
    let root = unique_test_root("config-version-mismatch");
    write_valid_fixture(&root);
    fs::write(
        root.join("chainbot.toml"),
        "manifest_version = \"2.0.0\"\nchainbot_version = \"0.9.0\"\nprofile = \"basic\"\nsecret_refs = [\"secret://ops/slack/webhook\"]\n\n[storage]\nmode = \"local\"\n\n[storage.local]\ndatabase_path = \"state/runtime.sqlite3\"\n",
    )
    .expect("root config with mismatched chainbot_version should be writable");

    let error = RootDefinitionBundle::load(&RootLayout::from_root(root.clone()))
        .expect_err("mismatched chainbot_version should require migration");
    assert!(matches!(
        error,
        ContractError::ConfigVersionMigrationRequired {
            stored_version,
            running_version,
        } if stored_version == "0.9.0" && running_version == env!("CARGO_PKG_VERSION")
    ));

    let root_config = fs::read_to_string(root.join("chainbot.toml"))
        .expect("mismatched root config should remain readable");
    assert!(root_config.contains("chainbot_version = \"0.9.0\""));
}

#[test]
fn canonical_plugin_packages_are_discovered() {
    let root = unique_test_root("canonical-plugin-packages");
    write_valid_fixture_with_plugin_package(&root);

    let layout = RootLayout::from_root(root);
    let bundle = RootDefinitionBundle::load(&layout).expect("canonical plugin package should load");

    assert_eq!(bundle.plugins.len(), 1);
    assert_eq!(bundle.plugins[0].plugin_id, "quote-plugin");
    assert_eq!(
        bundle.plugins[0].manifest_path,
        layout.plugins_dir.join("quote-plugin").join("config.toml")
    );
}

#[test]
fn workflow_subflow_call_dsl_is_lowered_during_loading() {
    let root = unique_test_root("workflow-subflow-call-dsl");
    write_subflow_call_fixture(&root);

    let layout = RootLayout::from_root(root);
    let bundle = RootDefinitionBundle::load(&layout).expect("subflow call fixture should load");
    let workflow = bundle
        .workflows
        .iter()
        .find(|workflow| workflow.workflow_id == "wf-parent")
        .expect("parent workflow should load");
    let node = workflow
        .nodes
        .iter()
        .find(|node| node.node_id == "call-strategy")
        .expect("subflow node should load");

    assert_eq!(node.kind, "subflow");
    assert_eq!(node.plugin_id, "builtin-subflow");
    assert_eq!(node.operation, "run");

    let contract = node
        .subflow
        .as_ref()
        .expect("subflow call DSL should lower into canonical contract");
    assert_eq!(contract.workflow_id, "wf-child");
    assert_eq!(contract.imports.len(), 2);
    assert_eq!(contract.exports.len(), 2);
    assert_eq!(contract.imports[0].child_key, "dry_run");
    assert_eq!(
        contract.imports[0].source.namespace,
        RuntimeVariableNamespace::ManualInvocationInput
    );
    assert_eq!(contract.imports[0].source.key, "dry_run");
    assert_eq!(contract.imports[1].child_key, "symbol");
    assert_eq!(
        contract.imports[1].source.namespace,
        RuntimeVariableNamespace::TriggerPayloadMapping
    );
    assert_eq!(contract.imports[1].source.key, "symbol");
    assert_eq!(contract.exports[0].child_key, "decision");
    assert_eq!(contract.exports[0].parent_key, "strategy_decision");
    assert_eq!(contract.exports[1].child_key, "reason");
    assert_eq!(contract.exports[1].parent_key, "strategy_reason");
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
    fs::create_dir_all(root.join("workflows")).expect("workflows directory should be creatable");
    fs::create_dir_all(root.join("triggers")).expect("triggers directory should be creatable");
    fs::create_dir_all(root.join("plugins").join("quote-plugin"))
        .expect("plugin package directory should be creatable");
    fs::create_dir_all(root.join("plugins").join("manifests"))
        .expect("plugin manifests directory should be creatable");
    fs::create_dir_all(root.join("secrets")).expect("secrets directory should be creatable");
    fs::create_dir_all(root.join("state")).expect("state directory should be creatable");
    fs::create_dir_all(root.join("workflows").join("wf-alpha"))
        .expect("workflow package directory should be creatable");
    fs::create_dir_all(root.join("triggers").join("tr-market"))
        .expect("trigger package directory should be creatable");

    fs::write(
        root.join("chainbot.toml"),
        &format!(
            "manifest_version = \"2.0.0\"\nchainbot_version = \"{}\"\nprofile = \"basic\"\nsecret_refs = [\"secret://ops/slack/webhook\"]\n\n[storage]\nmode = \"local\"\n\n[storage.local]\ndatabase_path = \"state/runtime.sqlite3\"\n",
            env!("CARGO_PKG_VERSION")
        ),
    )
    .expect("root config fixture should be writable");

    fs::write(
        root.join("workflows").join("wf-alpha").join("config.toml"),
        "[workflow]\nmanifest_version = \"2.0.0\"\nid = \"wf-alpha\"\nname = \"alpha\"\n\n[runtime.defaults]\nregion = \"us\"\n\n[[nodes]]\nmanifest_version = \"2.0.0\"\nid = \"node-1\"\nkind = \"plugin\"\nplugin = \"quote-plugin\"\noperation = \"normalize\"\ndepends_on = []\n",
    )
    .expect("workflow fixture should be writable");

    fs::write(
        root.join("triggers").join("tr-market").join("config.toml"),
        "manifest_version = \"2.0.0\"\ntrigger_id = \"tr-market\"\nkind = \"builtin\"\nsource = \"market_tick\"\nworkflow_id = \"wf-alpha\"\nenabled = true\n\n[input_mapping]\nregion = \"payload.region\"\n",
    )
    .expect("trigger fixture should be writable");

    fs::write(
        root.join("plugins").join("quote-plugin").join("config.toml"),
        "manifest_version = \"2.0.0\"\nplugin_id = \"quote-plugin\"\nkind = \"builtin\"\nentrypoint = \"plugins.quote\"\ncapabilities = [\"normalize\"]\n",
    )
    .expect("plugin fixture should be writable");
}

fn write_valid_fixture_with_overrides(root: &Path) {
    fs::create_dir_all(root.join("defs").join("workflow-pkgs").join("wf-alpha"))
        .expect("workflow override directory should be creatable");
    fs::create_dir_all(root.join("defs").join("trigger-pkgs").join("tr-market"))
        .expect("trigger override directory should be creatable");
    fs::create_dir_all(root.join("shared").join("plugins").join("quote-plugin"))
        .expect("plugin package directory should be creatable");
    fs::create_dir_all(root.join("vault")).expect("secrets override directory should be creatable");
    fs::create_dir_all(root.join("runtime-state"))
        .expect("state override directory should be creatable");

    fs::write(
        root.join("chainbot.toml"),
        &format!(
            "manifest_version = \"2.0.0\"\nchainbot_version = \"{}\"\nprofile = \"override\"\n\n[paths]\nworkflows_dir = \"defs/workflow-pkgs\"\ntriggers_dir = \"defs/trigger-pkgs\"\nplugins_dir = \"shared/plugins\"\nsecrets_dir = \"vault\"\nstate_dir = \"runtime-state\"\n\n[storage]\nmode = \"local\"\n\n[storage.local]\ndatabase_path = \"runtime-state/runtime.sqlite3\"\n",
            env!("CARGO_PKG_VERSION")
        ),
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
        "manifest_version = \"2.0.0\"\ntrigger_id = \"tr-market\"\nkind = \"builtin\"\nsource = \"market_tick\"\nworkflow_id = \"wf-alpha\"\nenabled = true\n",
    )
    .expect("override trigger fixture should be writable");

    fs::write(
        root.join("shared")
            .join("plugins")
            .join("quote-plugin")
            .join("config.toml"),
        "manifest_version = \"2.0.0\"\nplugin_id = \"quote-plugin\"\nkind = \"builtin\"\nentrypoint = \"plugins.quote\"\ncapabilities = [\"normalize\"]\n",
    )
    .expect("override plugin fixture should be writable");
}

fn write_subflow_call_fixture(root: &Path) {
    fs::create_dir_all(root.join("workflows").join("wf-parent"))
        .expect("parent workflow package directory should be creatable");
    fs::create_dir_all(root.join("workflows").join("wf-child"))
        .expect("child workflow package directory should be creatable");
    fs::create_dir_all(root.join("triggers").join("tr-market"))
        .expect("trigger package directory should be creatable");
    fs::create_dir_all(root.join("plugins").join("quote-plugin"))
        .expect("plugin package directory should be creatable");
    fs::create_dir_all(root.join("secrets")).expect("secrets directory should be creatable");
    fs::create_dir_all(root.join("state")).expect("state directory should be creatable");

    fs::write(
        root.join("chainbot.toml"),
        &format!(
            "manifest_version = \"2.0.0\"\nchainbot_version = \"{}\"\nprofile = \"subflow\"\n\n[storage]\nmode = \"local\"\n\n[storage.local]\ndatabase_path = \"state/runtime.sqlite3\"\n",
            env!("CARGO_PKG_VERSION")
        ),
    )
    .expect("root config fixture should be writable");

    fs::write(
        root.join("workflows").join("wf-parent").join("config.toml"),
        "[workflow]\nmanifest_version = \"2.0.0\"\nid = \"wf-parent\"\nname = \"parent\"\n\n[[nodes]]\nmanifest_version = \"2.0.0\"\nid = \"call-strategy\"\nkind = \"subflow\"\ndepends_on = []\n\n[nodes.when]\nsource = \"run.enabled\"\noperator = \"truthy\"\n\n[nodes.call]\nworkflow = \"wf-child\"\n\n[nodes.call.with]\nsymbol = \"trigger.symbol\"\ndry_run = \"manual.dry_run\"\n\n[nodes.call.returns]\ndecision = \"strategy_decision\"\nreason = \"strategy_reason\"\n",
    )
    .expect("parent workflow fixture should be writable");

    fs::write(
        root.join("workflows").join("wf-child").join("config.toml"),
        "[workflow]\nmanifest_version = \"2.0.0\"\nid = \"wf-child\"\nname = \"child\"\n\n[[nodes]]\nmanifest_version = \"2.0.0\"\nid = \"emit\"\nkind = \"plugin\"\nplugin = \"quote-plugin\"\noperation = \"normalize\"\ndepends_on = []\n",
    )
    .expect("child workflow fixture should be writable");

    fs::write(
        root.join("triggers").join("tr-market").join("config.toml"),
        "manifest_version = \"2.0.0\"\ntrigger_id = \"tr-market\"\nkind = \"builtin\"\nsource = \"market_tick\"\nworkflow_id = \"wf-parent\"\nenabled = true\n",
    )
    .expect("trigger fixture should be writable");

    fs::write(
        root.join("plugins").join("quote-plugin").join("config.toml"),
        "manifest_version = \"2.0.0\"\nplugin_id = \"quote-plugin\"\nkind = \"builtin\"\nentrypoint = \"plugins.quote\"\ncapabilities = [\"normalize\"]\n",
    )
    .expect("plugin fixture should be writable");
}

fn write_valid_fixture_with_plugin_package(root: &Path) {
    fs::create_dir_all(root.join("workflows").join("wf-alpha"))
        .expect("workflow package directory should be creatable");
    fs::create_dir_all(root.join("triggers").join("tr-market"))
        .expect("trigger package directory should be creatable");
    fs::create_dir_all(root.join("plugins").join("quote-plugin"))
        .expect("plugin package directory should be creatable");
    fs::create_dir_all(root.join("secrets")).expect("secrets directory should be creatable");
    fs::create_dir_all(root.join("state")).expect("state directory should be creatable");

    fs::write(
        root.join("chainbot.toml"),
        &format!(
            "manifest_version = \"2.0.0\"\nchainbot_version = \"{}\"\nprofile = \"basic\"\nsecret_refs = [\"secret://ops/slack/webhook\"]\n\n[storage]\nmode = \"local\"\n\n[storage.local]\ndatabase_path = \"state/runtime.sqlite3\"\n",
            env!("CARGO_PKG_VERSION")
        ),
    )
    .expect("root config fixture should be writable");

    fs::write(
        root.join("workflows").join("wf-alpha").join("config.toml"),
        "[workflow]\nmanifest_version = \"2.0.0\"\nid = \"wf-alpha\"\nname = \"alpha\"\n\n[[nodes]]\nmanifest_version = \"2.0.0\"\nid = \"node-1\"\nkind = \"plugin\"\nplugin = \"quote-plugin\"\noperation = \"normalize\"\ndepends_on = []\n",
    )
    .expect("workflow fixture should be writable");

    fs::write(
        root.join("triggers").join("tr-market").join("config.toml"),
        "manifest_version = \"2.0.0\"\ntrigger_id = \"tr-market\"\nkind = \"builtin\"\nsource = \"market_tick\"\nworkflow_id = \"wf-alpha\"\nenabled = true\n",
    )
    .expect("trigger fixture should be writable");

    fs::write(
        root.join("plugins").join("quote-plugin").join("config.toml"),
        "manifest_version = \"2.0.0\"\nplugin_id = \"quote-plugin\"\nkind = \"builtin\"\nentrypoint = \"plugins.quote\"\ncapabilities = [\"normalize\"]\n",
    )
    .expect("plugin package fixture should be writable");
}
