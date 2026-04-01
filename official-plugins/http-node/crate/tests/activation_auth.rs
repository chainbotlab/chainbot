#[path = "support/http_fixture.rs"]
mod http_fixture;

use http_fixture::HttpFixture;
use http_node_official_plugin::handle_request_json;
use serde_json::{json, Value};

#[tokio::test]
async fn request_uses_activation_authorization_for_allowed_origin() {
    let fixture = HttpFixture::start().await;
    let response = handle_request_json(
        &json!({
            "contract_version": "1.0.0",
            "plugin_id": "http-node",
            "node_id": "node-auth-1",
            "operation": "request",
            "input": {
                "url": http_fixture::path_url(&fixture.base_url, "/auth"),
                "method": "GET"
            },
            "activation": {
                "secrets": {
                    "authorization": "Bearer fixture-token"
                },
                "allowed_origins": [http_fixture::origin(&fixture.base_url)]
            }
        })
        .to_string(),
    )
    .await
    .expect("request should return response");
    let json: Value = serde_json::from_str(&response).expect("response should decode");
    assert_eq!(json.get("success"), Some(&json!(true)));
    assert_eq!(json.pointer("/output/status"), Some(&json!(200)));
}

#[tokio::test]
async fn request_rejects_origin_mismatch_for_activation_secret() {
    let fixture = HttpFixture::start().await;
    let response = handle_request_json(
        &json!({
            "contract_version": "1.0.0",
            "plugin_id": "http-node",
            "node_id": "node-auth-2",
            "operation": "request",
            "input": {
                "url": http_fixture::path_url(&fixture.base_url, "/auth"),
                "method": "GET"
            },
            "activation": {
                "secrets": {
                    "authorization": "Bearer fixture-token"
                },
                "allowed_origins": ["https://api.example.test"]
            }
        })
        .to_string(),
    )
    .await
    .expect("request should return failure response");
    let json: Value = serde_json::from_str(&response).expect("response should decode");
    assert_eq!(json.get("success"), Some(&json!(false)));
    assert!(json.get("error").and_then(Value::as_str).unwrap_or_default().contains("allowlisted"));
}

#[tokio::test]
async fn request_rejects_missing_allowed_origins_when_activation_secret_exists() {
    let fixture = HttpFixture::start().await;
    let response = handle_request_json(
        &json!({
            "contract_version": "1.0.0",
            "plugin_id": "http-node",
            "node_id": "node-auth-2b",
            "operation": "request",
            "input": {
                "url": http_fixture::path_url(&fixture.base_url, "/auth"),
                "method": "GET"
            },
            "activation": {
                "secrets": {
                    "authorization": "Bearer fixture-token"
                },
                "allowed_origins": []
            }
        })
        .to_string(),
    )
    .await
    .expect("request should return failure response");
    let json: Value = serde_json::from_str(&response).expect("response should decode");
    assert_eq!(json.get("success"), Some(&json!(false)));
    assert!(json.get("error").and_then(Value::as_str).unwrap_or_default().contains("at least one allowed origin"));
}

#[tokio::test]
async fn request_rejects_authorization_header_in_workflow_input() {
    let fixture = HttpFixture::start().await;
    let response = handle_request_json(
        &json!({
            "contract_version": "1.0.0",
            "plugin_id": "http-node",
            "node_id": "node-auth-3",
            "operation": "request",
            "input": {
                "url": http_fixture::path_url(&fixture.base_url, "/auth"),
                "method": "GET",
                "headers": {
                    "authorization": "Bearer manual-token"
                }
            },
            "activation": {
                "secrets": {
                    "authorization": "Bearer fixture-token"
                },
                "allowed_origins": [http_fixture::origin(&fixture.base_url)]
            }
        })
        .to_string(),
    )
    .await
    .expect("request should return failure response");
    let json: Value = serde_json::from_str(&response).expect("response should decode");
    assert_eq!(json.get("success"), Some(&json!(false)));
    assert!(json.get("error").and_then(Value::as_str).unwrap_or_default().contains("activation secret slot `authorization`"));
}
