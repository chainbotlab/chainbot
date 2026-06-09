# AGENTS.md

## Scope
- Position: Repository-local official plugin package catalog.
- Owns: Standalone plugin crates under package `crate/`, install/build manifests, and source-catalog discoverability.
- Excludes: Protocol-specific behavior leaking into generic runtime paths.

## Constraints
- Packages under `official-plugins/` are standalone Cargo crates, not root workspace members.
- Keep provider, exchange, and chain semantics inside the plugin packages.
- Align `config.toml`, `crate/Cargo.toml`, source entrypoints, tests, and declared build outputs.
- External node packages honor the `node.exec.v2` request/response contract.
- External trigger packages honor the `trigger.exec.v1` start/ready/event protocol.
- Re-check the top-level source catalog whenever a package path or summary changes.

## Members
- `aster-node/`, `aster-trigger/`: Aster official plugin packages.
- `binance-node/`, `binance-trigger/`: Binance official plugin packages.
- `bitget-node/`, `bitget-trigger/`: Bitget official plugin packages.
- `bybit-node/`, `bybit-trigger/`: Bybit official plugin packages.
- `gate-node/`, `gate-trigger/`: Gate official plugin packages.
- `okx-node/`, `okx-trigger/`: OKX official plugin packages.
- `hyperliquid-node/`, `hyperliquid-trigger/`: Hyperliquid official plugin packages.
- `eth-node/`, `eth-trigger/`: Ethereum official plugin packages.
- `solana-node/`, `solana-trigger/`: Solana official plugin packages.
- `jupiter-node/`: Jupiter swap API and Solana swap transaction helper plugin package.
- `uniswap-node/`: Uniswap V2 Router02-compatible quote, price polling, and swap plugin package.
- `uniswap-trigger/`: Uniswap V2 Router02-compatible price-threshold listener plugin package.
- `pancakeswap-node/`: PancakeSwap V3 exact-input swap plugin package; V2/fork-compatible routing stays in `uniswap-node/`.
- `raydium-node/`: Raydium Trade API quote and swap transaction helper plugin package.
- `sanctum-node/`: Sanctum LST metadata and swap order helper plugin package.
- `http-node/`: Outbound HTTP node reference implementation.
- `build-official-plugin/`, `echo-official-plugin/`: Source-install reference packages.

## Docs
- `../chainbot-plugin-index.toml`: Top-level plugin source catalog.
- `../docs/decisions/CHAINBOT_OFFICIAL_PLUGIN_DESIGN.md`: Official-plugin/runtime ownership boundary.
- `../docs/decisions/CHAINBOT_PLUGIN_ACTIVATION_CONFIG_DESIGN.md`: Activation secret contract for external plugins.
- `../docs/archive/2026-03-31-001-feat-eth-solana-official-plugins-plan.md`: Prior official-plugin rollout precedent.
- `../docs/archive/2026-04-01-001-feat-eth-solana-sdk-plugins-plan.md`: SDK-backed plugin implementation precedent.
