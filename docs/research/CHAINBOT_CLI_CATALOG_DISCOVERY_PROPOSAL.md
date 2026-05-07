# ChainBot CLI Catalog Discovery Proposal

## Goal

把 ChainBot 新增的 builtin nodes、builtin triggers、已安装 plugins，以及 plugin 的调用/事件结构，整理成一个对人类和 AI agent 都稳定可消费的 CLI discoverability surface。

这份文档是**实现规格**，面向即将开始的实现工作，不是最终稳定设计，也不是完成后的实现记录。

## Problem Statement

当前 CLI 已经能展示：

- 命令帮助
- root/status/observe
- 已配置 trigger 列表

但当前 CLI **不能直接回答**下面这些问题：

- ChainBot 内置了哪些 builtin nodes？
- ChainBot 内置了哪些 builtin triggers？
- 当前 root 装了哪些 plugins？
- external node plugin 需要什么输入、会产出什么输出？
- external trigger plugin 会产出什么事件结构？

这会让两类使用者都要额外读源码或示例目录：

- 只安装 CLI 的 operator
- 依赖结构化命令输出生成配置的 AI agent

## Scope

本次 proposal 包含：

1. 新增 `chainbot catalog list/show` 命令族
2. 为 CLI 建立独立的 catalog read model
3. 为 builtin nodes 和 builtin triggers 建立显式 descriptor source
4. 为 plugin manifest 增加可选 richer metadata
   - external node: `operations`
   - external trigger: `event_schema`
5. 在 `status` 中增加轻量 plugin summary
6. 在 `help` 中增加 catalog discoverability guidance
7. 新增独立 CLI integration tests 和 descriptor completeness tests

## Non-Goals

- 动态执行 plugin 来推断 schema
- 把 `status` 扩展成完整 discovery command
- 从 plugin 源码自动生成 `operations` 或 `event_schema`
- 重做 CLI 通用输出系统、配色、pager 或 verbosity 模型
- 在本 proposal 中修改稳定 design doc

## Current Constraints

实现必须遵守这些当前仓库事实：

- `docs/decisions/CHAINBOT_CLI_DESIGN.md` 定义了当前稳定 CLI surface
- `cli.rs` 已经承担 parser + help + renderer 角色，但文件较大
- builtin node kinds 当前来自 `crates/chainbot/src/builtins/nodes/registry.rs`
- builtin trigger kinds 当前来自 `crates/chainbot/src/builtins/triggers/registry.rs`
- installed plugins 当前由 `RootDefinitionBundle.plugins` 加载
- `PluginManifest` 当前已有 `input_schema` / `output_schema`
- external trigger runtime 当前只有 host protocol，没有 plugin-specific event schema
- 当前 JSON 输出遵循“默认 human-readable，`--json` 明确切换”的模式

## Chosen Direction

### Command Surface

新增：

```text
chainbot catalog list [--json] [--kind <builtin_node|builtin_trigger|plugin>]
chainbot catalog show <reference> [--json]
```

其中 `reference` 使用统一格式：

```text
builtin_node:builtin.data.merge
builtin_trigger:webhook
plugin:quote-node-plugin
```

### Responsibility Split

- `catalog`: 完整 discoverability surface
- `status`: 只提供轻量 plugin summary
- `help`: 只负责导航，不重复 catalog 内容

### Public Contract Rule

`catalog --json` **不得**直接序列化内部类型：

- `PluginManifest`
- `TriggerDefinition`
- runtime registry internals

而是通过专门设计的 CLI-facing read model 输出稳定字段。

## Data Flow

```text
                +---------------------------+
                | builtin node descriptors  |
                +-------------+-------------+
                              |
                +-------------v-------------+
                |   catalog read model      |
                +-------------+-------------+
                              |
                +-------------v-------------+
RootDefinition  | builtin trigger           |   CLI parser
Bundle.plugins +-> descriptors + plugin     +-> `catalog list/show`
                | manifest projection       |
                +-------------+-------------+
                              |
              +---------------+----------------+
              |                                |
    +---------v---------+            +---------v---------+
    | human text render |            | JSON render       |
    +-------------------+            +-------------------+
```

## Module Layout

选择**独立 catalog module**，避免继续膨胀 `cli.rs`。

推荐布局：

