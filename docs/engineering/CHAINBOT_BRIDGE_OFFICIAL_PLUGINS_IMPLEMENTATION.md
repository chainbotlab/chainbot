# ChainBot Bridge Official Plugins Implementation

## Scope

This record covers the first official bridge-plugin implementation pass:

- Route/quote API plugins: `lifi-node`, `squid-node`, `rango-node`, `across-node`, `debridge-node`.
- Unsigned bridge-action plugins: `stargate-node`, `layerzero-node`, `wormhole-node`, `ccip-node`, `usdt0-node`, `axelar-node`, `hyperlane-node`, `arbitrum-bridge-node`, `base-bridge-node`, `polygon-bridge-node`.
- `hyperliquid-node` Bridge2 enhancement.

All packages stay under `official-plugins/` as standalone Cargo crates and are discoverable through `chainbot-plugin-index.toml`.

## Implementation Boundary

API-backed route and quote providers use `bridge-node-core`, a shared internal crate that implements:

- `node.exec.v2` request and JSON-RPC response handling.
- Provider base URL override with activation `allowed_origins` binding.
- Basic destination policy checks for custom HTTP origins.
- Provider-specific operation declarations in each plugin crate.

Contract-backed bridge providers use `bridge-action-core`, a shared internal crate that implements:

- `node.exec.v2` request and JSON-RPC response handling.
- A normalized `unsigned_action` output with `chain_id`, `to`, `data`, `value`, `action_kind`, and provider metadata.
- Protocol-specific ABI calldata for stable bridge entrypoints, with caller-supplied calldata retained only where the public surface is version- or deployment-specific.

The bridge-action plugins intentionally do not sign, broadcast, or custody keys. They prepare auditable action envelopes that can be handed to signer/RPC nodes. This preserves the runtime boundary while allowing individual protocol paths to gain tested calldata encoders over time.

## Provider Notes

- LI.FI, Squid, Rango, Across, and deBridge expose HTTP route/quote or transaction-preparation APIs and return provider payloads directly.
- Arbitrum, Base, and Polygon canonical bridge plugins generate calldata for their stable bridge entrypoints.
- Stargate, LayerZero, and USDT0 generate calldata for LayerZero/OFT `send`, `sendToken`, `quoteSend`, and `setPeer` entrypoints.
- Axelar and Hyperlane generate calldata for Interchain Token Service, GMP Gateway, Warp Route, and Mailbox dispatch entrypoints.
- CCIP generates calldata for Router `ccipSend(uint64, Client.EVM2AnyMessage)`. CCIP TokenPool admin remains caller-supplied because pool administration is not a single stable cross-pool entrypoint.
- Wormhole generates calldata for NTT Manager `transfer` basic/advanced overloads and Core Bridge `publishMessage`.
- Hyperliquid Bridge2 support adds Arbitrum USDC deposit calldata generation, withdraw3 typed-data preparation, and deposit-with-permit typed-data preparation to the existing `hyperliquid-node`. The deposit-with-permit unsigned action still accepts caller-supplied calldata for the final batched bridge call because the plugin's stable contribution is the permit typed data, not a protocol-wide batch ABI.

## ABI Sources

- Chainlink CCIP `IRouterClient.ccipSend` and `Client.EVM2AnyMessage`: `https://docs.chain.link/ccip/api-reference/evm/v1.6.1/i-router-client` and `https://docs.chain.link/ccip/api-reference/evm/v1.6.1/client`.
- Wormhole NTT Manager `transfer`: `https://wormhole.com/docs/products/token-transfers/native-token-transfers/reference/manager/evm/`.
- Wormhole Core Bridge `publishMessage`: `https://wormhole.com/docs/products/messaging/reference/core-contract-evm/`.

## Validation

Compile-level validation was performed with `cargo test --offline --no-run --manifest-path ...` for every new plugin crate and the enhanced `hyperliquid-node`.

ABI encoder tests in `bridge-action-core` compare generated calldata against local `cast calldata` output for the implemented stable entrypoints.

Runtime HTTP mock tests exist for the API-backed plugins, but this execution environment rejects local listener binding with `Operation not permitted`; those tests should be run in a normal developer environment.
