/*
[INPUT]:  Trigger definitions, plugin manifests, synthetic trigger emissions, and temporary state roots.
[OUTPUT]: Integration coverage for trigger plugin manifest checks, dedup/cooldown behavior, run-request normalization, and file-backed trigger records.
[POS]:    Integration test boundary for task-6 trigger plane contracts.
[UPDATE]: 2026-03-16 - Add deterministic trigger-plane acceptance tests.
[UPDATE]: 2026-03-16 - Cover unknown trigger kinds and restart-safe coordination rebuild.
*/

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use chainbot::config::RootLayout;
use chainbot::errors::ContractError;
use chainbot::plugin::PluginManifest;
use chainbot::state::{StateLayout, TriggerEventRecord};
use chainbot::trigger::{
    TriggerDefinition, TriggerEmission, TriggerPlane, TriggerPlaneError, TriggerPluginHostPolicy,
    REQUIRED_TRIGGER_PLUGIN_CAPABILITY,
};
use rusqlite::Connection;

#[test]
fn trigger_plugin_manifest_validation() {
    let (state_layout, plugin_root) = unique_layout("trigger-plugin-manifest-validation");
    let valid_plugin_path = plugin_root.join("valid-trigger.sh");
    write_executable_script(
        &valid_plugin_path,
        "{\"api_version\":\"1.0.0\",\"events\":[]}",
    );

    let definitions = vec![trigger_definition(
        "trigger-external",
        "external_plugin",
        "plugin-ok",
    )];
    let valid_manifest = plugin_manifest(
        "plugin-ok",
        "1.0.0",
        "trigger",
        "valid-trigger.sh",
        &[REQUIRED_TRIGGER_PLUGIN_CAPABILITY],
    );
    let valid_policy = policy(
        &plugin_root,
        &["plugin-ok"],
        &[REQUIRED_TRIGGER_PLUGIN_CAPABILITY],
    );

    let open_result = TriggerPlane::open(
        state_layout.clone(),
        definitions.clone(),
        vec![valid_manifest.clone()],
        valid_policy,
        BTreeMap::new(),
        1_710_100_000_000,
    );
    open_result.expect("valid trigger plugin manifest should pass host validation");

    let alias_kind_open_result = TriggerPlane::open(
        state_layout.clone(),
        vec![trigger_definition(
            "trigger-external-alias",
            "plugin",
            "plugin-ok",
        )],
        vec![valid_manifest.clone()],
        policy(
            &plugin_root,
            &["plugin-ok"],
            &[REQUIRED_TRIGGER_PLUGIN_CAPABILITY],
        ),
        BTreeMap::new(),
        1_710_100_000_000,
    );
    alias_kind_open_result.expect("plugin alias kind should be accepted for trigger definitions");

    let duplicate_trigger_error = TriggerPlane::open(
        state_layout.clone(),
        vec![
            trigger_definition("dup-trigger", "builtin", "market-feed"),
            trigger_definition("dup-trigger", "builtin", "market-feed-two"),
        ],
        Vec::new(),
        policy(&plugin_root, &[], &[REQUIRED_TRIGGER_PLUGIN_CAPABILITY]),
        BTreeMap::new(),
        1_710_100_000_000,
    )
    .expect_err("duplicate trigger ids should be rejected during trigger-plane open");
    assert!(matches!(
        duplicate_trigger_error,
        TriggerPlaneError::Contract(ContractError::DuplicateTriggerId { trigger_id })
            if trigger_id == "dup-trigger"
    ));

    let empty_source_error = TriggerPlane::open(
        state_layout.clone(),
        vec![trigger_definition("bad-source", "builtin", "")],
        Vec::new(),
        policy(&plugin_root, &[], &[REQUIRED_TRIGGER_PLUGIN_CAPABILITY]),
        BTreeMap::new(),
        1_710_100_000_000,
    )
    .expect_err("empty trigger source should be rejected during trigger-plane open");
    assert!(matches!(
        empty_source_error,
        TriggerPlaneError::Contract(ContractError::InvalidTriggerDefinitionField {
            trigger_id,
            field: "trigger.source",
            ..
        }) if trigger_id == "bad-source"
    ));

    let unknown_kind_error = TriggerPlane::open(
        state_layout.clone(),
        vec![trigger_definition(
            "bad-kind",
            "builtin_typo",
            "market-feed",
        )],
        Vec::new(),
        policy(&plugin_root, &[], &[REQUIRED_TRIGGER_PLUGIN_CAPABILITY]),
        BTreeMap::new(),
        1_710_100_000_000,
    )
    .expect_err("unknown trigger kinds should fail validation during trigger-plane open");
    assert!(matches!(
        unknown_kind_error,
        TriggerPlaneError::Contract(ContractError::UnknownTriggerKind { trigger_id, kind })
            if trigger_id == "bad-kind" && kind == "builtin_typo"
    ));

    let missing_capability_manifest = plugin_manifest(
        "plugin-ok",
        "1.0.0",
        "trigger",
        "valid-trigger.sh",
        &["trigger.observe"],
    );
    let missing_capability_error = TriggerPlane::open(
        state_layout.clone(),
        definitions.clone(),
        vec![missing_capability_manifest],
        policy(
            &plugin_root,
            &["plugin-ok"],
            &[REQUIRED_TRIGGER_PLUGIN_CAPABILITY],
        ),
        BTreeMap::new(),
        1_710_100_000_001,
    )
    .expect_err("manifest without required capability must be rejected");
    assert!(matches!(
        missing_capability_error,
        TriggerPlaneError::Contract(ContractError::TriggerPluginMissingCapability {
            plugin_id,
            capability,
        }) if plugin_id == "plugin-ok" && capability == REQUIRED_TRIGGER_PLUGIN_CAPABILITY
    ));

    let escaping_manifest = plugin_manifest(
        "plugin-ok",
        "1.0.0",
        "trigger",
        "../escape.sh",
        &[REQUIRED_TRIGGER_PLUGIN_CAPABILITY],
    );
    let escaping_error = TriggerPlane::open(
        state_layout.clone(),
        definitions.clone(),
        vec![escaping_manifest],
        policy(
            &plugin_root,
            &["plugin-ok"],
            &[REQUIRED_TRIGGER_PLUGIN_CAPABILITY],
        ),
        BTreeMap::new(),
        1_710_100_000_002,
    )
    .expect_err("entrypoint escaping plugin root must be rejected");
    assert!(matches!(
        escaping_error,
        TriggerPlaneError::Contract(ContractError::TriggerPluginEntrypointMustBeRelative {
            plugin_id,
            entrypoint,
        }) if plugin_id == "plugin-ok" && entrypoint == "../escape.sh"
    ));

    let unsupported_api_manifest = plugin_manifest(
        "plugin-ok",
        "2.0.0",
        "trigger",
        "valid-trigger.sh",
        &[REQUIRED_TRIGGER_PLUGIN_CAPABILITY],
    );
    let unsupported_api_error = TriggerPlane::open(
        state_layout,
        definitions,
        vec![unsupported_api_manifest],
        policy(
            &plugin_root,
            &["plugin-ok"],
            &[REQUIRED_TRIGGER_PLUGIN_CAPABILITY],
        ),
        BTreeMap::new(),
        1_710_100_000_003,
    )
    .expect_err("future-major plugin manifest must be rejected");
    assert!(matches!(
        unsupported_api_error,
        TriggerPlaneError::Contract(ContractError::UnsupportedFutureMajorVersion {
            field: "plugin.api_version",
            major: 2,
            max_supported_major: 1,
        })
    ));
}

