# AGENTS.md

## Scope
- Position: Per-kind builtin trigger emitters.
- Owns: Validation and emission logic for cron, manual, market-tick, webhook, and websocket triggers.
- Excludes: Shared trigger contracts and fan-out orchestration.

## Constraints
- Keep shared trigger contracts in `../`.

## Members
- `mod.rs`: Emitter exports.
- `cron.rs`: Cron trigger behavior.
- `manual.rs`: Manual trigger behavior.
- `market_tick.rs`: Market-tick trigger behavior.
- `webhook.rs`: Webhook trigger behavior.
- `websocket.rs`: Websocket trigger behavior.