```text
crates/chainbot/src/
|- catalog.rs
|- builtins/
|  |- nodes/
|  |  `- catalog.rs
|  `- triggers/
|     `- catalog.rs
`- plugin/
   `- contract.rs   # richer metadata types live here
```

### Why this layout

- `cli.rs` 只保留 argv parsing、subcommand routing、top-level command help
- builtin descriptor 与 builtin registry 同域维护，减少漂移
- plugin metadata 仍然由 plugin contract 拥有，不把 schema 逻辑散到 CLI 层

## Read Model

### Core enums

```rust
pub enum CatalogEntryKind {
    BuiltinNode,
    BuiltinTrigger,
    Plugin,
}

pub enum CatalogReference {
    BuiltinNode(String),
    BuiltinTrigger(String),
    Plugin(String),
}
```

### List view

```rust
pub struct CatalogListOutput {
    pub builtin_nodes: Vec<BuiltinNodeSummary>,
    pub builtin_triggers: Vec<BuiltinTriggerSummary>,
    pub plugins: Vec<PluginSummary>,
}
```

### Detail view

```rust
pub enum CatalogDetail {
    BuiltinNode(BuiltinNodeDetail),
    BuiltinTrigger(BuiltinTriggerDetail),
    Plugin(PluginDetail),
}

pub struct CatalogShowOutput {
    pub reference: String,
    pub detail: CatalogDetail,
}
```

## Descriptor Model

### Builtin node descriptors

每个 builtin node 维护一份显式 descriptor：

```rust
pub struct BuiltinNodeDescriptor {
    pub kind: &'static str,
    pub summary: &'static str,
    pub operations: &'static [&'static str],
    pub inputs: &'static [CatalogFieldDescriptor],
    pub outputs: &'static [CatalogFieldDescriptor],
}
```

### Builtin trigger descriptors

```rust
pub struct BuiltinTriggerDescriptor {
    pub source: &'static str,
    pub summary: &'static str,
    pub params: &'static [CatalogFieldDescriptor],
    pub payload: &'static [CatalogFieldDescriptor],
    pub mode: BuiltinTriggerMode,
}

pub enum BuiltinTriggerMode {
    Poll,
    Ingress,
}
```

### Plugin metadata

在 `PluginManifest` 上新增可选字段：

```toml
[[operations]]
name = "normalize"
summary = "Normalize quote payload"
input_schema = ["symbol", "token"]
output_schema = ["decision"]

[event_schema]
summary = "Market tick payload"
fields = ["symbol", "price", "venue"]
```

Rust contract 方向：

```rust
pub struct PluginOperationDescriptor {
    pub name: String,
    pub summary: Option<String>,
    pub input_schema: Vec<String>,
    pub output_schema: Vec<String>,
}

pub struct PluginEventSchemaDescriptor {
    pub summary: Option<String>,
    pub fields: Vec<String>,
}
```

### Backward compatibility

- external node plugin 若没有 `operations`，detail fallback 到 manifest-level `input_schema` / `output_schema`
- external trigger plugin 若没有 `event_schema`，detail fallback 到 protocol summary，并明确标记 `schema_status = "protocol_only"`

## JSON Contract

### `chainbot catalog list --json`

```json
{
  "builtin_nodes": [
    {
      "reference": "builtin_node:builtin.data.merge",
      "kind": "builtin.data.merge",
      "summary": "Merge objects into a single result"
    }
  ],
  "builtin_triggers": [
    {
      "reference": "builtin_trigger:webhook",
      "source": "webhook",
      "summary": "Receive JSON events over HTTP"
    }
  ],
  "plugins": [
    {
      "reference": "plugin:quote-node-plugin",
      "plugin_id": "quote-node-plugin",
      "kind": "external_node",
      "entrypoint": "node.exec.v1",
      "capabilities": ["node:execute"]
    }
  ]
}
```

### `chainbot catalog list --kind plugin --json`

```json
{
  "plugins": [
    {
      "reference": "plugin:quote-node-plugin",
      "plugin_id": "quote-node-plugin",
      "kind": "external_node",
      "entrypoint": "node.exec.v1",
      "capabilities": ["node:execute"]
    }
  ]
}
```

### `chainbot catalog show plugin:quote-node-plugin --json`

```json
{
  "reference": "plugin:quote-node-plugin",
  "detail": {
    "kind": "plugin",
    "plugin_id": "quote-node-plugin",
    "plugin_kind": "external_node",
    "entrypoint": "node.exec.v1",
    "capabilities": ["node:execute"],
    "schema_status": "declared",
    "operations": [
      {
        "name": "normalize",
        "summary": "Normalize quote payload",
        "input_schema": ["symbol", "token"],
        "output_schema": ["decision"]
      }
    ]
  }
}
```

### `chainbot catalog show plugin:market-trigger-plugin --json`

```json
{
  "reference": "plugin:market-trigger-plugin",
  "detail": {
    "kind": "plugin",
    "plugin_id": "market-trigger-plugin",
    "plugin_kind": "external_trigger",
    "entrypoint": "trigger.exec.v1",
    "capabilities": ["trigger.listen.event"],
    "schema_status": "protocol_only",
    "protocol": {
      "start_message": ["protocol_version", "trigger_id", "source", "params", "resume_checkpoint"],
      "event_message": ["checkpoint", "event_key", "occurred_at_ms", "payload", "dedup_key", "cooldown_key"]
    }
  }
}
```

### Stability rule

- 允许新增字段
- 不重命名已有字段
- 不移除已发布字段
- `status --json` 只允许新增 `plugins` summary，不修改已有 top-level keys

## Human-readable Output

### `catalog list`

```text
Builtin nodes
  builtin_node:builtin.data.merge       Merge objects into a single result
  builtin_node:builtin.data.template    Render string templates

