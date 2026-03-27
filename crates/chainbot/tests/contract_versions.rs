//! [INPUT]
//! Serialized config, plugin, and worker fixtures plus direct secret reference literals.
//!
//! [OUTPUT]
//! Verifies acceptance of supported versions and rejection of unsupported future majors across frozen contracts.
//!
//! [ROLE]
//! Guards the crate's versioned contract compatibility boundary.

use chainbot::errors::ContractError;
use chainbot::infrastructure::config::ConfigRoot;
use chainbot::plugin::PluginManifest;
use chainbot::script_protocol::WorkerRequestEnvelope;
use chainbot::secrets::SecretReference;

#[test]
fn contract_versions() {
    let config_fixture = r#"
    {
      "schema_version": "2.0.0",
      "workflows": [
        {
          "workflow": {
            "manifest_version": "2.2.0",
            "id": "wf-alpha",
            "name": "alpha"
          },
          "nodes": [
            {
              "manifest_version": "2.1.0",
              "id": "node-quote",
              "kind": "plugin",
              "plugin": "quote-plugin",
              "operation": "normalize",
              "depends_on": []
            }
          ]
        }
      ],
      "plugins": [
        {
          "manifest_version": "2.0.0",
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
    assert_eq!(config.schema_version, "2.0.0");

    let secret =
        SecretReference::parse("secret://vault/api_key#value").expect("secret syntax should parse");
    assert_eq!(secret.to_uri(), "secret://vault/api_key#value");
}

#[test]
fn invalid_contract_fixtures_are_rejected() {
    let config_err = ConfigRoot::from_json_str(
        r#"{
          "schema_version": "4.0.0",
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
            major: 4,
            max_supported_major: 2
        }
    ));

    let plugin_err = PluginManifest::from_json_str(
        r#"{
          "manifest_version": "9.0.0",
          "plugin_id": "p1",
          "kind": "builtin",
          "entrypoint": "plugins.p1",
          "capabilities": []
        }"#,
    )
    .expect_err("future-major plugin api must be rejected");
    assert!(matches!(
        plugin_err,
        ContractError::UnsupportedMajorVersion {
            field: "plugin.manifest_version",
            major: 9,
            supported_major: 2
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