#[test]
fn trigger_dedup_and_cooldown() {
    let (state_layout, plugin_root) = unique_layout("trigger-dedup-and-cooldown");
    let definitions = vec![trigger_definition(
        "builtin-market",
        "builtin",
        "market-feed",
    )];
    let manifests = Vec::<PluginManifest>::new();
    let host_policy = policy(&plugin_root, &[], &[REQUIRED_TRIGGER_PLUGIN_CAPABILITY]);

    let mut plane_first = TriggerPlane::open(
        state_layout.clone(),
        definitions.clone(),
        manifests.clone(),
        host_policy.clone(),
        BTreeMap::from([(
            "builtin-market".to_string(),
            vec![
                builtin_event("event-a", "wf-a", "dedup-a", 1_000, "cooldown-a", 5_000),
                builtin_event("event-b", "wf-a", "dedup-a", 1_000, "cooldown-a", 5_000),
                builtin_event("event-c", "wf-a", "dedup-c", 1_000, "cooldown-a", 5_000),
            ],
        )]),
        1_710_100_010_000,
    )
    .expect("first trigger plane should open");
    let first_requests = plane_first
        .collect_run_requests(1_710_100_010_000)
        .expect("first dedup/cooldown pass should succeed");
    assert_eq!(first_requests.len(), 1);
    assert_eq!(first_requests[0].event_id, "event-a");

    let mut plane_second = TriggerPlane::open(
        state_layout.clone(),
        definitions.clone(),
        manifests.clone(),
        host_policy.clone(),
        BTreeMap::from([(
            "builtin-market".to_string(),
            vec![builtin_event(
                "event-d",
                "wf-a",
                "dedup-a",
                1_000,
                "cooldown-b",
                5_000,
            )],
        )]),
        1_710_100_010_200,
    )
    .expect("second trigger plane should open");
    let second_requests = plane_second
        .collect_run_requests(1_710_100_010_200)
        .expect("second dedup pass should succeed");
    assert!(second_requests.is_empty());

    let mut plane_third = TriggerPlane::open(
        state_layout.clone(),
        definitions.clone(),
        manifests.clone(),
        host_policy.clone(),
        BTreeMap::from([(
            "builtin-market".to_string(),
            vec![builtin_event(
                "event-e",
                "wf-a",
                "dedup-e",
                1_000,
                "cooldown-a",
                5_000,
            )],
        )]),
        1_710_100_012_000,
    )
    .expect("third trigger plane should open");
    let third_requests = plane_third
        .collect_run_requests(1_710_100_012_000)
        .expect("cooldown pass should succeed");
    assert!(third_requests.is_empty());

    let mut plane_fourth = TriggerPlane::open(
        state_layout,
        definitions,
        manifests,
        host_policy,
        BTreeMap::from([(
            "builtin-market".to_string(),
            vec![builtin_event(
                "event-f",
                "wf-a",
                "dedup-f",
                1_000,
                "cooldown-a",
                5_000,
            )],
        )]),
        1_710_100_016_000,
    )
    .expect("fourth trigger plane should open");
    let fourth_requests = plane_fourth
        .collect_run_requests(1_710_100_016_000)
        .expect("post-cooldown pass should succeed");
    assert_eq!(fourth_requests.len(), 1);
    assert_eq!(fourth_requests[0].event_id, "event-f");
}

