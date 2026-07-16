//! [INPUT]
//! Trigger definitions, plugin manifests, synthetic trigger emissions, and temporary state roots.
//!
//! [OUTPUT]
//! Verifies trigger validation, accepted-event coordination, and external trigger host isolation.
//!
//! [ROLE]
//! Covers the trigger-plane boundary as an integration test.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use chainbot::builtins::triggers::build_builtin_trigger_emissions;
use chainbot::domain::state::{StagedTriggerEventRecord, TriggerEventRecord};
use chainbot::domain::trigger::{
    TriggerDefinition, TriggerEmission, TriggerHostMessage, TriggerPlane, TriggerPlaneError,
    TriggerPluginHostPolicy, TriggerStartCommand, REQUIRED_TRIGGER_PLUGIN_CAPABILITY,
};
use chainbot::errors::ContractError;
use chainbot::infrastructure::config::{RootLayout, RuntimeStorageBackend, RuntimeStorageConfig};
use chainbot::infrastructure::state::{RuntimeStateStore, StateLayout};
use chainbot::plugin::{
    ExternalTriggerRuntimeContract, PluginEventSchemaDescriptor, PluginManifest,
    TriggerDurableAckSemantics, TriggerHostErrorCategory, TriggerPushCallbackSemantics,
    TriggerRuntimeLifecycle,
};
use rusqlite::Connection;

