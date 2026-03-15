/*
[INPUT]:  JSON fixtures for config/plugin/worker and direct secret reference literals.
[OUTPUT]: Regression coverage for version-acceptance and future-major rejection behavior.
[POS]:    Integration test boundary for V2 contract freeze.
[UPDATE]: 2026-03-16 - Add contract-version acceptance and rejection tests.
*/

use chainbot::config::ConfigRoot;
use chainbot::errors::ContractError;
use chainbot::plugin::PluginManifest;
use chainbot::secrets::SecretReference;
use chainbot::worker::WorkerRequestEnvelope;

#[test]
fn contract_versions() {
    let config_fixture = r#"
    {
      "schema_version": "1.0.0",
      "workflows": [
        {
          "api_version": "1.2.0",
          "workflow_id": "wf-alpha",
          "name": "alpha",
          "triggers": [
            {
              "api_version": "1.0.1",
              "trigger_id": "tr-market",
              "kind": "market_tick",
              "source": "market-feed",
              "enabled": true
            }
          ],
          "nodes": [
            {
              "api_version": "1.1.0",
              "node_id": "node-quote",
              "kind": "plugin",
              "plugin_id": "quote-plugin",
              "operation": "normalize",
              "depends_on": []
            }
          ]
        }
      ],
      "plugins": [
        {
          "api_version": "1.0.0",
          "plugin_id": "quote-plugin",
          "kind": "builtin",
          "entrypoint": "plugins.quote",
          "capabilities": ["normalize"]
        }
      ],
      "worker_templates": [
        {
          "name": "default-worker",
          "request": {
            "protocol_version": "1.0.0",
            "request_id": "req-1",
            "worker_id": "worker-a",
            "workflow_id": "wf-alpha",
            "payload": {"task": "run"}
          },
          "response": {
            "protocol_version": "1.0.0",
            "request_id": "req-1",
            "success": true,
            "output": {"ok": true}
          }
        }
      ],
      "run_defaults": {
        "schema_version": "1.0.0",
        "run_id": "run-1",
        "workflow_id": "wf-alpha",
        "status": "pending",
        "started_at_ms": 1,
        "finished_at_ms": null
      }
    }
    "#;

    let config = ConfigRoot::from_json_str(config_fixture).expect("config fixture should be valid");
    let config_json = serde_json::to_string(&config).expect("config should serialize");
    let reparsed =
        ConfigRoot::from_json_str(&config_json).expect("serialized config should reparse");
    assert_eq!(reparsed.schema_version, "1.0.0");

    let secret =
        SecretReference::parse("secret://vault/api_key#value").expect("secret syntax should parse");
    assert_eq!(secret.to_uri(), "secret://vault/api_key#value");
}

#[test]
fn invalid_contract_fixtures_are_rejected() {
    let config_err = ConfigRoot::from_json_str(
        r#"{
          "schema_version": "2.0.0",
          "workflows": [],
          "plugins": [],
          "worker_templates": [],
          "run_defaults": null
        }"#,
    )
    .expect_err("future-major config schema must be rejected");
    assert!(matches!(
        config_err,
        ContractError::UnsupportedFutureMajorVersion {
            field: "config.schema_version",
            major: 2,
            max_supported_major: 1
        }
    ));

    let plugin_err = PluginManifest::from_json_str(
        r#"{
          "api_version": "9.0.0",
          "plugin_id": "p1",
          "kind": "builtin",
          "entrypoint": "plugins.p1",
          "capabilities": []
        }"#,
    )
    .expect_err("future-major plugin api must be rejected");
    assert!(matches!(
        plugin_err,
        ContractError::UnsupportedFutureMajorVersion {
            field: "plugin.api_version",
            major: 9,
            max_supported_major: 1
        }
    ));

    let worker_err = WorkerRequestEnvelope::from_json_str(
        r#"{
          "protocol_version": "3.0.0",
          "request_id": "req-x",
          "worker_id": "worker-a",
          "workflow_id": "wf-a",
          "payload": {}
        }"#,
    )
    .expect_err("future-major worker protocol must be rejected");
    assert!(matches!(
        worker_err,
        ContractError::UnsupportedFutureMajorVersion {
            field: "worker_request.protocol_version",
            major: 3,
            max_supported_major: 1
        }
    ));

    let invalid_secret = SecretReference::parse("secret://vault-only")
        .expect_err("invalid secret URI must be rejected");
    assert!(matches!(
        invalid_secret,
        ContractError::InvalidSecretReferenceSyntax { .. }
    ));
}