#[test]
fn builtin_and_external_trigger_emit_run_requests() {
    let (state_layout, plugin_root) = unique_layout("builtin-and-external-emit-run-requests");
    let external_plugin_path = plugin_root.join("plugin-external.sh");
    write_executable_script(
        &external_plugin_path,
        "{\"api_version\":\"1.0.0\",\"events\":[{\"event_id\":\"event/ext\",\"workflow_id\":\"wf-external\",\"occurred_at_ms\":1710100020000,\"source\":\"plugin-source\",\"payload\":{\"side\":\"sell\"}}]}",
    );

    let definitions = vec![
        trigger_definition("builtin-trigger", "builtin", "market-feed"),
        trigger_definition("external-trigger", "external_plugin", "plugin-external"),
    ];
    let manifests = vec![plugin_manifest(
        "plugin-external",
        "1.0.0",
        "trigger",
        "plugin-external.sh",
        &[REQUIRED_TRIGGER_PLUGIN_CAPABILITY],
    )];
    let mut plane = TriggerPlane::open(
        state_layout,
        definitions,
        manifests,
        policy(
            &plugin_root,
            &["plugin-external"],
            &[REQUIRED_TRIGGER_PLUGIN_CAPABILITY],
        ),
        BTreeMap::from([(
            "builtin-trigger".to_string(),
            vec![TriggerEmission {
                event_id: "event-builtin".to_string(),
                workflow_id: "wf-builtin".to_string(),
                occurred_at_ms: 1_710_100_020_000,
                source: Some("builtin-source".to_string()),
                payload: serde_json::json!({"side": "buy"}),
                dedup_key: None,
                dedup_window_ms: None,
                cooldown_key: None,
                cooldown_ms: None,
            }],
        )]),
        1_710_100_020_010,
    )
    .expect("trigger plane should open for builtin+external flow");

    let requests = plane
        .collect_run_requests(1_710_100_020_020)
        .expect("builtin and external triggers should emit normalized run requests");
    assert_eq!(requests.len(), 2);

    let mut request_ids = requests
        .iter()
        .map(|request| request.trigger_id.clone())
        .collect::<Vec<_>>();
    request_ids.sort();
    assert_eq!(request_ids, vec!["builtin-trigger", "external-trigger"]);

    assert!(requests.iter().any(|request| {
        request.workflow_id == "wf-builtin" && request.event_id == "event-builtin"
    }));
    assert!(requests.iter().any(|request| {
        request.workflow_id == "wf-external" && request.event_id == "event/ext"
    }));
}

