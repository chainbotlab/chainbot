# Local Rules

## Scope
- Position: Ingress runtime subsystem for listener-backed builtin triggers in the `chainbot` crate.
- Logic: Owns trigger-local ingress params decoding contracts, listener reconciliation, webhook/websocket transport handling, and durable inbox bridging into the trigger plane.
- Constraints: Keep network listener lifecycle here, bound to the `serve` daemon lease; do not move accepted-event normalization out of `trigger.rs`.

## Members
- `mod.rs`: Ingress subsystem root exporting contracts, inbox helpers, reconciliation, and supervisor types.
- `contract.rs`: Shared ingress params contracts, desired-listener specs, and route normalization helpers.
- `reconcile.rs`: Converts trigger definitions into desired ingress listeners and validates route collisions.
- `inbox.rs`: Durable inbox bridging from ingress rows into `TriggerEmission` values.
- `supervisor.rs`: Background listener supervisor tied to daemon lifecycle and graceful shutdown.
- `webhook.rs`: HTTP webhook route assembly, request validation, and inbox append logic.
- `websocket.rs`: WebSocket route assembly, message validation, and inbox append logic.
