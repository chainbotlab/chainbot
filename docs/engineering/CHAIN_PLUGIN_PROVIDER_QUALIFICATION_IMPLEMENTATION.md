# Chain Plugin Provider Qualification Implementation

## Goal

记录官方链插件的 live provider qualification 约束，确保真实公网 provider 验证保持 opt-in，而不是进入默认 correctness gate。

## Scope

- 适用于 `official-plugins/eth-node/crate`
- 适用于 `official-plugins/solana-node/crate`
- 适用于 `official-plugins/eth-trigger/crate`
- 适用于 `official-plugins/solana-trigger/crate`

## Qualification Contract

- 每个官方链插件 crate 都提供 `tests/live_qualification.rs`。
- 这些 tests 受 cargo feature `live-qualification` 保护。
- 这些 tests 同时标记为 `#[ignore]`，因此默认 `cargo test` 不会执行。
- 只有显式启用 feature 并显式运行 ignored tests 时，才允许访问真实 provider endpoint。

## Required Environment Variables

- `CHAINBOT_ETH_NODE_LIVE_ENDPOINT`
- `CHAINBOT_SOLANA_NODE_LIVE_ENDPOINT`
- `CHAINBOT_ETH_TRIGGER_LIVE_ENDPOINT`
- `CHAINBOT_SOLANA_TRIGGER_LIVE_ENDPOINT`
- optional: `CHAINBOT_ETH_TRIGGER_LIVE_ALLOWED_ORIGIN`
- optional: `CHAINBOT_SOLANA_TRIGGER_LIVE_ALLOWED_ORIGIN`
- optional: `CHAINBOT_ETH_TRIGGER_LIVE_RPC_TOKEN`
- optional: `CHAINBOT_SOLANA_TRIGGER_LIVE_RPC_TOKEN`

## Operator Expectations

- live qualification 只验证 provider compatibility 与 endpoint health。
- live qualification 不是默认 CI correctness gate。
- deterministic local tests 仍然是主验收面。
- provider credential 必须通过环境变量或 activation-secret-equivalent 注入，不能写进 curated examples。
- node qualification 会走一次真实 `raw_read`，验证 endpoint health 和基础 JSON-RPC compatibility。
- trigger qualification 会完成一次真实 trigger startup，并由本地 capture WebSocket 接住第一条 subscription request，验证公网 provider 配置和 activation contract 能一起完成启动。

## Notes

- 如果将来需要 richer qualification matrix，应继续沿用 feature-gated + ignored test 的模式，而不是把公网依赖塞进默认 test target。
