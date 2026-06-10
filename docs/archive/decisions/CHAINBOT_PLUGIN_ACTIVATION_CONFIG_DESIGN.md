---
type: archive
status: superseded
updated: 2026-06-10
replacement:
  - ../../../.agents/skills/decision-chainbot-plugin-activation-boundary/SKILL.md
---

# ChainBot Plugin Activation Config Design

Archive Note: Active decision authority moved to `.agents/skills/decision-chainbot-plugin-activation-boundary/SKILL.md` on 2026-06-10. This file is retained as a historical snapshot.

## Goal

定义 operator-owned plugin activation config，使已安装 plugin 可以声明 root-local secret bindings，并由 `chainbot runtime` 在 execution time 解密后注入 plugin request，同时保持 package manifest 与业务输入边界清晰。

## Scope

- 本设计适用于 `<root>/chainbot.toml` 中的 root-owned activation config。
- 本设计适用于 installed node plugins 与 installed trigger plugins。
- 本设计不改变 plugin source install metadata。
- 本设计不把 chain-specific signing algorithm 放入 `chainbot runtime`。

## Ownership Boundary

- `plugins/<plugin_id>/config.toml` 是 package-owned manifest，由 plugin package 与 install flow 管理。
- `chainbot.toml` 中的 plugin activation config 是 operator-owned runtime config。
- plugin reinstall 或 upgrade 不得覆盖 operator-owned activation config。
- workflow node `input` 与 trigger `params` 继续承载业务输入，不承载 root-local activation secret bindings。

## Config Location

plugin activation config 定义在 `chainbot.toml`：

```toml
manifest_version = "2.0.0"
chainbot_version = "2.1.5"
profile = "prod"

[plugin_activation."eth-node".secret_bindings]
signer = "secret://wallets/eth/hot#private_key"
rpc_token = "secret://providers/alchemy#token"

[plugin_activation."solana-trigger".secret_bindings]
rpc_token = "secret://providers/helius#token"
```

## Activation Contract

- `plugin_activation.<plugin_id>` 按 installed `plugin_id` 建立 root-local activation scope。
- `secret_bindings` 是 `slot -> secret ref` 的映射。
- `slot` 是 plugin-owned 名称，例如 `signer`、`rpc_token`、`ws_token`。
- `chainbot runtime` 不解释 `slot` 的业务含义，只负责按名称注入对应 plaintext secret。
- activation config 是可选的；未配置的 plugin 不获得 activation secret bindings。

## Execution Semantics

- activation config 在 root loading 时只校验结构与 `secret ref` syntax。
- secret existence、decryption、与 keyed lookup 继续延后到 execution time。
- node plugin request 与 trigger start envelope 应通过 dedicated activation section 接收 resolved plaintext secret bindings。
- activation secret bindings 与 node operation `input`、trigger `params` 保持分离，不做隐式字段合并。
- plugin 负责消费 injected secrets，并在本地完成 chain-specific signing、authentication、或 provider access。

## Protocol Envelope

为避免 node execution 与 trigger startup 采用不同形状，activation payload 使用统一 envelope：

```json
{
  "activation": {
    "secrets": {
      "signer": "<plaintext secret>",
      "rpc_token": "<plaintext secret>"
    }
  }
}
```

### Node Plugin Request

- `ExternalNodePluginRequest` 增加 optional `activation` object。
- `activation.secrets` 是 `slot -> plaintext secret` 的 map。
- `input` 继续承载 request-scoped business input；`activation.secrets` 承载 host-resolved operator bindings。

示例：

```json
{
  "contract_version": "1.0.0",
  "plugin_id": "eth-node",
  "node_id": "transfer-usdc",
  "operation": "eth_transfer_erc20",
  "requested_capabilities": ["node:execute"],
  "input": {
    "endpoint": "https://rpc.example",
    "to": "0xabc",
    "amount": "1000000",
    "confirmation_mode": "safe"
  },
  "activation": {
    "secrets": {
      "signer": "<plaintext secret>",
      "rpc_token": "<plaintext secret>"
    }
  }
}
```

### Trigger Start Envelope

- `TriggerStartCommand` 增加 optional `activation` object。
- `activation.secrets` 与 node request 采用相同 shape。
- `params` 继续承载 trigger behavior config；`activation.secrets` 承载 host-resolved operator bindings。

示例：

```json
{
  "type": "start",
  "protocol_version": "2.0.0",
  "trigger_id": "eth-transfer-listener",
  "source": "eth_log",
  "params": {
    "endpoint": "wss://rpc.example",
    "contract_address": "0xabc"
  },
  "heartbeat_interval_ms": 5000,
  "shutdown_grace_ms": 10000,
  "activation": {
    "secrets": {
      "rpc_token": "<plaintext secret>"
    }
  }
}
```

## Envelope Rules

- `activation` 整体可选；未配置 activation binding 时可省略。
- `activation.secrets` 整体可选；为空时不应发送空意义字段。
- `activation.secrets` 的 value 必须是 plaintext string，不使用 structured JSON value。
- plugin 不得假设所有 configured slots 都一定存在；缺失 slot 应由 plugin 按自身 contract fail closed。
- host 不把 `activation.secrets` 回写到 runtime state、checkpoint、或 user-visible read model。
- V1 official chain trigger plugins may ignore `resume_checkpoint` and emit live-only events after listener startup.

## Validation Rules

- `plugin_activation` 只能引用已安装的 `plugin_id`。
- `secret_bindings` 的 key 必须非空。
- `secret_bindings` 的 value 必须是可解析的 `secret://...` reference。
- activation config 不要求在 root load 时验证 secret file 是否存在。
- activation config 不要求在 root load 时验证 plugin 是否真的会消费某个 `slot`。

## Runtime Boundary

- `chainbot runtime` 负责：
  - load root-owned activation config
  - resolve configured `secret ref` values at execution time
  - inject plaintext secret bindings into plugin envelopes using the shared `activation.secrets` shape
  - redact surfaced secret material from runtime-visible outputs and errors
- `chainbot runtime` 不负责：
  - signing algorithm
  - transaction assembly
  - provider-specific auth semantics
  - chain-specific meaning of any activation `slot`

## Non-Goals

- 不把 activation config 写回 plugin package directory。
- 不通过 environment variables 作为首选 secret transport。
- 不把 activation secret bindings 混入 workflow node `input` 或 trigger `params`。
- 不在 install time 预解密或持久化 plaintext secrets。
- 不为每条链在 runtime 中增加专有 activation logic。

## Rationale

- operator-owned activation config 与 package-owned manifest 分离后，install 与 upgrade 不会破坏本地 secret bindings。
- dedicated activation section 比把 secret 平铺进业务输入更清晰，也更不容易与 operation input schema 冲突。
- execution-time resolution 保持了现有 secret contract，并避免把明文 secret 持久化到 root config 或 plugin package。

## Change Triggers

- 改变 activation config 的 root location 或 TOML shape。
- 改变 `secret_bindings` 的注入语义或 transport envelope。
- 改变 operator-owned 与 package-owned config 的边界。
- 允许 activation config 承载除 secret bindings 之外的新稳定字段。
