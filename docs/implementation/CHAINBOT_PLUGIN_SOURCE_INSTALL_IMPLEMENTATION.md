# ChainBot Plugin Source Install Implementation

## Implemented Surface

```text
chainbot plugin source list <github|git> <target> [--ref <git-ref>] [--json]
chainbot plugin source show <github|git> <target> --plugin <plugin_id> [--ref <git-ref>] [--json]
chainbot plugin install <github|git> <target> [--ref <git-ref>] [--plugin <plugin_id>] [--force]
```

## Module Layout

- `crates/chainbot/src/plugin/source/locator.rs`
- `crates/chainbot/src/plugin/source/manifest.rs`
- `crates/chainbot/src/plugin/source/transport.rs`
- `crates/chainbot/src/plugin/source/discover.rs`
- `crates/chainbot/src/plugin/source/prepare.rs`
- `crates/chainbot/src/plugin/source/install.rs`
- `crates/chainbot/src/plugin/source/fs.rs`
- `crates/chainbot/src/app/cli/view/plugin_source.rs`

## Current Notes

- `git` transport is covered by integration tests through local git repositories.
- `github` transport uses source archive download and unzip into a temporary repo root.
- source metadata extraction now reads `config.toml[source]`; legacy `source.toml` parser has been removed.
- install success is gated by post-swap `load_root_definition_bundle(...)` revalidation.
