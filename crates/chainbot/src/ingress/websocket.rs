//! [INPUT]
//! Ingress listener specs, storage configuration, websocket transport primitives, auth contracts, and inbox record types.
//!
//! [OUTPUT]
//! Builds websocket routes that validate inbound messages and append accepted payloads into the ingress inbox.
//!
//! [ROLE]
//! Implements the WebSocket transport for ingress-backed builtin triggers.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::ConnectInfo;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Extension, Router};
use futures_util::{SinkExt, StreamExt};

use crate::domain::state::IngressInboxRecord;
use crate::infrastructure::config::RuntimeStorageConfig;
use crate::infrastructure::state::RuntimeStateStore;

use super::contract::{IngressAuthConfig, IngressListenerSpec, IngressRuntimeError};

static WEBSOCKET_EVENT_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
struct WebSocketRouteState {
    spec: IngressListenerSpec,
    storage_config: RuntimeStorageConfig,
    active_connections: Arc<AtomicUsize>,
}

pub fn build_websocket_route(
    spec: IngressListenerSpec,
    storage_config: RuntimeStorageConfig,
) -> Result<Router, IngressRuntimeError> {
    let path = spec.path.clone();
    let state = Arc::new(WebSocketRouteState {
        spec,
        storage_config,
        active_connections: Arc::new(AtomicUsize::new(0)),
    });
    Ok(Router::new().route(&path, get(handle_websocket)).layer(Extension(state)))
}

async fn handle_websocket(
    ws: WebSocketUpgrade,
    Extension(state): Extension<Arc<WebSocketRouteState>>,
    ConnectInfo(remote_addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(status) = validate_auth(&state.spec.auth, &headers) {
        return status.into_response();
    }
    let max_connections = state.spec.max_connections.unwrap_or(100);
    let active = state.active_connections.fetch_add(1, Ordering::Relaxed) + 1;
    if active > max_connections {
        state.active_connections.fetch_sub(1, Ordering::Relaxed);
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    let state_for_upgrade = Arc::clone(&state);
    let active_connections = Arc::clone(&state.active_connections);
    ws.on_upgrade(move |socket| async move {
        handle_socket(socket, state_for_upgrade, remote_addr, headers).await;
        active_connections.fetch_sub(1, Ordering::Relaxed);
    })
    .into_response()
}

async fn handle_socket(
    mut socket: WebSocket,
    state: Arc<WebSocketRouteState>,
    remote_addr: SocketAddr,
    headers: HeaderMap,
) {
    let idle_timeout = state.spec.idle_timeout_ms;
    loop {
        let next_message = match idle_timeout {
            Some(timeout_ms) => {
                match tokio::time::timeout(
                    tokio::time::Duration::from_millis(u64::try_from(timeout_ms).unwrap_or(u64::MAX)),
                    socket.next(),
                )
                .await
                {
                    Ok(message) => message,
                    Err(_) => {
                        let _ = socket.close().await;
                        return;
                    }
                }
            }
            None => socket.next().await,
        };
        let Some(Ok(message)) = next_message else {
            return;
        };
        match message {
            Message::Text(text) => {
                let max_message_bytes = state.spec.max_message_bytes.unwrap_or(64 * 1024);
                if text.len() > max_message_bytes {
                    let _ = socket.close().await;
                    return;
                }
                let payload: serde_json::Value = match serde_json::from_str(&text) {
                    Ok(payload) => payload,
                    Err(_) => {
                        let _ = socket.close().await;
                        return;
                    }
                };
                let now_ms = current_time_ms();
                let ingress_event_id = format!(
                    "websocket-{now_ms}-{}",
                    WEBSOCKET_EVENT_COUNTER.fetch_add(1, Ordering::Relaxed)
                );
                let record = IngressInboxRecord {
                    inbox_id: format!("{}:{ingress_event_id}", state.spec.trigger_id),
                    schema_version: String::from("1.0.0"),
                    trigger_id: state.spec.trigger_id.clone(),
                    workflow_id: state.spec.workflow_id.clone(),
                    transport_kind: String::from("websocket"),
                    ingress_event_id,
                    source: String::from("websocket"),
                    route_path: state.spec.path.clone(),
                    http_method: None,
                    received_at_ms: now_ms,
                    payload,
                    headers: collect_headers(&headers),
                    remote_addr: Some(remote_addr.to_string()),
                    processed_at_ms: None,
                    last_error: None,
                };
                let storage_config = state.storage_config.clone();
                let append_result = tokio::task::spawn_blocking(move || {
                    let mut store = RuntimeStateStore::open(&storage_config, now_ms)?;
                    store.append_ingress_inbox_record(&record)
                })
                .await;
                if !matches!(append_result, Ok(Ok(_))) {
                    let _ = socket.close().await;
                    return;
                }
            }
            Message::Close(_) => return,
            Message::Ping(_) | Message::Pong(_) => {}
            Message::Binary(_) => {
                let _ = socket.close().await;
                return;
            }
        }
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
