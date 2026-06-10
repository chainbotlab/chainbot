# Changelog

All notable changes to chainbot.

## [0.1.1.0] - 2026-06-09

### Added
- Bridge shared crates: `bridge-node-core` and `bridge-action-core` with destination policy validation (SSRF protection, IP blocklists, origin binding)
- Route/quote bridge API plugins: LI.FI, Squid, Rango, Across, deBridge — each wraps bridge-node-core for cross-chain route discovery
- Unsigned bridge-action plugins: Stargate, Arbitrum, Base, Polygon, LayerZero, Wormhole, USDT0, CCIP, Axelar, Hyperlane — each wraps bridge-action-core with protocol-specific calldata encoding
- Hyperliquid Bridge2 EIP-712 typed-data operations: prepare_deposit, prepare_withdraw3, prepare_deposit_with_permit
- 14 protocol-specific ABI calldata encodings across OP Stack, Polygon POS, Arbitrum Gateway, LayerZero OFT/Stargate, Wormhole NTT, Axelar ITS/GMP, CCIP, and Hyperlane warp/mailbox
- All bridge plugins registered in chainbot-plugin-index.toml with catalog descriptions

### Changed
- Hyperliquid-node plugin summary updated to "Info API and Bridge2"
