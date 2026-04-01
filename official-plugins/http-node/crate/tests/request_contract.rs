#[path = "support/http_fixture.rs"]
mod http_fixture;

use http_fixture::HttpFixture;
use http_node_official_plugin::handle_request_json;
use serde_json::{json, Value};

#[tokio::test]
async fn request_returns_compatible_output_for_text_response() {
    let fixture = HttpFixture::start().await;
    let response = handle_request_json(
        &json!({
            "contract_version": "1.0.0",
            "plugin_id": "http-node",
            "node_id": "node-1",
            "operation": "request",
            "input": {
                "url": http_fixture::path_url(&fixture.base_url, "/echo"),
                "method": "GET"
            }
        })
        .to_string(),
    )
    .await
    .expect("request should succeed");

    let json: Value = serde_json::from_str(&response).expect("response should decode");
    assert_eq!(json.get("success"), Some(&json!(true)));
    assert_eq!(json.pointer("/output/status"), Some(&json!(200)));
    assert_eq!(json.pointer("/output/ok"), Some(&json!(true)));
    assert!(json.pointer("/output/body").and_then(Value::as_str).unwrap_or_default().contains("ok"));
    assert!(json.pointer("/output/headers").is_some());
}

#[tokio::test]
async fn request_rejects_private_and_metadata_targets() {
    for url in [
        http_fixture::private_ip_url(),
        http_fixture::metadata_ip_url(),
        String::from("http://100.64.0.1/shared"),
        String::from("http://[::ffff:127.0.0.1]:65535/private"),
    ] {
        let response = handle_request_json(
            &json!({
                "contract_version": "1.0.0",
                "plugin_id": "http-node",
                "node_id": "node-2",
                "operation": "request",
                "input": {
                    "url": url,
                    "method": "GET"
                }
            })
            .to_string(),
        )
        .await
        .expect("request should return failure response");
        let json: Value = serde_json::from_str(&response).expect("response should decode");
        let error = json.get("error").and_then(Value::as_str).unwrap_or_default();
        assert_eq!(json.get("success"), Some(&json!(false)));
        assert!(error.contains("blocked") || error.contains("failed to resolve destination host"));
    }
}

#[tokio::test]
async fn request_rejects_redirect_and_binary_payloads() {
    let fixture = HttpFixture::start().await;
    let redirect = handle_request_json(
        &json!({
            "contract_version": "1.0.0",
            "plugin_id": "http-node",
            "node_id": "node-3",
            "operation": "request",
            "input": {
                "url": http_fixture::path_url(&fixture.base_url, "/redirect"),
                "method": "GET"
            }
        })
        .to_string(),
    )
    .await
    .expect("request should return failure response");
    let redirect_json: Value = serde_json::from_str(&redirect).expect("response should decode");
    assert_eq!(redirect_json.get("success"), Some(&json!(false)));
    assert!(redirect_json.get("error").and_then(Value::as_str).unwrap_or_default().contains("redirect responses are not supported"));

    let binary = handle_request_json(
        &json!({
            "contract_version": "1.0.0",
            "plugin_id": "http-node",
            "node_id": "node-4",
            "operation": "request",
            "input": {
                "url": http_fixture::path_url(&fixture.base_url, "/binary"),
                "method": "GET"
            }
        })
        .to_string(),
    )
    .await
    .expect("request should return failure response");
    let binary_json: Value = serde_json::from_str(&binary).expect("response should decode");
    assert_eq!(binary_json.get("success"), Some(&json!(false)));
    assert!(binary_json.get("error").and_then(Value::as_str).unwrap_or_default().contains("unsupported response content-type"));
}

#[tokio::test]
async fn request_rejects_secret_references_and_host_override() {
    let fixture = HttpFixture::start().await;
    let secret_url = handle_request_json(
        &json!({
            "contract_version": "1.0.0",
            "plugin_id": "http-node",
            "node_id": "node-5",
            "operation": "request",
            "input": {
                "url": "secret://ops/http/url#token"
            }
        })
        .to_string(),
    )
    .await
    .expect("request should return failure response");
    let secret_url_json: Value = serde_json::from_str(&secret_url).expect("response should decode");
    assert!(secret_url_json.get("error").and_then(Value::as_str).unwrap_or_default().contains("must not contain secret references"));

    let secret_body = handle_request_json(
        &json!({
            "contract_version": "1.0.0",
            "plugin_id": "http-node",
            "node_id": "node-6",
            "operation": "request",
            "input": {
                "url": http_fixture::path_url(&fixture.base_url, "/echo"),
                "method": "POST",
                "body": "secret://ops/http/body#token"
            }
        })
        .to_string(),
    )
    .await
    .expect("request should return failure response");
    let secret_body_json: Value = serde_json::from_str(&secret_body).expect("response should decode");
    assert!(secret_body_json.get("error").and_then(Value::as_str).unwrap_or_default().contains("must not contain secret references"));

    let host_override = handle_request_json(
        &json!({
            "contract_version": "1.0.0",
            "plugin_id": "http-node",
            "node_id": "node-7",
            "operation": "request",
            "input": {
                "url": http_fixture::path_url(&fixture.base_url, "/echo"),
                "method": "GET",
                "headers": {
                    "host": "attacker.example.com"
                }
            }
        })
        .to_string(),
    )
    .await
    .expect("request should return failure response");
    let host_override_json: Value = serde_json::from_str(&host_override).expect("response should decode");
    assert!(host_override_json.get("error").and_then(Value::as_str).unwrap_or_default().contains("must not set `host`"));
}
