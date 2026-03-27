# Local Rules

## Scope
- Position: Per-kind builtin trigger emitter implementations.
- Logic: Owns validation and emission logic for cron, manual, market-tick, webhook, and websocket trigger kinds.
- Constraints: Keep shared trigger contracts and fan-out orchestration in `../`.

## Members
- `mod.rs`: Emitter module boundary.
- `cron.rs`: Cron schedule emitter implementation.
- `manual.rs`: Manual trigger emitter implementation.
- `market_tick.rs`: Market-tick trigger emitter implementation.
- `webhook.rs`: Webhook trigger adapter emitter implementation.
- `websocket.rs`: WebSocket trigger adapter emitter implementation.
