use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::{to_bytes, Body};
use axum::extract::ConnectInfo;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{on, MethodFilter};
use axum::{Extension, Router};

use crate::config::RuntimeStorageConfig;
use crate::state::IngressInboxRecord;
use crate::state_db::RuntimeStateStore;

use super::contract::{IngressAuthConfig, IngressListenerSpec, IngressRuntimeError};

static WEBHOOK_EVENT_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
struct WebhookRouteState {
    spec: IngressListenerSpec,
    storage_config: RuntimeStorageConfig,
}

pub fn build_webhook_route(
    spec: IngressListenerSpec,
    storage_config: RuntimeStorageConfig,
) -> Result<Router, IngressRuntimeError> {
    let method = spec
        .method
        .as_deref()
        .ok_or_else(|| IngressRuntimeError::InvalidConfig(format!("webhook trigger {} is missing method", spec.trigger_id)))?;
    let filter = method_filter(method).ok_or_else(|| {
        IngressRuntimeError::InvalidConfig(format!(
            "webhook trigger {} uses unsupported method {method}",
            spec.trigger_id
        ))
    })?;
    let path = spec.path.clone();
    let state = Arc::new(WebhookRouteState { spec, storage_config });
    Ok(Router::new()
        .route(&path, on(filter, handle_webhook))
        .layer(Extension(state)))
}

async fn handle_webhook(
    Extension(state): Extension<Arc<WebhookRouteState>>,
    ConnectInfo(remote_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Body,
) -> impl IntoResponse {
    if let Err(status) = validate_auth(&state.spec.auth, &headers) {
        return status.into_response();
    }
    if let Some(expected) = state.spec.content_type.as_deref() {
        let matches = headers
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(|actual| actual.starts_with(expected))
            .unwrap_or(false);
        if !matches {
            return StatusCode::UNSUPPORTED_MEDIA_TYPE.into_response();
        }
    }

    let limit = state.spec.max_body_bytes.unwrap_or(64 * 1024);
    let bytes = match to_bytes(body, limit).await {
        Ok(bytes) => bytes,
        Err(_) => return StatusCode::PAYLOAD_TOO_LARGE.into_response(),
    };
    let payload: serde_json::Value = match serde_json::from_slice(&bytes) {
        Ok(payload) => payload,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };

    let now_ms = current_time_ms();
    let ingress_event_id = resolve_webhook_event_id(&state.spec, &headers, now_ms);
    let record = IngressInboxRecord {
        inbox_id: format!("{}:{ingress_event_id}", state.spec.trigger_id),
        schema_version: String::from("1.0.0"),
        trigger_id: state.spec.trigger_id.clone(),
        workflow_id: state.spec.workflow_id.clone(),
        transport_kind: String::from("webhook"),
        ingress_event_id,
        source: String::from("webhook"),
        route_path: state.spec.path.clone(),
        http_method: state.spec.method.clone(),
        received_at_ms: now_ms,
        payload,
        headers: collect_headers(&headers),
        remote_addr: Some(remote_addr.to_string()),
        processed_at_ms: None,
        last_error: None,
    };
    let append_result = tokio::task::spawn_blocking({
        let storage_config = state.storage_config.clone();
        move || {
            let mut store = RuntimeStateStore::open(&storage_config, now_ms)?;
            store.append_ingress_inbox_record(&record)
        }
    })
    .await;
    match append_result {
        Ok(Ok(_)) => StatusCode::ACCEPTED.into_response(),
        Ok(Err(_)) | Err(_) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

fn method_filter(value: &str) -> Option<MethodFilter> {
    match value {
        "GET" => Some(MethodFilter::GET),
        "POST" => Some(MethodFilter::POST),
        "PUT" => Some(MethodFilter::PUT),
        "PATCH" => Some(MethodFilter::PATCH),
        "DELETE" => Some(MethodFilter::DELETE),
        "OPTIONS" => Some(MethodFilter::OPTIONS),
        "HEAD" => Some(MethodFilter::HEAD),
        _ => None,
    }
}

fn validate_auth(auth: &Option<IngressAuthConfig>, headers: &HeaderMap) -> Result<(), StatusCode> {
    match auth {
        None => Ok(()),
        Some(IngressAuthConfig::HeaderToken { header_name, token }) => {
            let actual = headers
                .get(header_name)
                .and_then(|value| value.to_str().ok())
                .ok_or(StatusCode::UNAUTHORIZED)?;
            if actual == token {
                Ok(())
            } else {
                Err(StatusCode::FORBIDDEN)
            }
        }
    }
}

fn resolve_webhook_event_id(spec: &IngressListenerSpec, headers: &HeaderMap, now_ms: i64) -> String {
    if let Some(header_name) = spec.idempotency_header.as_deref() {
        if let Some(value) = headers.get(header_name).and_then(|header| header.to_str().ok()) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return trimmed.to_owned();
            }
        }
    }
    format!(
        "webhook-{now_ms}-{}",
        WEBHOOK_EVENT_COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

fn collect_headers(headers: &HeaderMap) -> BTreeMap<String, String> {
    headers
        .iter()
        .filter_map(|(name, value)| value.to_str().ok().map(|value| (name.to_string(), value.to_owned())))
        .collect()
}

fn current_time_ms() -> i64 {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
}
