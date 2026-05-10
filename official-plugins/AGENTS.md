# AGENTS.md

## Scope
- Repository-local official plugin packages and their install/build manifests.
- Standalone plugin crates under each package `crate/` subtree.
- Source-catalog entries that make official packages discoverable through `chainbot-plugin-index.toml`.

## Constraints
- Packages under `official-plugins/` are standalone Cargo crates, not root workspace members.
- Keep provider, exchange, and chain semantics inside the plugin packages; do not move protocol-specific behavior into `crates/chainbot` generic runtime paths.
- Each package must keep `config.toml`, `crate/Cargo.toml`, source entrypoints, tests, and declared build outputs aligned.
- External node packages must honor the `node.exec.v2` request/response contract. External trigger packages must honor the `trigger.exec.v1` start/ready/event protocol.

## Members
- `binance-node/`: Official Binance REST node toolkit for spot, USDⓈ-M, and COIN-M operations.
- `binance-trigger/`: Official Binance market-stream and user-stream trigger toolkit.
- `build-official-plugin/`: Build-required reference package for source install flow.
- `echo-official-plugin/`: Direct-install reference package for source discovery and activation.
- `eth-node/`: Official Ethereum node toolkit reference implementation.
- `eth-trigger/`: Official Ethereum trigger toolkit reference implementation.
- `http-node/`: Official outbound HTTP node plugin reference implementation.
- `okx-node/`: Official OKX REST node toolkit for public market data and private account balance reads.
- `okx-trigger/`: Official OKX public market-stream and private account-stream trigger toolkit.
- `solana-node/`: Official Solana node toolkit reference implementation.
- `solana-trigger/`: Official Solana trigger toolkit reference implementation.

## Dependencies
- `../chainbot-plugin-index.toml`: Top-level plugin source catalog.
- `../docs/decisions/CHAINBOT_OFFICIAL_PLUGIN_DESIGN.md`: Official-plugin/runtime ownership boundary.
- `../docs/decisions/CHAINBOT_PLUGIN_ACTIVATION_CONFIG_DESIGN.md`: Activation secret contract for external plugins.

## Docs
- `../docs/archive/2026-03-31-001-feat-eth-solana-official-plugins-plan.md`: Prior official-plugin rollout precedent.
- `../docs/archive/2026-04-01-001-feat-eth-solana-sdk-plugins-plan.md`: SDK-backed plugin implementation precedent.

## Review Triggers
- Update this file when a package is added, removed, or materially changes responsibility.
- Update package manifests and this directory map when build artifact names, activation slots, or runtime entrypoints change.
- Re-check `chainbot-plugin-index.toml` whenever a package path or summary changes.
