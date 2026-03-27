//! [INPUT]
//! Trigger definitions and ingress params contracts for listener-backed builtin triggers.
//!
//! [OUTPUT]
//! Builds the desired ingress listener set and rejects route collisions before runtime startup.
//!
//! [ROLE]
//! Keeps ingress route selection explicit and validated outside transport startup.

use std::collections::BTreeMap;

use crate::domain::trigger::{TriggerDefinition, TriggerKind};
use crate::errors::ContractError;

use super::contract::{
    decode_webhook_params, decode_websocket_params, normalize_bind, normalize_method,
    normalize_path, DesiredIngressState, IngressListenerSpec, IngressTransportKind,
    BUILTIN_TRIGGER_WEBHOOK_KIND, BUILTIN_TRIGGER_WEBSOCKET_KIND,
};

pub fn build_desired_ingress_state(
    triggers: &[TriggerDefinition],
) -> Result<DesiredIngressState, ContractError> {
    let mut listeners = Vec::new();
    let mut webhook_routes = BTreeMap::<(String, String, String), String>::new();
    let mut websocket_routes = BTreeMap::<(String, String), String>::new();

    for trigger in triggers {
        if !trigger.enabled || !matches!(trigger.kind()?, TriggerKind::Builtin) {
            continue;
        }
        match trigger.builtin_subtype()? {
            Some(BUILTIN_TRIGGER_WEBHOOK_KIND) => {
                let params = decode_webhook_params(trigger)?;
                let bind = normalize_bind(&params.bind);
                let path = normalize_path(&params.path);
                let method = normalize_method(&params.method);
                if let Some(existing) = webhook_routes.insert(
                    (bind.clone(), method.clone(), path.clone()),
                    trigger.trigger_id.clone(),
                ) {
                    return Err(ContractError::InvalidTriggerDefinitionField {
                        trigger_id: trigger.trigger_id.clone(),
                        field: "trigger.params.path",
                        detail: format!(
                            "ingress route collision with trigger `{existing}` at {method} {path} on {bind}"
                        ),
                    });
                }
                listeners.push(IngressListenerSpec {
                    trigger_id: trigger.trigger_id.clone(),
                    workflow_id: trigger.workflow_id.clone(),
                    transport: IngressTransportKind::Webhook,
                    bind,
                    path,
                    method: Some(method),
                    auth: params.auth,
                    max_body_bytes: Some(params.max_body_bytes),
                    max_message_bytes: None,
                    max_connections: None,
                    idle_timeout_ms: None,
                    content_type: params.content_type,
                    idempotency_header: params.idempotency_header,
                });
            }
            Some(BUILTIN_TRIGGER_WEBSOCKET_KIND) => {
                let params = decode_websocket_params(trigger)?;
                let bind = normalize_bind(&params.bind);
                let path = normalize_path(&params.path);
                if let Some(existing) = websocket_routes
                    .insert((bind.clone(), path.clone()), trigger.trigger_id.clone())
                {
                    return Err(ContractError::InvalidTriggerDefinitionField {
                        trigger_id: trigger.trigger_id.clone(),
                        field: "trigger.params.path",
                        detail: format!(
                            "ingress route collision with trigger `{existing}` at websocket {path} on {bind}"
                        ),
                    });
                }
                listeners.push(IngressListenerSpec {
                    trigger_id: trigger.trigger_id.clone(),
                    workflow_id: trigger.workflow_id.clone(),
                    transport: IngressTransportKind::WebSocket,
                    bind,
                    path,
                    method: None,
                    auth: params.auth,
                    max_body_bytes: None,
                    max_message_bytes: Some(params.max_message_bytes),
                    max_connections: Some(params.max_connections),
                    idle_timeout_ms: params.idle_timeout_ms,
                    content_type: None,
                    idempotency_header: None,
                });
            }
            _ => {}
        }
    }

    listeners.sort_by(|left, right| {
        (&left.bind, &left.path, &left.trigger_id).cmp(&(
            &right.bind,
            &right.path,
            &right.trigger_id,
        ))
    });

    Ok(DesiredIngressState { listeners })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use super::*;

    fn trigger_definition(trigger_id: &str, source: &str, enabled: bool) -> TriggerDefinition {
        TriggerDefinition {
            api_version: String::from("2.0.0"),
            trigger_id: trigger_id.to_owned(),
            kind: String::from("builtin"),
            source: source.to_owned(),
            plugin: None,
            workflow_id: String::from("wf-alpha"),
            enabled,
            params: BTreeMap::new(),
            input_mapping: BTreeMap::new(),
            package_root: PathBuf::from(trigger_id),
        }
    }

    #[test]
    fn build_desired_ingress_state_collects_enabled_listener_triggers() {
        let mut webhook = trigger_definition("tr-webhook", BUILTIN_TRIGGER_WEBHOOK_KIND, true);
        webhook.params = BTreeMap::from([
            (String::from("bind"), serde_json::json!("127.0.0.1:9100")),
            (String::from("path"), serde_json::json!("/hook")),
            (String::from("method"), serde_json::json!("post")),
            (String::from("max_body_bytes"), serde_json::json!(4096)),
        ]);
        let mut websocket =
            trigger_definition("tr-websocket", BUILTIN_TRIGGER_WEBSOCKET_KIND, true);
        websocket.params = BTreeMap::from([
            (String::from("bind"), serde_json::json!("127.0.0.1:9100")),
            (String::from("path"), serde_json::json!("/stream")),
            (String::from("max_connections"), serde_json::json!(32)),
            (String::from("max_message_bytes"), serde_json::json!(2048)),
        ]);
        let disabled = trigger_definition("tr-disabled", BUILTIN_TRIGGER_WEBHOOK_KIND, false);

        let desired = build_desired_ingress_state(&[webhook, websocket, disabled])
            .expect("listener-backed ingress triggers should decode");

        assert_eq!(desired.listeners.len(), 2);
        assert_eq!(desired.listeners[0].bind, "127.0.0.1:9100");
        assert_eq!(desired.listeners[0].path, "/hook");
        assert_eq!(desired.listeners[0].method.as_deref(), Some("POST"));
        assert_eq!(desired.listeners[1].path, "/stream");
        assert_eq!(
            desired.listeners[1].transport,
            IngressTransportKind::WebSocket
        );
    }

    #[test]
    fn build_desired_ingress_state_rejects_route_collisions() {
        let mut left = trigger_definition("tr-left", BUILTIN_TRIGGER_WEBHOOK_KIND, true);
        left.params = BTreeMap::from([
            (String::from("bind"), serde_json::json!("127.0.0.1:9200")),
            (String::from("path"), serde_json::json!("/hook/")),
            (String::from("method"), serde_json::json!("POST")),
            (String::from("max_body_bytes"), serde_json::json!(4096)),
        ]);
        let mut right = trigger_definition("tr-right", BUILTIN_TRIGGER_WEBHOOK_KIND, true);
        right.params = BTreeMap::from([
            (String::from("bind"), serde_json::json!("127.0.0.1:9200")),
            (String::from("path"), serde_json::json!("hook")),
            (String::from("method"), serde_json::json!("post")),
            (String::from("max_body_bytes"), serde_json::json!(2048)),
        ]);

        let error = build_desired_ingress_state(&[left, right])
            .expect_err("colliding normalized routes should be rejected");

        assert!(error.to_string().contains("ingress route collision"));
    }
}
