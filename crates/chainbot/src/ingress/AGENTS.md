# AGENTS.md

## Scope
- Position: Ingress runtime subsystem for listener-backed builtin triggers.
- Owns: Ingress parameter decoding, listener reconciliation, webhook/websocket transport, and inbox bridging.
- Excludes: Accepted-event normalization semantics.

## Constraints
- Keep listener lifecycle here and tied to the `serve` daemon lease.
- Keep accepted-event normalization in the trigger plane.

## Members
- `mod.rs`: Ingress namespace root.
- `contract.rs`: Ingress parameter and listener contracts.
- `reconcile.rs`: Listener reconciliation.
- `inbox.rs`: Event inbox bridging.
- `supervisor.rs`: Ingress supervision.
- `webhook.rs`: Webhook transport.
- `websocket.rs`: Websocket transport.