#[test]
fn trigger_plugin_manifest_validation() {
    let (state_layout, plugin_root) = unique_layout("trigger-plugin-manifest-validation");
    let valid_plugin_path = plugin_root.join("valid-trigger.sh");
    write_executable_script(
        &valid_plugin_path,
        "{\"type\":\"heartbeat\",\"at_ms\":1710100000000}",
    );

    let definitions = vec![trigger_definition(
        "trigger-external",
        "external_plugin",
        "plugin-ok",
    )];
    let valid_manifest = plugin_manifest(
        "plugin-ok",
        "2.0.0",
        "external_trigger",
        "valid-trigger.sh",
        &[REQUIRED_TRIGGER_PLUGIN_CAPABILITY],
    );
    let valid_policy = policy(
        &plugin_root,
        &["plugin-ok"],
        &[REQUIRED_TRIGGER_PLUGIN_CAPABILITY],
    );

    let open_result = TriggerPlane::open_legacy_state_layout_for_tests(
        state_layout.clone(),
        definitions.clone(),
        vec![valid_manifest.clone()],
        valid_policy,
        BTreeMap::new(),
        1_710_100_000_000,
    );
    open_result.expect("valid trigger plugin manifest should pass host validation");

    let alias_kind_open_result = TriggerPlane::open_legacy_state_layout_for_tests(
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
    let alias_kind_error = alias_kind_open_result
        .expect_err("plugin alias kind should be rejected for trigger definitions");
    assert!(matches!(
        alias_kind_error,
        TriggerPlaneError::Contract(ContractError::UnknownTriggerKind { trigger_id, kind })
            if trigger_id == "trigger-external-alias" && kind == "plugin"
    ));

    let duplicate_trigger_error = TriggerPlane::open_legacy_state_layout_for_tests(
        state_layout.clone(),
        vec![
            trigger_definition("dup-trigger", "builtin", "market_tick"),
            trigger_definition("dup-trigger", "builtin", "market_tick"),
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

    let empty_source_error = TriggerPlane::open_legacy_state_layout_for_tests(
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

    let unknown_kind_error = TriggerPlane::open_legacy_state_layout_for_tests(
        state_layout.clone(),
        vec![trigger_definition(
            "bad-kind",
            "builtin_typo",
            "market_tick",
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
        "2.0.0",
        "external_trigger",
        "valid-trigger.sh",
        &["trigger.observe"],
    );
    let missing_capability_error = TriggerPlane::open_legacy_state_layout_for_tests(
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
        "2.0.0",
        "external_trigger",
        "../escape.sh",
        &[REQUIRED_TRIGGER_PLUGIN_CAPABILITY],
    );
    let escaping_error = TriggerPlane::open_legacy_state_layout_for_tests(
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
        TriggerPlaneError::Contract(ContractError::TriggerPluginEntrypointEscapesRoot {
            plugin_id,
            entrypoint,
            ..
        }) if plugin_id == "plugin-ok" && entrypoint == "../escape.sh"
    ));

    let unsupported_api_manifest = plugin_manifest(
        "plugin-ok",
        "4.0.0",
        "external_trigger",
        "valid-trigger.sh",
        &[REQUIRED_TRIGGER_PLUGIN_CAPABILITY],
    );
    let unsupported_api_error = TriggerPlane::open_legacy_state_layout_for_tests(
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
        TriggerPlaneError::Contract(ContractError::UnsupportedMajorVersion {
            field: "plugin.manifest_version",
            major: 4,
            supported_major: 2,
        })
    ));

    let wasm_manifest = PluginManifest {
        api_version: "2.0.0".to_owned(),
        plugin_id: "plugin-ok".to_owned(),
        kind: "external_trigger".to_owned(),
        entrypoint: "plugins.plugin-ok".to_owned(),
        capabilities: vec![REQUIRED_TRIGGER_PLUGIN_CAPABILITY.to_owned()],
        executable: None,
        input_schema: Vec::new(),
        output_schema: Vec::new(),
        trigger_runtime: Some(ExternalTriggerRuntimeContract {
            lifecycle: Some(TriggerRuntimeLifecycle::WasmDaemonPersistentSession),
            push_callback: Some(TriggerPushCallbackSemantics::HostCallback),
            durable_ack: Some(TriggerDurableAckSemantics::AfterStorePersist),
            host_error_categories: vec![
                TriggerHostErrorCategory::Transport,
                TriggerHostErrorCategory::ProtocolContract,
                TriggerHostErrorCategory::PluginFatal,
            ],
            module: Some("bin/external_trigger.wasm".to_owned()),
            abi: None,
        }),
        operations: Vec::new(),
        event_schema: Some(PluginEventSchemaDescriptor {
            summary: Some("External trigger payload".to_owned()),
            fields: vec!["symbol".to_owned(), "price".to_owned()],
            ..PluginEventSchemaDescriptor::default()
        }),
        activation: None,
        mcp: None,
        manifest_path: plugin_root.join("plugin-ok").join("config.toml"),
    };
    TriggerPlane::open_legacy_state_layout_for_tests(
        unique_layout("trigger-plugin-wasm-manifest").0,
        vec![trigger_definition(
            "trigger-external-wasm",
            "external_plugin",
            "plugin-ok",
        )],
        vec![wasm_manifest],
        policy(
            &plugin_root,
            &["plugin-ok"],
            &[REQUIRED_TRIGGER_PLUGIN_CAPABILITY],
        ),
        BTreeMap::new(),
        1_710_100_000_010,
    )
    .expect(
        "wasm external trigger manifest without executable should pass runtime host validation",
    );
}

#[test]
fn trigger_dedup_and_cooldown() {
    let (state_layout, plugin_root) = unique_layout("trigger-dedup-and-cooldown");
    let definitions = vec![trigger_definition(
        "builtin-market",
        "builtin",
        "market_tick",
    )];
    let manifests = Vec::<PluginManifest>::new();
    let host_policy = policy(&plugin_root, &[], &[REQUIRED_TRIGGER_PLUGIN_CAPABILITY]);

    let mut plane_first = TriggerPlane::open_legacy_state_layout_for_tests(
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

    let mut plane_second = TriggerPlane::open_legacy_state_layout_for_tests(
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

    let mut plane_third = TriggerPlane::open_legacy_state_layout_for_tests(
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

    let mut plane_fourth = TriggerPlane::open_legacy_state_layout_for_tests(
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
fn trigger_sequence_continues_from_snapshot_after_restart() {
    let (state_layout, plugin_root) = unique_layout("trigger-sequence-continues-from-snapshot");
    let definitions = vec![trigger_definition(
        "builtin-market",
        "builtin",
        "market_tick",
    )];
    let manifests = Vec::<PluginManifest>::new();
    let host_policy = policy(&plugin_root, &[], &[REQUIRED_TRIGGER_PLUGIN_CAPABILITY]);

    let mut first_plane = TriggerPlane::open_legacy_state_layout_for_tests(
        state_layout.clone(),
        definitions.clone(),
        manifests.clone(),
        host_policy.clone(),
        BTreeMap::from([(
            "builtin-market".to_string(),
            vec![builtin_event(
                "event-a",
                "wf-a",
                "dedup-a",
                1_000,
                "cooldown-a",
                5_000,
            )],
        )]),
        1_710_100_020_000,
    )
    .expect("first trigger plane should open");
    let first_requests = first_plane
        .collect_run_requests(1_710_100_020_000)
        .expect("first trigger pass should succeed");
    assert_eq!(first_requests.len(), 1);
    assert!(first_requests[0].run_id.ends_with("00000000000000000001"));

    let mut second_plane = TriggerPlane::open_legacy_state_layout_for_tests(
        state_layout,
        definitions,
        manifests,
        host_policy,
        BTreeMap::from([(
            "builtin-market".to_string(),
            vec![builtin_event(
                "event-b",
                "wf-a",
                "dedup-b",
                1_000,
                "cooldown-b",
                5_000,
            )],
        )]),
        1_710_100_021_000,
    )
    .expect("second trigger plane should open from persisted snapshot");
    let second_requests = second_plane
        .collect_run_requests(1_710_100_021_000)
        .expect("second trigger pass should succeed");
    assert_eq!(second_requests.len(), 1);
    assert!(second_requests[0].run_id.ends_with("00000000000000000002"));
}

#[test]
fn builtin_and_external_trigger_emit_run_requests() {
    let (state_layout, plugin_root) = unique_layout("builtin-and-external-emit-run-requests");
    let external_plugin_path = plugin_root.join("plugin-external.sh");
    write_executable_script(
        &external_plugin_path,
        "{\"type\":\"event\",\"checkpoint\":\"cp-ext\",\"event_key\":\"event/ext\",\"occurred_at_ms\":1710100020000,\"payload\":{\"side\":\"sell\"}}",
    );

    let definitions = vec![
        trigger_definition("builtin-trigger", "builtin", "market_tick"),
        trigger_definition("external-trigger", "external_plugin", "plugin-external"),
    ];
    let manifests = vec![plugin_manifest(
        "plugin-external",
        "2.0.0",
        "external_trigger",
        "plugin-external.sh",
        &[REQUIRED_TRIGGER_PLUGIN_CAPABILITY],
    )];
    let mut plane = TriggerPlane::open_legacy_state_layout_for_tests(
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
                occurred_at_ms: 1_710_100_020_000,
                checkpoint: None,
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
        request.workflow_id == "wf-external" && request.event_id == "external-trigger:event/ext"
    }));
}

#[test]
fn trigger_records_are_db_backed() {
    let (state_layout, plugin_root) = unique_layout("trigger-records-are-file-backed");
    let definitions = vec![trigger_definition(
        "builtin-record",
        "builtin",
        "market_tick",
    )];
    let mut plane = TriggerPlane::open_legacy_state_layout_for_tests(
        state_layout.clone(),
        definitions,
        Vec::new(),
        policy(&plugin_root, &[], &[REQUIRED_TRIGGER_PLUGIN_CAPABILITY]),
        BTreeMap::from([(
            "builtin-record".to_string(),
            vec![TriggerEmission {
                event_id: "btc/usdt@1m".to_string(),
                occurred_at_ms: 1_710_100_030_000,
                checkpoint: None,
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
    .expect("trigger plane should open for DB-backed record validation");

    let requests = plane
        .collect_run_requests(1_710_100_030_020)
        .expect("builtin trigger record should be persisted");
    assert_eq!(requests.len(), 1);

    let request = &requests[0];
    assert_eq!(
        request.trigger_record_ref,
        "db://trigger_event_records/builtin-record/1"
    );
    let sqlite = Connection::open(&state_layout.coordination_db_path)
        .expect("runtime sqlite database should be readable");
    let record = sqlite
        .query_row(
            "SELECT trigger_id, event_id, source FROM trigger_event_records WHERE trigger_id = ?1 AND sequence = ?2",
            rusqlite::params!["builtin-record", 1_i64],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .expect("persisted trigger record row should load");
    assert_eq!(record.0, "builtin-record");
    assert_eq!(record.1, "btc/usdt@1m");
    assert_eq!(record.2, "builtin-feed");
}

#[test]
fn builtin_trigger_kind_aliases_are_accepted() {
    let (state_layout, plugin_root) = unique_layout("builtin-trigger-kind-aliases");
    let definitions = vec![
        trigger_definition("manual-trigger", "builtin", "manual"),
        trigger_definition("market-trigger", "builtin", "market_tick"),
    ];

    let mut plane = TriggerPlane::open_legacy_state_layout_for_tests(
        state_layout,
        definitions,
        Vec::new(),
        policy(&plugin_root, &[], &[REQUIRED_TRIGGER_PLUGIN_CAPABILITY]),
        BTreeMap::from([
            (
                "manual-trigger".to_string(),
                vec![TriggerEmission {
                    event_id: "manual-event".to_string(),
                    occurred_at_ms: 1_710_100_040_000,
                    checkpoint: None,
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
                    occurred_at_ms: 1_710_100_040_001,
                    checkpoint: None,
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
    .expect("trigger plane should accept canonical builtin trigger kinds");

    let requests = plane
        .collect_run_requests(1_710_100_040_110)
        .expect("canonical builtin trigger kinds should emit run requests");

    assert_eq!(requests.len(), 2);
    assert!(requests.iter().any(
        |request| request.trigger_id == "manual-trigger" && request.event_id == "manual-event"
    ));
    assert!(requests.iter().any(
        |request| request.trigger_id == "market-trigger" && request.event_id == "market-event"
    ));
}

#[test]
fn builtin_trigger_fanout_emits_multiple_run_requests_end_to_end() {
    let (state_layout, plugin_root) = unique_layout("builtin-trigger-fanout-end-to-end");
    let definitions = vec![
        trigger_definition("manual-trigger-a", "builtin", "manual"),
        trigger_definition("manual-trigger-b", "builtin", "manual"),
    ];
    let builtin_events = build_builtin_trigger_emissions(&definitions, 1_710_100_041_000)
        .expect("builtin trigger emission generation should succeed");

    let mut plane = TriggerPlane::open_legacy_state_layout_for_tests(
        state_layout,
        definitions,
        Vec::new(),
        policy(&plugin_root, &[], &[REQUIRED_TRIGGER_PLUGIN_CAPABILITY]),
        builtin_events,
        1_710_100_041_050,
    )
    .expect("trigger plane should open for builtin fanout scenario");

    let requests = plane
        .collect_run_requests(1_710_100_041_060)
        .expect("builtin fanout events should normalize into run requests");

    assert_eq!(requests.len(), 2);
    assert!(requests.iter().any(|request| {
        request.trigger_id == "manual-trigger-a"
            && request.event_id == "builtin-event-manual-trigger-a"
            && request.workflow_id == "wf-test"
    }));
    assert!(requests.iter().any(|request| {
        request.trigger_id == "manual-trigger-b"
            && request.event_id == "builtin-event-manual-trigger-b"
            && request.workflow_id == "wf-test"
    }));
}

#[test]
fn builtin_trigger_generation_preserves_alias_payload_contract() {
    let definitions = vec![
        trigger_definition("manual-trigger", "builtin", "manual"),
        trigger_definition("market-trigger", "builtin", "market_tick"),
    ];

    let emissions = build_builtin_trigger_emissions(&definitions, 1_710_100_041_500)
        .expect("builtin trigger emission generation should support alias and builtin forms");

    assert_eq!(emissions["manual-trigger"].len(), 1);
    assert_eq!(
        emissions["manual-trigger"][0].payload,
        serde_json::json!({"kind": "manual", "source": "manual"})
    );
    assert_eq!(emissions["market-trigger"].len(), 1);
    assert_eq!(
        emissions["market-trigger"][0].payload,
        serde_json::json!({"kind": "market_tick", "source": "market_tick", "symbol": "BTCUSDT"})
    );
}

#[test]
fn builtin_cron_trigger_uses_params_and_restarts_without_duplicate_events() {
    let (state_layout, plugin_root) = unique_layout("builtin-cron-trigger-restart-safe");
    let mut definition = trigger_definition("cron-trigger", "builtin", "cron");
    definition.params =
        BTreeMap::from([(String::from("schedule"), serde_json::json!("*/15 * * * *"))]);

    let accepted_at_ms = 1_736_172_900_123;
    let builtin_events = build_builtin_trigger_emissions(&[definition.clone()], accepted_at_ms)
        .expect("cron builtin events should build from params");
    let mut first_plane = TriggerPlane::open_legacy_state_layout_for_tests(
        state_layout.clone(),
        vec![definition.clone()],
        Vec::new(),
        policy(&plugin_root, &[], &[REQUIRED_TRIGGER_PLUGIN_CAPABILITY]),
        builtin_events,
        accepted_at_ms,
    )
    .expect("trigger plane should open for cron trigger");

    let first_requests = first_plane
        .collect_run_requests(accepted_at_ms)
        .expect("cron trigger should emit one accepted request for due slot");
    assert_eq!(first_requests.len(), 1);
    assert_eq!(
        first_requests[0].event_id,
        "cron:cron-trigger:1736172900000"
    );
    assert_eq!(
        first_requests[0].payload,
        serde_json::json!({
            "kind": "cron",
            "source": "cron",
            "schedule": "*/15 * * * *",
            "slot_start_ms": 1736172900000_i64,
            "timezone": "UTC"
        })
    );

    let repeat_events = build_builtin_trigger_emissions(&[definition.clone()], accepted_at_ms)
        .expect("same-slot cron builtin events should still build");
    let mut reopened_plane = TriggerPlane::open_legacy_state_layout_for_tests(
        state_layout,
        vec![definition],
        Vec::new(),
        policy(&plugin_root, &[], &[REQUIRED_TRIGGER_PLUGIN_CAPABILITY]),
        repeat_events,
        accepted_at_ms,
    )
    .expect("reopened trigger plane should open for duplicate suppression");

    let repeated_requests = reopened_plane
        .collect_run_requests(accepted_at_ms)
        .expect("same cron slot should be suppressed after restart");
    assert!(repeated_requests.is_empty());
}

#[test]
fn builtin_cron_trigger_requires_schedule_param() {
    let (state_layout, plugin_root) = unique_layout("builtin-cron-trigger-validation");
    let definition = trigger_definition("cron-trigger-invalid", "builtin", "cron");

    let error = TriggerPlane::open_legacy_state_layout_for_tests(
        state_layout,
        vec![definition],
        Vec::new(),
        policy(&plugin_root, &[], &[REQUIRED_TRIGGER_PLUGIN_CAPABILITY]),
        BTreeMap::new(),
        1_736_172_900_123,
    )
    .expect_err("cron trigger without schedule params should fail validation");

    assert!(matches!(
        error,
        TriggerPlaneError::Contract(ContractError::InvalidTriggerDefinitionField {
            trigger_id,
            field: "trigger.params",
            ..
        }) if trigger_id == "cron-trigger-invalid"
    ));
}

#[test]
fn trigger_dedup_and_cooldown_persist_from_db_records_after_restart() {
    let (state_layout, plugin_root) = unique_layout("trigger-coordination-rebuild-after-restart");
    let definitions = vec![trigger_definition(
        "builtin-market",
        "builtin",
        "market_tick",
    )];
    let policy = policy(&plugin_root, &[], &[REQUIRED_TRIGGER_PLUGIN_CAPABILITY]);

    let mut first_plane = TriggerPlane::open_legacy_state_layout_for_tests(
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
        .expect("first trigger request should persist a DB-backed record");
    assert_eq!(first_requests.len(), 1);

    let mut reopened_plane = TriggerPlane::open_legacy_state_layout_for_tests(
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
    .expect("reopened trigger plane should keep dedup/cooldown suppression from DB records");

    let reopened_requests = reopened_plane
        .collect_run_requests(1_710_100_050_200)
        .expect("DB-backed dedup/cooldown should still suppress collisions");
    assert!(reopened_requests.is_empty());
}

#[test]
fn trigger_plugin_host_uses_default_deny_environment() {
    let probe_key = select_non_allowlisted_host_env_key();
    let (state_layout, plugin_root) = unique_layout("trigger-plugin-default-deny-env");
    let marker_path = plugin_root.join("env-leak.marker");
    let executable = plugin_root.join("plugin-env-probe.sh");
    write_trigger_env_probe_script(&executable, &probe_key, &marker_path);

    let definitions = vec![trigger_definition(
        "trigger-external",
        "external_plugin",
        "plugin-env-probe",
    )];
    let manifests = vec![plugin_manifest(
        "plugin-env-probe",
        "2.0.0",
        "external_trigger",
        "plugin-env-probe.sh",
        &[REQUIRED_TRIGGER_PLUGIN_CAPABILITY],
    )];

    let mut plane = TriggerPlane::open_legacy_state_layout_for_tests(
        state_layout,
        definitions,
        manifests,
        policy(
            &plugin_root,
            &["plugin-env-probe"],
            &[REQUIRED_TRIGGER_PLUGIN_CAPABILITY],
        ),
        BTreeMap::new(),
        1_710_100_060_000,
    )
    .expect("trigger plane should open under default-deny trigger plugin host env");

    let requests = plane
        .collect_run_requests(1_710_100_060_010)
        .expect("external trigger plugin should execute under default-deny env");
    assert_eq!(requests.len(), 1);
    assert!(
        !marker_path.exists(),
        "trigger plugin inherited unexpected host environment variable {probe_key}"
    );
}

#[test]
fn external_trigger_plugin_receives_params_via_stdin_protocol() {
    let (state_layout, plugin_root) = unique_layout("trigger-plugin-stdin-params");
    let input_capture_path = plugin_root.join("plugin-input.json");
    let executable = plugin_root.join("plugin-params-probe.sh");
    write_trigger_input_capture_script(&executable, &input_capture_path);

    let mut definition = trigger_definition("external-trigger", "external_plugin", "plugin-params");
    definition.params = BTreeMap::from([
        (String::from("region"), serde_json::json!("apac")),
        (String::from("threshold"), serde_json::json!(12)),
    ]);
    let manifests = vec![plugin_manifest(
        "plugin-params",
        "2.0.0",
        "external_trigger",
        "plugin-params-probe.sh",
        &[REQUIRED_TRIGGER_PLUGIN_CAPABILITY],
    )];

    let mut plane = TriggerPlane::open_legacy_state_layout_for_tests(
        state_layout,
        vec![definition],
        manifests,
        policy(
            &plugin_root,
            &["plugin-params"],
            &[REQUIRED_TRIGGER_PLUGIN_CAPABILITY],
        ),
        BTreeMap::new(),
        1_710_100_070_000,
    )
    .expect("trigger plane should open for params-aware external plugin");

    let requests = plane
        .collect_run_requests(1_710_100_070_010)
        .expect("external trigger plugin should receive params and emit a request");
    assert_eq!(requests.len(), 1);

    let captured_input: TriggerHostMessage = serde_json::from_str(
        &fs::read_to_string(&input_capture_path)
            .expect("captured trigger plugin input should be readable"),
    )
    .expect("captured trigger plugin input should decode");
    let TriggerHostMessage::Start(TriggerStartCommand {
        trigger_id,
        source,
        params,
        ..
    }) = captured_input
    else {
        panic!("expected start message");
    };
    assert_eq!(trigger_id, "external-trigger");
    assert_eq!(source, "plugin-params");
    assert_eq!(params["region"], serde_json::json!("apac"));
    assert_eq!(params["threshold"], serde_json::json!(12));
}

#[test]
fn external_trigger_plugin_rejects_event_before_ready() {
    let (state_layout, plugin_root) = unique_layout("trigger-plugin-event-before-ready");
    let executable = plugin_root.join("plugin-bad-order.sh");
    write_protocol_script(
        &executable,
        "printf '%s\n' '{\"type\":\"event\",\"checkpoint\":\"cp-bad\",\"event_key\":\"bad-order\",\"occurred_at_ms\":1710100080000,\"payload\":{\"kind\":\"probe\"}}'\nprintf '%s\n' '{\"type\":\"ready\",\"protocol_version\":\"2.0.0\"}'\n",
    );

    let manifests = vec![plugin_manifest(
        "plugin-bad-order",
        "2.0.0",
        "external_trigger",
        "plugin-bad-order.sh",
        &[REQUIRED_TRIGGER_PLUGIN_CAPABILITY],
    )];
    let definitions = vec![trigger_definition(
        "external-trigger",
        "external_plugin",
        "plugin-bad-order",
    )];

    let mut plane = TriggerPlane::open_legacy_state_layout_for_tests(
        state_layout,
        definitions,
        manifests,
        policy(
            &plugin_root,
            &["plugin-bad-order"],
            &[REQUIRED_TRIGGER_PLUGIN_CAPABILITY],
        ),
        BTreeMap::new(),
        1_710_100_080_000,
    )
    .expect("trigger plane should open for protocol-order validation");

    let error = plane
        .collect_run_requests(1_710_100_080_010)
        .expect_err("event before ready should fail");
    assert!(matches!(
        error,
        TriggerPlaneError::Contract(ContractError::TriggerPluginProtocolContractViolation { .. })
    ));
}

#[test]
fn staged_external_rows_bridge_through_trigger_plane_acceptance_path() {
    let (state_layout, plugin_root) = unique_layout("staged-external-bridge-trigger-plane");
    let executable = plugin_root.join("plugin-ready-only.sh");
    write_protocol_script(
        &executable,
        "printf '%s\n' '{\"type\":\"ready\",\"protocol_version\":\"2.0.0\"}'\n",
    );

    let mut definitions = vec![trigger_definition(
        "external-trigger",
        "external_plugin",
        "plugin-ready-only",
    )];
    definitions[0]
        .input_mapping
        .insert(String::from("symbol"), String::from("payload.quote.symbol"));
    let manifests = vec![plugin_manifest(
        "plugin-ready-only",
        "2.0.0",
        "external_trigger",
        "plugin-ready-only.sh",
        &[REQUIRED_TRIGGER_PLUGIN_CAPABILITY],
    )];

    let mut state_store = open_runtime_store(&state_layout, 1_710_100_090_000);
    state_store
        .append_staged_trigger_event_record(&StagedTriggerEventRecord {
            schema_version: String::from("1.0.0"),
            staging_id: String::from("staging-bridge-1"),
            trigger_id: String::from("external-trigger"),
            workflow_id: String::from("wf-external"),
            event_id: String::from("external-trigger:event-staged"),
            source: String::from("plugin-ready-only"),
            occurred_at_ms: 1_710_100_090_000,
            staged_at_ms: 1_710_100_090_001,
            checkpoint: Some(String::from("cp-staged")),
            payload: serde_json::json!({"quote": {"symbol": "BTCUSDT"}}),
            dedup_key: None,
            dedup_window_ms: None,
            cooldown_key: None,
            cooldown_ms: None,
            accepted_at_ms: None,
            last_error: None,
        })
        .expect("staged external row should persist before trigger-plane collection");
    drop(state_store);

    let mut plane = TriggerPlane::open_legacy_state_layout_for_tests(
        state_layout.clone(),
        definitions,
        manifests,
        policy(
            &plugin_root,
            &["plugin-ready-only"],
            &[REQUIRED_TRIGGER_PLUGIN_CAPABILITY],
        ),
        BTreeMap::new(),
        1_710_100_090_010,
    )
    .expect("trigger plane should open for staged external bridge scenario");

    let requests = plane
        .collect_run_requests(1_710_100_090_020)
        .expect("staged external row should normalize through trigger acceptance path");
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].event_id, "external-trigger:event-staged");
    assert_eq!(
        requests[0].payload,
        serde_json::json!({"symbol": "BTCUSDT"}),
        "staged external payload should be mapped exactly once at acceptance"
    );

    let mut verify_store = open_runtime_store(&state_layout, 1_710_100_090_030);
    let pending = verify_store
        .list_pending_staged_trigger_event_records("external-trigger", 10)
        .expect("pending staged rows should be queryable after bridge");
    assert!(pending.is_empty());
}

#[test]
fn staged_duplicate_event_does_not_create_second_accepted_record() {
    let (state_layout, plugin_root) = unique_layout("staged-external-no-double-accept");
    let executable = plugin_root.join("plugin-ready-only.sh");
    write_protocol_script(
        &executable,
        "printf '%s\n' '{\"type\":\"ready\",\"protocol_version\":\"2.0.0\"}'\n",
    );

    let definitions = vec![trigger_definition(
        "external-trigger",
        "external_plugin",
        "plugin-ready-only",
    )];
    let manifests = vec![plugin_manifest(
        "plugin-ready-only",
        "2.0.0",
        "external_trigger",
        "plugin-ready-only.sh",
        &[REQUIRED_TRIGGER_PLUGIN_CAPABILITY],
    )];

    let mut state_store = open_runtime_store(&state_layout, 1_710_100_091_000);
    state_store
        .write_trigger_record(&TriggerEventRecord {
            schema_version: String::from("1.0.0"),
            run_id: String::from("run-preaccepted"),
            sequence: 1,
            trigger_id: String::from("external-trigger"),
            workflow_id: String::from("wf-external"),
            event_id: String::from("external-trigger:event-dupe"),
            checkpoint: Some(String::from("cp-dupe")),
            source: String::from("plugin-ready-only"),
            accepted_at_ms: 1_710_100_091_000,
            payload: serde_json::json!({"side": "sell"}),
            dedup_key: None,
            dedup_expires_at_ms: None,
            cooldown_key: None,
            cooldown_expires_at_ms: None,
        })
        .expect("pre-accepted trigger record should persist");
    state_store
        .append_staged_trigger_event_record(&StagedTriggerEventRecord {
            schema_version: String::from("1.0.0"),
            staging_id: String::from("staging-dupe-1"),
            trigger_id: String::from("external-trigger"),
            workflow_id: String::from("wf-external"),
            event_id: String::from("external-trigger:event-dupe"),
            source: String::from("plugin-ready-only"),
            occurred_at_ms: 1_710_100_091_010,
            staged_at_ms: 1_710_100_091_011,
            checkpoint: Some(String::from("cp-dupe")),
            payload: serde_json::json!({"side": "sell"}),
            dedup_key: None,
            dedup_window_ms: None,
            cooldown_key: None,
            cooldown_ms: None,
            accepted_at_ms: None,
            last_error: None,
        })
        .expect("duplicate staged row should persist for replay suppression coverage");
    drop(state_store);

    let mut plane = TriggerPlane::open_legacy_state_layout_for_tests(
        state_layout.clone(),
        definitions,
        manifests,
        policy(
            &plugin_root,
            &["plugin-ready-only"],
            &[REQUIRED_TRIGGER_PLUGIN_CAPABILITY],
        ),
        BTreeMap::new(),
        1_710_100_091_020,
    )
    .expect("trigger plane should open for duplicate staged replay scenario");

    let requests = plane
        .collect_run_requests(1_710_100_091_030)
        .expect("duplicate staged event replay should not fail");
    assert!(requests.is_empty());

    let mut verify_store = open_runtime_store(&state_layout, 1_710_100_091_040);
    let pending = verify_store
        .list_pending_staged_trigger_event_records("external-trigger", 10)
        .expect("pending staged rows should be queryable after duplicate replay");
    assert!(pending.is_empty());

    let sqlite = Connection::open(&state_layout.coordination_db_path)
        .expect("runtime sqlite database should be readable for duplicate assertion");
    let accepted_count: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM trigger_event_records WHERE trigger_id = ?1 AND event_id = ?2",
            rusqlite::params!["external-trigger", "external-trigger:event-dupe"],
            |row| row.get(0),
        )
        .expect("accepted trigger record count should load");
    assert_eq!(accepted_count, 1);
}

#[test]
fn disabled_or_removed_trigger_does_not_accept_future_staged_rows_after_restart() {
    let (state_layout, plugin_root) = unique_layout("staged-events-disable-remove-restart-safe");
    let definitions = vec![trigger_definition(
        "external-trigger",
        "external_plugin",
        "plugin-ready-only",
    )];
    let manifests = vec![plugin_manifest(
        "plugin-ready-only",
        "2.0.0",
        "external_trigger",
        "plugin-ready-only.sh",
        &[REQUIRED_TRIGGER_PLUGIN_CAPABILITY],
    )];

    let executable = plugin_root.join("plugin-ready-only.sh");
    write_protocol_script(
        &executable,
        "printf '%s\n' '{\"type\":\"ready\",\"protocol_version\":\"2.0.0\"}'\n",
    );

    let mut state_store = open_runtime_store(&state_layout, 1_710_100_093_000);
    state_store
        .append_staged_trigger_event_record(&StagedTriggerEventRecord {
            schema_version: String::from("1.0.0"),
            staging_id: String::from("staging-disable-remove-1"),
            trigger_id: String::from("external-trigger"),
            workflow_id: String::from("wf-external"),
            event_id: String::from("external-trigger:event-before-disable"),
            source: String::from("plugin-ready-only"),
            occurred_at_ms: 1_710_100_093_000,
            staged_at_ms: 1_710_100_093_001,
            checkpoint: Some(String::from("cp-before-disable")),
            payload: serde_json::json!({"side": "buy"}),
            dedup_key: None,
            dedup_window_ms: None,
            cooldown_key: None,
            cooldown_ms: None,
            accepted_at_ms: None,
            last_error: None,
        })
        .expect("first staged row should persist before initial acceptance");
    drop(state_store);

    let first_store = open_runtime_store(&state_layout, 1_710_100_093_010);
    let mut enabled_plane = TriggerPlane::open_with_store_acceptance_only(
        first_store,
        definitions.clone(),
        manifests.clone(),
        policy(
            &plugin_root,
            &["plugin-ready-only"],
            &[REQUIRED_TRIGGER_PLUGIN_CAPABILITY],
        ),
        BTreeMap::new(),
    )
    .expect("acceptance-only trigger plane should open for enabled baseline");
    let first_requests = enabled_plane
        .collect_run_requests(1_710_100_093_020)
        .expect("enabled trigger should accept staged row exactly once");
    assert_eq!(first_requests.len(), 1);
    assert_eq!(
        first_requests[0].event_id,
        "external-trigger:event-before-disable"
    );

    let mut second_store = open_runtime_store(&state_layout, 1_710_100_093_030);
    second_store
        .append_staged_trigger_event_record(&StagedTriggerEventRecord {
            schema_version: String::from("1.0.0"),
            staging_id: String::from("staging-disable-remove-2"),
            trigger_id: String::from("external-trigger"),
            workflow_id: String::from("wf-external"),
            event_id: String::from("external-trigger:event-after-disable"),
            source: String::from("plugin-ready-only"),
            occurred_at_ms: 1_710_100_093_030,
            staged_at_ms: 1_710_100_093_031,
            checkpoint: Some(String::from("cp-after-disable")),
            payload: serde_json::json!({"side": "sell"}),
            dedup_key: None,
            dedup_window_ms: None,
            cooldown_key: None,
            cooldown_ms: None,
            accepted_at_ms: None,
            last_error: None,
        })
        .expect("second staged row should persist for disable/remove replay checks");
    drop(second_store);

    let mut disabled_definition =
        trigger_definition("external-trigger", "external_plugin", "plugin-ready-only");
    disabled_definition.enabled = false;
    let disabled_store = open_runtime_store(&state_layout, 1_710_100_093_040);
    let mut disabled_plane = TriggerPlane::open_with_store_acceptance_only(
        disabled_store,
        vec![disabled_definition],
        manifests.clone(),
        policy(
            &plugin_root,
            &["plugin-ready-only"],
            &[REQUIRED_TRIGGER_PLUGIN_CAPABILITY],
        ),
        BTreeMap::new(),
    )
    .expect("acceptance-only trigger plane should open for disabled trigger state");
    let disabled_requests = disabled_plane
        .collect_run_requests(1_710_100_093_050)
        .expect("disabled trigger should not accept staged rows");
    assert!(disabled_requests.is_empty());

    let removed_store = open_runtime_store(&state_layout, 1_710_100_093_060);
    let mut removed_plane = TriggerPlane::open_with_store_acceptance_only(
        removed_store,
        Vec::new(),
        manifests,
        policy(
            &plugin_root,
            &["plugin-ready-only"],
            &[REQUIRED_TRIGGER_PLUGIN_CAPABILITY],
        ),
        BTreeMap::new(),
    )
    .expect("acceptance-only trigger plane should open after trigger removal");
    let removed_requests = removed_plane
        .collect_run_requests(1_710_100_093_070)
        .expect("removed trigger should not accept stale staged rows");
    assert!(removed_requests.is_empty());

    let mut verify_store = open_runtime_store(&state_layout, 1_710_100_093_080);
    let pending = verify_store
        .list_pending_staged_trigger_event_records("external-trigger", 10)
        .expect("pending staged rows should remain queryable after disable/remove replay");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].event_id, "external-trigger:event-after-disable");

    let sqlite = Connection::open(&state_layout.coordination_db_path)
        .expect("runtime sqlite database should be readable for accepted-count assertion");
    let accepted_count: i64 = sqlite
        .query_row(
            "SELECT COUNT(*) FROM trigger_event_records WHERE trigger_id = ?1",
            rusqlite::params!["external-trigger"],
            |row| row.get(0),
        )
        .expect("accepted trigger record count should load after disable/remove replay");
    assert_eq!(accepted_count, 1);
}

#[test]
fn acceptance_only_trigger_plane_accepts_known_staged_rows_and_keeps_unknown_rows_pending() {
    let (state_layout, plugin_root) = unique_layout("acceptance-only-known-vs-unknown-staged");
    let executable = plugin_root.join("plugin-ready-only.sh");
    write_protocol_script(
        &executable,
        "printf '%s\n' '{\"type\":\"ready\",\"protocol_version\":\"2.0.0\"}'\n",
    );

    let definitions = vec![
        trigger_definition("external-trigger-a", "external_plugin", "plugin-ready-only"),
        trigger_definition("external-trigger-b", "external_plugin", "plugin-ready-only"),
    ];
    let manifests = vec![plugin_manifest(
        "plugin-ready-only",
        "2.0.0",
        "external_trigger",
        "plugin-ready-only.sh",
        &[REQUIRED_TRIGGER_PLUGIN_CAPABILITY],
    )];

    let mut state_store = open_runtime_store(&state_layout, 1_710_100_094_000);
    for (staging_id, trigger_id, workflow_id, event_id, staged_at_ms) in [
        (
            "staging-known-1",
            "external-trigger-a",
            "wf-test",
            "external-trigger-a:event-1",
            1_710_100_094_001,
        ),
        (
            "staging-known-2",
            "external-trigger-b",
            "wf-test",
            "external-trigger-b:event-1",
            1_710_100_094_002,
        ),
        (
            "staging-unknown-1",
            "external-trigger-unknown",
            "wf-missing",
            "external-trigger-unknown:event-1",
            1_710_100_094_003,
        ),
    ] {
        state_store
            .append_staged_trigger_event_record(&StagedTriggerEventRecord {
                schema_version: String::from("1.0.0"),
                staging_id: String::from(staging_id),
                trigger_id: String::from(trigger_id),
                workflow_id: String::from(workflow_id),
                event_id: String::from(event_id),
                source: String::from("plugin-ready-only"),
                occurred_at_ms: 1_710_100_094_000,
                staged_at_ms,
                checkpoint: Some(String::from("cp-staged")),
                payload: serde_json::json!({"kind": "matrix"}),
                dedup_key: None,
                dedup_window_ms: None,
                cooldown_key: None,
                cooldown_ms: None,
                accepted_at_ms: None,
                last_error: None,
            })
            .expect("staged row should persist for acceptance-only matrix coverage");
    }
    drop(state_store);

    let first_store = open_runtime_store(&state_layout, 1_710_100_094_010);
    let mut plane = TriggerPlane::open_with_store_acceptance_only(
        first_store,
        definitions,
        manifests,
        policy(
            &plugin_root,
            &["plugin-ready-only"],
            &[REQUIRED_TRIGGER_PLUGIN_CAPABILITY],
        ),
        BTreeMap::new(),
    )
    .expect("acceptance-only trigger plane should open for known-vs-unknown staged rows");

    let requests = plane
        .collect_run_requests(1_710_100_094_020)
        .expect("known staged rows should normalize while unknown rows remain pending");
    assert_eq!(requests.len(), 2);
    let request_ids = requests
        .iter()
        .map(|request| request.event_id.as_str())
        .collect::<BTreeSet<_>>();
    assert!(request_ids.contains("external-trigger-a:event-1"));
    assert!(request_ids.contains("external-trigger-b:event-1"));

    let mut verify_store = open_runtime_store(&state_layout, 1_710_100_094_030);
    assert!(verify_store
        .list_pending_staged_trigger_event_records("external-trigger-a", 10)
        .expect("pending rows for known trigger should list")
        .is_empty());
    assert!(verify_store
        .list_pending_staged_trigger_event_records("external-trigger-b", 10)
        .expect("pending rows for known trigger should list")
        .is_empty());
    let unknown_pending = verify_store
        .list_pending_staged_trigger_event_records("external-trigger-unknown", 10)
        .expect("pending rows for unknown trigger should remain available for future reconcile");
    assert_eq!(unknown_pending.len(), 1);
    assert_eq!(
        unknown_pending[0].event_id,
        "external-trigger-unknown:event-1"
    );
}

#[test]
fn acceptance_only_trigger_plane_consumes_staged_external_rows_without_direct_process_polling() {
    let (state_layout, plugin_root) = unique_layout("acceptance-only-external-bridge");
    let external_plugin_path = plugin_root.join("plugin-external.sh");
    write_executable_script(
        &external_plugin_path,
        "{\"type\":\"event\",\"checkpoint\":\"cp-ext\",\"event_key\":\"event/ext\",\"occurred_at_ms\":1710100120000,\"payload\":{\"side\":\"sell\"}}",
    );

    let definitions = vec![trigger_definition(
        "external-trigger",
        "external_plugin",
        "plugin-external",
    )];
    let manifests = vec![plugin_manifest(
        "plugin-external",
        "2.0.0",
        "external_trigger",
        "plugin-external.sh",
        &[REQUIRED_TRIGGER_PLUGIN_CAPABILITY],
    )];

    let first_store = open_runtime_store(&state_layout, 1_710_100_092_000);
    let mut acceptance_only_plane = TriggerPlane::open_with_store_acceptance_only(
        first_store,
        definitions.clone(),
        manifests.clone(),
        policy(
            &plugin_root,
            &["plugin-external"],
            &[REQUIRED_TRIGGER_PLUGIN_CAPABILITY],
        ),
        BTreeMap::new(),
    )
    .expect("acceptance-only trigger plane should open");

    let no_direct_requests = acceptance_only_plane
        .collect_run_requests(1_710_100_092_010)
        .expect("acceptance-only mode should not poll process plugin directly");
    assert!(no_direct_requests.is_empty());

    let mut stage_store = open_runtime_store(&state_layout, 1_710_100_092_020);
    stage_store
        .append_staged_trigger_event_record(&StagedTriggerEventRecord {
            schema_version: String::from("1.0.0"),
            staging_id: String::from("staging-acceptance-only-1"),
            trigger_id: String::from("external-trigger"),
            workflow_id: String::from("wf-external"),
            event_id: String::from("external-trigger:event/ext"),
            source: String::from("plugin-external"),
            occurred_at_ms: 1_710_100_092_000,
            staged_at_ms: 1_710_100_092_021,
            checkpoint: Some(String::from("cp-ext")),
            payload: serde_json::json!({"side": "sell"}),
            dedup_key: None,
            dedup_window_ms: None,
            cooldown_key: None,
            cooldown_ms: None,
            accepted_at_ms: None,
            last_error: None,
        })
        .expect("staged external row should persist for acceptance-only replay");
    drop(stage_store);

    let second_store = open_runtime_store(&state_layout, 1_710_100_092_030);
    let mut replay_plane = TriggerPlane::open_with_store_acceptance_only(
        second_store,
        definitions,
        manifests,
        policy(
            &plugin_root,
            &["plugin-external"],
            &[REQUIRED_TRIGGER_PLUGIN_CAPABILITY],
        ),
        BTreeMap::new(),
    )
    .expect("acceptance-only trigger plane should reopen");

    let staged_requests = replay_plane
        .collect_run_requests(1_710_100_092_040)
        .expect("acceptance-only mode should normalize staged external rows");
    assert_eq!(staged_requests.len(), 1);
    assert_eq!(staged_requests[0].event_id, "external-trigger:event/ext");
}

fn trigger_definition(trigger_id: &str, kind: &str, source: &str) -> TriggerDefinition {
    TriggerDefinition {
        api_version: "2.0.0".to_string(),
        trigger_id: trigger_id.to_string(),
        kind: kind.to_string(),
        source: source.to_string(),
        plugin: (kind == "external_plugin").then(|| source.to_string()),
        workflow_id: match trigger_id {
            "builtin-trigger" => "wf-builtin",
            "external-trigger" => "wf-external",
            "manual-trigger" => "wf-manual",
            "market-trigger" => "wf-market",
            "cron-trigger" => "wf-cron",
            _ => "wf-test",
        }
        .to_string(),
        enabled: true,
        params: BTreeMap::new(),
        input_mapping: BTreeMap::new(),
        package_root: PathBuf::new(),
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
        trigger_runtime: (kind == "external_trigger").then(|| ExternalTriggerRuntimeContract {
            lifecycle: Some(TriggerRuntimeLifecycle::ProcessShortLived),
            push_callback: Some(TriggerPushCallbackSemantics::InlineResponse),
            durable_ack: Some(TriggerDurableAckSemantics::CallerScope),
            host_error_categories: vec![
                TriggerHostErrorCategory::Transport,
                TriggerHostErrorCategory::ProtocolContract,
                TriggerHostErrorCategory::PluginFatal,
            ],
            module: None,
            abi: None,
        }),
        operations: Vec::new(),
        event_schema: (kind == "external_trigger").then(|| PluginEventSchemaDescriptor {
            summary: Some("External trigger payload".to_owned()),
            fields: vec!["symbol".to_owned(), "price".to_owned()],
            ..PluginEventSchemaDescriptor::default()
        }),
        activation: None,
        mcp: None,
        manifest_path: PathBuf::new(),
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
        plugin_activation: BTreeMap::new(),
        secrets_root_dir: plugin_root.join("../secrets"),
    }
}

fn builtin_event(
    event_id: &str,
    _workflow_id: &str,
    dedup_key: &str,
    dedup_window_ms: i64,
    cooldown_key: &str,
    cooldown_ms: i64,
) -> TriggerEmission {
    TriggerEmission {
        event_id: event_id.to_string(),
        occurred_at_ms: 1_710_100_010_000,
        checkpoint: None,
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
    let script = format!(
        "#!/bin/sh\nIFS= read -r _start_line\nprintf '%s\n' '{{\"type\":\"ready\",\"protocol_version\":\"2.0.0\"}}'\nprintf '%s\n' '{json_payload}'\n"
    );
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

fn write_trigger_env_probe_script(path: &Path, probe_key: &str, marker_path: &Path) {
    let script = format!(
        "#!/bin/sh\nIFS= read -r _start_line\nif [ -n \"$(printenv '{probe_key}' 2>/dev/null)\" ]; then\n  printf 'leaked' > \"{}\"\nfi\nprintf '%s\n' '{{\"type\":\"ready\",\"protocol_version\":\"2.0.0\"}}'\nprintf '%s\n' '{{\"type\":\"event\",\"checkpoint\":\"cp-env\",\"event_key\":\"event-env\",\"occurred_at_ms\":1710100060000,\"payload\":{{\"kind\":\"probe\"}}}}'\n",
        marker_path.display()
    );
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

fn write_trigger_input_capture_script(path: &Path, input_capture_path: &Path) {
    let script = format!(
        "#!/bin/sh\nIFS= read -r first_line\nprintf '%s' \"$first_line\" > \"{}\"\nprintf '%s\n' '{{\"type\":\"ready\",\"protocol_version\":\"2.0.0\"}}'\nprintf '%s\n' '{{\"type\":\"event\",\"checkpoint\":\"cp-params\",\"event_key\":\"event-params\",\"occurred_at_ms\":1710100070000,\"payload\":{{\"kind\":\"probe\"}}}}'\n",
        input_capture_path.display()
    );
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

fn write_protocol_script(path: &Path, body: &str) {
    let script = format!("#!/bin/sh\nIFS= read -r _start_line\n{body}");
    fs::write(path, script).expect("protocol script should be writable");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path)
            .expect("protocol script metadata should be readable")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).expect("protocol script should become executable");
    }
}

fn select_non_allowlisted_host_env_key() -> String {
    let mut candidates = std::env::vars_os()
        .filter_map(|(key, value)| {
            let key = key.to_string_lossy().to_string();
            if value.is_empty() || TEST_PLUGIN_HOST_ENV_ALLOWLIST.contains(&key.as_str()) {
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

fn open_runtime_store(state_layout: &StateLayout, now_ms: i64) -> RuntimeStateStore {
    RuntimeStateStore::open(
        &RuntimeStorageConfig {
            backend: RuntimeStorageBackend::Local {
                database_path: state_layout.coordination_db_path.clone(),
            },
            history_retention: None,
            raw_debug_enabled: false,
            raw_debug_artifacts_dir: None,
        },
        now_ms,
    )
    .expect("runtime state store should open for staged trigger tests")
}

const TEST_PLUGIN_HOST_ENV_ALLOWLIST: &[&str] =
    &["PATH", "SYSTEMROOT", "WINDIR", "COMSPEC", "PATHEXT"];