Builtin triggers
  builtin_trigger:cron                  Emit scheduled minute-slot events
  builtin_trigger:webhook               Receive JSON events over HTTP

Installed plugins
  plugin:quote-node-plugin              external_node   entrypoint=node.exec.v1
  plugin:market-trigger-plugin          external_trigger entrypoint=trigger.exec.v1
```

### `catalog show`

```text
Catalog entry
  reference: plugin:quote-node-plugin
  kind: external_node
  entrypoint: node.exec.v1

Operations
  normalize
    inputs: symbol, token
    outputs: decision
```

## `status` Changes

`status` 只增加轻量 summary：

### Human output

```text
Plugins
  installed=2 external_node=1 external_trigger=1
  use `chainbot catalog list` for capability details
```

### JSON output

```json
{
  "plugins": {
    "installed_count": 2,
    "external_node_count": 1,
    "external_trigger_count": 1
  }
}
```

该字段追加到现有 `status --json` 顶层，不修改已有字段。

## `help` Changes

### General help

新增：

```text
chainbot catalog list [--json] [--kind <builtin_node|builtin_trigger|plugin>]
chainbot catalog show <reference> [--json]
```

### AI workflow hints

新增：

- use `chainbot catalog list --json` when an agent needs a machine-readable capability inventory
- use `chainbot catalog show <reference> --json` for one entry's callable or event structure

### Command help

新增 `help catalog` 卡片，并在 `help status` / `help trigger` / `help validate` 中加入 `See also: catalog`

## File-by-File Change Plan

### Rust source

1. `crates/chainbot/src/cli.rs`
   - 增加 `catalog` argv parsing
   - 增加 `help catalog`
   - 调用 `catalog` module renderers
   - 在 `status` 中追加 plugin summary

2. `crates/chainbot/src/catalog.rs`
   - 定义 catalog read model
   - 实现 `list/show`
   - 实现 human/JSON render
   - 解析 `reference`

3. `crates/chainbot/src/builtins/nodes/catalog.rs`
   - 定义 builtin node descriptors
   - 导出 descriptor slice

4. `crates/chainbot/src/builtins/nodes/mod.rs`
   - 暴露 node catalog module

5. `crates/chainbot/src/builtins/triggers/catalog.rs`
   - 定义 builtin trigger descriptors
   - 导出 descriptor slice

6. `crates/chainbot/src/builtins/triggers/mod.rs`
   - 暴露 trigger catalog module

7. `crates/chainbot/src/plugin/contract.rs`
   - 新增 `operations`
   - 新增 `event_schema`
   - 更新 validation
   - 保持 backward compatibility

8. `crates/chainbot/src/lib.rs`
   - 暴露新增 module（如当前模块图需要）

### Tests

9. `crates/chainbot/tests/catalog_surface.rs`
   - 新增 catalog integration coverage

10. `crates/chainbot/tests/cli_surface.rs`
    - 只补 `status` summary 与 `help` 跳转回归

11. `crates/chainbot/src/builtins/nodes/catalog.rs` tests
    - descriptor completeness / uniqueness

12. `crates/chainbot/src/builtins/triggers/catalog.rs` tests
    - descriptor completeness / uniqueness

13. `crates/chainbot/src/plugin/contract.rs` tests
    - richer metadata validation + fallback compatibility

### Docs

14. `docs/decisions/CHAINBOT_CLI_DESIGN.md`
    - 实现完成后再更新稳定命令面

15. `README.md`
    - 实现完成后补 catalog usage

16. `examples/plugin-integrations/`
    - 实现完成后补 richer metadata 示例

## Rollout Sequence

推荐顺序：

```text
Step 1  plugin contract richer metadata
Step 2  builtin descriptors
Step 3  catalog read model + parser
Step 4  catalog renderers
Step 5  status plugin summary
Step 6  help integration
Step 7  tests
Step 8  docs/examples update
```

### Why this order

- 先把 metadata source 建好，避免 CLI 实现反向驱动 contract
- 再实现 catalog read model，确保 renderer 只消费稳定中间层
- 最后再接 help/status/docs，降低回归扩散范围

## Test Plan

### New integration coverage

`crates/chainbot/tests/catalog_surface.rs` 必须覆盖：

1. `catalog list` human output
2. `catalog list --json`
3. `catalog list --kind builtin_node`
4. `catalog list --kind builtin_trigger`
5. `catalog list --kind plugin`
6. `catalog show builtin_node:*`
7. `catalog show builtin_trigger:*`
8. `catalog show plugin:*` for external node with `operations`
9. `catalog show plugin:*` for external trigger with `event_schema`
10. fallback behavior for old plugins without richer metadata
11. malformed reference error
12. unknown reference error
13. unsupported `--kind` error
14. empty plugin root output

### Existing CLI regression coverage

`crates/chainbot/tests/cli_surface.rs` 只补：

- `help` includes `catalog`
- `help catalog` card shape
- `status` human output includes plugin summary
- `status --json` includes plugin summary while old fields remain unchanged

### Descriptor correctness tests

- every registered builtin node kind has one descriptor
- every registered builtin trigger kind has one descriptor
- no duplicate descriptor keys
- detail renderer prints copy-pasteable references

## Failure Modes and Handling

### Missing descriptor for a registered builtin

- Failure: builtin exists in execution plane but missing from catalog
- Handling: unit test must fail on mismatch

### Old plugin lacks richer metadata

- Failure: `catalog show plugin:*` cannot render richer detail
- Handling: fallback to protocol summary or manifest-level schema

### User passes malformed reference

- Failure: cannot determine entry kind or target id
- Handling: return usage error with expected `kind:value` format

### User passes unknown reference

- Failure: requested builtin/plugin not found
- Handling: return next-step guidance to `chainbot catalog list`

### `status --json` breaks old automation

- Failure: consumers depend on old keys
- Handling: append-only summary field, no existing key changes

## Open Decisions Already Locked By Review

这几项不再重新讨论，按 review 结论执行：

1. 使用独立 catalog read model
2. `status` 只做轻量 summary
3. discovery 不走 runtime registry construction path
4. descriptor 只维护一份真相源
5. richer plugin metadata 并入本次范围，而不是延期

## Implementation Exit Criteria

实现完成后，必须满足：

1. `chainbot help` 能看到 `catalog`
2. `chainbot catalog list/show` human output 可读
3. `chainbot catalog list/show --json` 可稳定解析
4. builtin nodes / triggers 能完整发现
5. installed plugins 能完整发现
6. richer plugin metadata 在声明时可见，未声明时 fallback 正常
7. `status` 增加轻量 plugin summary 且不破坏旧 contract
8. 所有新增/修改测试通过
9. 稳定 design / README / examples 在实现完成后同步更新

## Recommended Next Step

按本 proposal 先实现 Rust module 和 tests，再在通过验证后更新：

- `docs/decisions/CHAINBOT_CLI_DESIGN.md`
- `README.md`
- `examples/plugin-integrations/`

避免先改稳定文档，再让实现追着文档补齐。
