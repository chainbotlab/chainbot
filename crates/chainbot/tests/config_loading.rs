/*
[INPUT]:  Temporary fixture roots and TOML definitions for root validation.
[OUTPUT]: Integration coverage for root layout resolution and TOML loader behavior.
[POS]:    Integration test boundary for task-2 root layout and loader contracts.
[UPDATE]: 2026-03-16 - Add root layout, TOML validation, and invalid TOML tests.
*/

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use chainbot::config::{RootDefinitionBundle, RootLayout, DEFAULT_ROOT_DIR_NAME};
use chainbot::errors::ContractError;

#[test]
fn config_root_layout() {
    let home_root = unique_test_root("config-root-layout-home");
    let fake_home = home_root.join("home-user");
    let resolved = RootLayout::resolve_with_home(None, Some(&fake_home))
        .expect("default root should resolve from provided home path");
    assert_eq!(resolved.root, fake_home.join(DEFAULT_ROOT_DIR_NAME));

    let override_root = home_root.join("custom-root");
    let override_layout = RootLayout::resolve_with_home(Some(&override_root), Some(&fake_home))
        .expect("override root should bypass default ~/.chainbot resolution");
    assert_eq!(override_layout.root, override_root);
}

#[test]
fn toml_definition_validation() {
    let valid_root = unique_test_root("toml-valid");
    write_valid_fixture(&valid_root);
    let valid_layout = RootLayout::from_root(valid_root);
    let bundle = RootDefinitionBundle::load(&valid_layout).expect("valid fixture root should load");
    assert_eq!(bundle.root_config.schema_version, "1.0.0");
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
        "schema_version = \"2.0.0\"\nprofile = \"test\"\n",
    )
    .expect("invalid-version root fixture should be writable");

    let invalid_version_layout = RootLayout::from_root(invalid_version_root);
    let version_error = RootDefinitionBundle::load(&invalid_version_layout)
        .expect_err("future-major root schema must be rejected");
    assert!(matches!(
        version_error,
        ContractError::UnsupportedFutureMajorVersion {
            field: "root_config.schema_version",
            major: 2,
            max_supported_major: 1
        }
    ));
}

#[test]
fn invalid_toml_fixture_rejected() {
    let invalid_root = unique_test_root("toml-invalid-syntax");
    write_valid_fixture(&invalid_root);
    fs::write(
        invalid_root.join("workflows").join("wf_alpha.toml"),
        "api_version = \"1.0.0\"\nworkflow_id = \"wf-alpha\n",
    )
    .expect("invalid TOML fixture should be writable");

    let layout = RootLayout::from_root(invalid_root.join("."));
    let error = RootDefinitionBundle::load(&layout)
        .expect_err("broken workflow TOML should produce structured decode error");
    assert!(matches!(error, ContractError::TomlDecode { .. }));
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
    fs::create_dir_all(root.join("plugins")).expect("plugins directory should be creatable");
    fs::create_dir_all(root.join("secrets")).expect("secrets directory should be creatable");
    fs::create_dir_all(root.join("state")).expect("state directory should be creatable");

    fs::write(
        root.join("config").join("root.toml"),
        "schema_version = \"1.0.0\"\nprofile = \"basic\"\nsecret_refs = [\"secret://ops/slack/webhook\"]\n",
    )
    .expect("root config fixture should be writable");

    fs::write(
        root.join("workflows").join("wf_alpha.toml"),
        "api_version = \"1.0.0\"\nworkflow_id = \"wf-alpha\"\nname = \"alpha\"\n\n[[triggers]]\napi_version = \"1.0.0\"\ntrigger_id = \"inline-tr\"\nkind = \"manual\"\nsource = \"inline\"\nenabled = true\n\n[[nodes]]\napi_version = \"1.0.0\"\nnode_id = \"node-1\"\nkind = \"plugin\"\nplugin_id = \"quote-plugin\"\noperation = \"normalize\"\ndepends_on = []\n",
    )
    .expect("workflow fixture should be writable");

    fs::write(
        root.join("triggers").join("trigger_market.toml"),
        "api_version = \"1.0.0\"\ntrigger_id = \"tr-market\"\nkind = \"market_tick\"\nsource = \"market-feed\"\nenabled = true\n",
    )
    .expect("trigger fixture should be writable");

    fs::write(
        root.join("plugins").join("quote_plugin.toml"),
        "api_version = \"1.0.0\"\nplugin_id = \"quote-plugin\"\nkind = \"builtin\"\nentrypoint = \"plugins.quote\"\ncapabilities = [\"normalize\"]\n",
    )
    .expect("plugin fixture should be writable");
}
