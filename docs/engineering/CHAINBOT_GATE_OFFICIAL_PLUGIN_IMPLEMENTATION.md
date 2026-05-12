# ChainBot Gate Official Plugin Implementation

## Goal

为 ChainBot workspace 增加 Gate 官方插件接入，范围严格收敛为 Gate Spot V1：

- `official-plugins/gate-node/`：Spot REST 读写 node plugin
- `official-plugins/gate-trigger/`：Spot market/user-stream trigger plugin

不把 Gate 协议细节回填到 `crates/chainbot` runtime。

## Implemented Changes

- 新增 `official-plugins/gate-node/` package：
  - `config.toml` 使用 `external_node` + `node.exec.v2`
  - activation slots: `api_key`, `api_secret`
  - Spot read operations:
    - `gate_get_server_time`
    - `gate_get_currency_pairs`
    - `gate_get_ticker`
    - `gate_get_depth`
    - `gate_get_klines`
    - `gate_get_accounts`
    - `gate_get_open_orders`
    - `gate_get_order`
  - Spot write operations:
    - `gate_place_order`
    - `gate_cancel_order`
    - `gate_cancel_all_orders`
  - package-local provider 实现 Gate API v4 Spot request signing、origin binding、destination policy 与 write confirmation lifecycle。
- 新增 `official-plugins/gate-trigger/` package：
  - `config.toml` 使用 `external_trigger` + `trigger.exec.v1`
  - activation slots: `api_key`, `api_secret`
  - trigger sources:
    - `gate_spot_market_stream`
    - `gate_spot_user_stream`
  - private channel 覆盖：
    - `spot.orders`
    - `spot.usertrades`
    - `spot.balances`
  - public market stream 保持 Gate WebSocket subscribe/update normalization 在 package 内部完成。
- 更新 `chainbot-plugin-index.toml`：登记 `gate-node` 与 `gate-trigger` official source catalog entry。
- 更新 `official-plugins/AGENTS.md`：补充 Gate packages 成员说明。
- 更新宿主侧集成测试：
  - `crates/chainbot/tests/chain_node_plugin_host.rs`
    - 新增 Gate activation secret + allowed_origins 注入覆盖
  - `crates/chainbot/tests/chain_trigger_runtime.rs`
    - 新增 Gate trigger start envelope activation 注入覆盖
  - `crates/chainbot/tests/catalog_surface.rs`
    - 新增 Gate official plugins catalog list/show discoverability 覆盖

## Contracts Preserved

- `chainbot runtime` 仍然只负责 generic orchestration：plugin activation secret resolution、allowed_origins transport、catalog projection、trigger-plane lifecycle。
- Gate-specific signing、REST endpoint shaping、WebSocket auth、channel normalization、event key derivation 都保持在 plugin package 内部。
- `allowed_origins` 继续作为 activation-bound secret transport 的必需 guardrail，不通过 workflow business input 传递 operator-owned secrets。

## Validation

- `cargo test --manifest-path official-plugins/gate-node/crate/Cargo.toml`
- `cargo test --manifest-path official-plugins/gate-trigger/crate/Cargo.toml`
- `cargo test -p chainbot --test chain_node_plugin_host`
- `cargo test -p chainbot --test chain_trigger_runtime`
- `cargo test -p chainbot --test catalog_surface`

## Test Coverage Added or Updated

- `official-plugins/gate-node/crate/tests/read_rpc.rs`
  - server time read contract
  - signed account read contract
- `official-plugins/gate-node/crate/tests/write_lifecycle.rs`
  - `submit_only` and `safe` confirmation lifecycle
  - allowlisted-origin enforcement
  - plugin id guardrail
- `official-plugins/gate-trigger/crate/tests/log_listener.rs`
  - market/user mock listener protocol
  - unsupported source / protocol rejection
  - insecure endpoint rejection
- `crates/chainbot/tests/chain_node_plugin_host.rs`
  - Gate node activation binding transport coverage
- `crates/chainbot/tests/chain_trigger_runtime.rs`
  - Gate trigger activation binding transport coverage
- `crates/chainbot/tests/catalog_surface.rs`
  - Gate official plugin catalog list/show discoverability coverage