#[test]
fn trigger_records_are_file_backed() {
    let (state_layout, plugin_root) = unique_layout("trigger-records-are-file-backed");
    let definitions = vec![trigger_definition(
        "builtin-record",
        "builtin",
        "market-feed",
    )];
    let mut plane = TriggerPlane::open(
        state_layout.clone(),
        definitions,
        Vec::new(),
        policy(&plugin_root, &[], &[REQUIRED_TRIGGER_PLUGIN_CAPABILITY]),
        BTreeMap::from([(
            "builtin-record".to_string(),
            vec![TriggerEmission {
                event_id: "btc/usdt@1m".to_string(),
                workflow_id: "wf-record".to_string(),
                occurred_at_ms: 1_710_100_030_000,
                source: Some("builtin-feed".to_string()),
                payload: serde_json::json!({"price": 64000}),
                dedup_key: None,
                dedup_window_ms: None,
                cooldown_key: None,
                cooldown_ms: None,
            }],
        )]),
        1_710_100_030_010,
    )
    .expect("trigger plane should open for file-backed record validation");

    let requests = plane
        .collect_run_requests(1_710_100_030_020)
        .expect("builtin trigger record should be persisted");
    assert_eq!(requests.len(), 1);

    let request = &requests[0];
    assert!(request.trigger_record_path.exists());
    assert!(request
        .trigger_record_path
        .starts_with(&state_layout.trigger_records_dir));
    assert_eq!(
        request.trigger_record_path,
        state_layout.trigger_record_path(&request.run_id, 1, "builtin-record", "btc/usdt@1m")
    );

    let record: TriggerEventRecord = serde_json::from_str(
        &fs::read_to_string(&request.trigger_record_path)
            .expect("persisted trigger record should be readable"),
    )
    .expect("persisted trigger record should decode");
    assert_eq!(record.trigger_id, "builtin-record");
    assert_eq!(record.event_id, "btc/usdt@1m");
    assert_eq!(record.source, "builtin-feed");

    let file_name = request
        .trigger_record_path
        .file_name()
        .expect("trigger record path should have file name")
        .to_string_lossy()
        .to_string();
    assert!(file_name.contains("btc_usdt_1m"));
}

#[test]
fn builtin_trigger_kind_aliases_are_accepted() {
    let (state_layout, plugin_root) = unique_layout("builtin-trigger-kind-aliases");
    let definitions = vec![
        trigger_definition("manual-trigger", "manual", "manual-source"),
        trigger_definition("market-trigger", "market_tick", "market-feed"),
    ];

    let mut plane = TriggerPlane::open(
        state_layout,
        definitions,
        Vec::new(),
        policy(&plugin_root, &[], &[REQUIRED_TRIGGER_PLUGIN_CAPABILITY]),
        BTreeMap::from([
            (
                "manual-trigger".to_string(),
                vec![TriggerEmission {
                    event_id: "manual-event".to_string(),
                    workflow_id: "wf-manual".to_string(),
                    occurred_at_ms: 1_710_100_040_000,
                    source: Some("manual-source".to_string()),
                    payload: serde_json::json!({"kind": "manual"}),
                    dedup_key: None,
                    dedup_window_ms: None,
                    cooldown_key: None,
                    cooldown_ms: None,
                }],
            ),
            (
                "market-trigger".to_string(),
                vec![TriggerEmission {
                    event_id: "market-event".to_string(),
                    workflow_id: "wf-market".to_string(),
                    occurred_at_ms: 1_710_100_040_001,
                    source: Some("market-feed".to_string()),
                    payload: serde_json::json!({"kind": "market_tick"}),
                    dedup_key: None,
                    dedup_window_ms: None,
                    cooldown_key: None,
                    cooldown_ms: None,
                }],
            ),
        ]),
        1_710_100_040_100,
    )
    .expect("trigger plane should accept builtin trigger kind aliases");

    let requests = plane
        .collect_run_requests(1_710_100_040_110)
        .expect("builtin trigger kind aliases should emit run requests");

    assert_eq!(requests.len(), 2);
    assert!(requests.iter().any(
        |request| request.trigger_id == "manual-trigger" && request.event_id == "manual-event"
    ));
    assert!(requests.iter().any(
        |request| request.trigger_id == "market-trigger" && request.event_id == "market-event"
    ));
}

