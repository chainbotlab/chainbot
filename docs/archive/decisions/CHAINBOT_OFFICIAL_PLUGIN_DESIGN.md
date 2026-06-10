---
type: archive
status: superseded
updated: 2026-06-10
replacement:
  - ../../../.agents/skills/decision-chainbot-official-plugin-design/SKILL.md
  - ../../../.agents/skills/decision-chainbot-plugin-activation-boundary/SKILL.md
---

# ChainBot Official Plugin Design

Archive Note: Active decision authority moved to `.agents/skills/decision-chainbot-official-plugin-design/SKILL.md` and `.agents/skills/decision-chainbot-plugin-activation-boundary/SKILL.md` on 2026-06-10. This file is retained as a historical snapshot.

## Goal

定义 repository-local official plugins 的稳定实现偏好、package 形态与 runtime 边界，使 official capability 可以长期演进，同时避免把链业务逻辑回填到 `chainbot runtime`。

## Scope

- 本设计适用于 `official-plugins/` 下的一方 official packages。
- 本设计不强制约束 third-party source repositories 或外部 plugin authors。
- 本设计定义 official plugin 的宿主边界，不定义 Ethereum 或 Solana 的具体业务 surface。

## Implementation Preference

- official plugins 优先使用 Rust 实现。
- official node packages 与 official trigger packages 优先作为 package-local Cargo binary crates 交付。
- official packages 优先采用以下 install contract：
  - `install_mode = "build_required"`
  - `build.kind = "cargo"`
  - `runtime = "bin"`
  - `entry_artifact` 指向 package-local `bin/` executable
- 编译产物必须保留在 plugin package 内部，不得越过 package containment boundary。
- 只有当 Rust 会显著放大 integration cost、阻断上游库复用、或无法满足宿主 contract 时，才允许 non-Rust exception。
- non-Rust exception 必须在对应的 design 或 plan 文档中显式记录，而不是实现阶段临时决定。

## Package Boundary

- each official package remains a normal ChainBot plugin package.
- source install 继续通过 prepare、staging、swap、revalidation 流程交付 official packages。
- official package identity 继续由 directory name 和 package-local `config.toml` 决定。
- official binaries、assets、build outputs 都必须留在 package root 内部。

## Runtime Boundary

- chain-specific logic 必须保留在 official plugins 内部，包括：
  - RPC request shaping
  - chain data decoding
  - transaction assembly
  - signing payload construction
  - signing algorithm selection and execution
  - confirmation polling strategy
  - listener cursor encoding
  - live listener event filtering and normalization
  - retry and backoff constants
  - provider-specific compatibility handling
- `chainbot runtime` 保持 thin control plane，只负责 generic platform concerns：
  - package discovery 与 source install orchestration
  - workflow scheduling 与 node dispatch
  - process / MCP host lifecycle
  - execution-time `secret ref` resolution、plaintext secret injection、与 redaction
  - accepted-event durability 与 listener supervision
  - stable CLI 与 catalog read models

## Secret Handling Boundary

- official signed operations may depend on host-resolved secrets.
- operator-owned plugin activation bindings are defined by `docs/decisions/CHAINBOT_PLUGIN_ACTIVATION_CONFIG_DESIGN.md`.
- `chainbot runtime` may resolve configured `secret ref` values at execution time and pass plaintext secret material into the plugin request.
- secret resolution in runtime does not imply chain-aware signing behavior in runtime.
- official plugins remain responsible for consuming the injected secret material and performing chain-specific signing locally.
- `chainbot runtime` must not embed Ethereum-specific、Solana-specific、or any other chain-specific signing algorithm.

## Runtime Change Rule

- 只有满足以下任一条件时，才允许修改 `chainbot runtime`：
  - 某项能力无法在 plugin-local code 中表达，且会破坏现有 package、state、或 trigger durability invariant
  - 所有 official plugins 都会重复实现同一类 host-side safety 或 orchestration logic
  - 某项保证必须由 host enforced，而不能依赖 plugin convention
- 不允许为了单条链的 protocol 细节，把 Ethereum 或 Solana 分支逻辑直接写进 scheduler、generic plugin dispatch、或 generic trigger acceptance。

## Non-Goals

- 不把 chain business workflows 移入 `chainbot runtime`。
- 不要求 runtime 解析 chain-specific checkpoint payloads。
- 不在 generic execution path 中加入 Ethereum-only 或 Solana-only protocol branches。
- 不在 runtime 中定义 official plugin shared retry 或 backoff constants。
- V1 不要求 official chain trigger plugins 实现 historical replay 或 reconnect backfill。
- 不要求 third-party plugins 继承 official implementation preference。

## Rationale

- Rust 与 Cargo build-to-bin 形态最符合当前 repository 的 workspace、tooling、和 official package install contract。
- 把链语义留在 official plugins 内部，可以避免 runtime 因 protocol 差异而持续膨胀。
- 把 secret resolution、durability、dispatch、catalog 这些跨插件共享的宿主能力留在 runtime，可以减少重复实现并保留统一安全边界。

## Change Triggers

- 改变 official plugin 的首选实现语言或 build flow。
- 改变 official package 的 install contract 或 package containment rule。
- 改变 `chainbot runtime` 与 official plugins 的职责边界。
- 引入新的 official plugin class，使现有 boundary 不再成立。
