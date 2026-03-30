# ChainBot Plugin Source Install Design

## Goal

定义 ChainBot 远端插件 source discoverability 与 install 的稳定架构边界、source repo contract、和安装安全语义。

## Architecture

- `catalog` 继续描述当前 root 已安装能力。
- `plugin source` 描述远端 source repo 中可安装插件。
- `plugin install` 是唯一会写当前 root 的 operator-facing 入口。
- 新能力作为 `src/plugin/source/` 内部 vertical slice 存在，而不是拆到独立顶层 infra 目录。

## Contracts

- source repo 可以是：
  - 单插件仓库：repo root 包含 `config.toml`，并在 `config.toml[source]` 中声明 install metadata
  - 多插件仓库：repo root 包含 `chainbot-plugin-index.toml`
- package-local install metadata 必须声明在 `config.toml[source]`；legacy `source.toml` 不再支持。
- runtime 继续只依赖 `<root>/plugins/<plugin_id>/config.toml` 作为 canonical plugin package entrypoint。

## Safety

- source list/show 只读，不写 root。
- install 必须经过 prepare、staging、swap、root revalidation。
- 已存在目标目录时默认拒绝覆盖，仅 `--force` 允许替换。
- 替换必须带 backup 与 rollback 语义。