#[test]
fn trigger_coordination_rebuilds_from_file_records_after_restart() {
    let (state_layout, plugin_root) = unique_layout("trigger-coordination-rebuild-after-restart");
    let definitions = vec![trigger_definition(
        "builtin-market",
        "builtin",
        "market-feed",
    )];
    let policy = policy(&plugin_root, &[], &[REQUIRED_TRIGGER_PLUGIN_CAPABILITY]);

    let mut first_plane = TriggerPlane::open(
        state_layout.clone(),
        definitions.clone(),
        Vec::new(),
        policy.clone(),
        BTreeMap::from([(
            "builtin-market".to_string(),
            vec![builtin_event(
                "event-a",
                "wf-a",
                "dedup-shared",
                1_000,
                "cooldown-shared",
                5_000,
            )],
        )]),
        1_710_100_050_000,
    )
    .expect("first trigger plane should open");

    let first_requests = first_plane
        .collect_run_requests(1_710_100_050_000)
        .expect("first trigger request should persist a coordination-backed record");
    assert_eq!(first_requests.len(), 1);

    let sqlite = Connection::open(&state_layout.coordination_db_path)
        .expect("coordination database should be readable");
    sqlite
        .execute("DELETE FROM coordination_tokens", [])
        .expect("test should simulate lost sqlite coordination state");

    let mut reopened_plane = TriggerPlane::open(
        state_layout,
        definitions,
        Vec::new(),
        policy,
        BTreeMap::from([(
            "builtin-market".to_string(),
            vec![builtin_event(
                "event-b",
                "wf-a",
                "dedup-shared",
                1_000,
                "cooldown-shared",
                5_000,
            )],
        )]),
        1_710_100_050_200,
    )
    .expect("reopened trigger plane should rebuild coordination from trigger records");

    let reopened_requests = reopened_plane
        .collect_run_requests(1_710_100_050_200)
        .expect("rebuilt coordination should still suppress dedup/cooldown collisions");
    assert!(reopened_requests.is_empty());
}

fn trigger_definition(trigger_id: &str, kind: &str, source: &str) -> TriggerDefinition {
    TriggerDefinition {
        api_version: "1.0.0".to_string(),
        trigger_id: trigger_id.to_string(),
        kind: kind.to_string(),
        source: source.to_string(),
        enabled: true,
    }
}

fn plugin_manifest(
    plugin_id: &str,
    api_version: &str,
    kind: &str,
    executable: &str,
    capabilities: &[&str],
) -> PluginManifest {
    PluginManifest {
        api_version: api_version.to_string(),
        plugin_id: plugin_id.to_string(),
        kind: kind.to_string(),
        entrypoint: format!("plugins.{plugin_id}"),
        capabilities: capabilities.iter().map(|value| value.to_string()).collect(),
        executable: Some(executable.to_string()),
        input_schema: Vec::new(),
        output_schema: Vec::new(),
    }
}

fn policy(
    plugin_root: &Path,
    allowlisted_plugin_ids: &[&str],
    allowed_capabilities: &[&str],
) -> TriggerPluginHostPolicy {
    TriggerPluginHostPolicy {
        allowlisted_plugin_ids: allowlisted_plugin_ids
            .iter()
            .map(|value| value.to_string())
            .collect::<BTreeSet<_>>(),
        allowed_capabilities: allowed_capabilities
            .iter()
            .map(|value| value.to_string())
            .collect::<BTreeSet<_>>(),
        plugin_root_dir: plugin_root.to_path_buf(),
    }
}

fn builtin_event(
    event_id: &str,
    workflow_id: &str,
    dedup_key: &str,
    dedup_window_ms: i64,
    cooldown_key: &str,
    cooldown_ms: i64,
) -> TriggerEmission {
    TriggerEmission {
        event_id: event_id.to_string(),
        workflow_id: workflow_id.to_string(),
        occurred_at_ms: 1_710_100_010_000,
        source: Some("builtin-source".to_string()),
        payload: serde_json::json!({"event": event_id}),
        dedup_key: Some(dedup_key.to_string()),
        dedup_window_ms: Some(dedup_window_ms),
        cooldown_key: Some(cooldown_key.to_string()),
        cooldown_ms: Some(cooldown_ms),
    }
}

fn unique_layout(prefix: &str) -> (StateLayout, PathBuf) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after UNIX_EPOCH")
        .as_nanos();
    let root = workspace_root()
        .join("target")
        .join("test-roots")
        .join(format!("{prefix}-{now}"));
    let root_layout = RootLayout::from_root(root);

    fs::create_dir_all(&root_layout.plugins_dir)
        .expect("plugin directory should be creatable for trigger tests");
    fs::create_dir_all(&root_layout.state_dir)
        .expect("state directory should be creatable for trigger tests");

    (
        StateLayout::from_root_layout(&root_layout),
        root_layout.plugins_dir,
    )
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

fn write_executable_script(path: &Path, json_payload: &str) {
    let script = format!("#!/bin/sh\ncat <<'JSON'\n{json_payload}\nJSON\n");
    fs::write(path, script).expect("script fixture should be writable");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path)
            .expect("script fixture metadata should be readable")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).expect("script fixture should become executable");
    }
}
