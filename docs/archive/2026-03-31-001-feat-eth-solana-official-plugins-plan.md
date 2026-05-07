---
title: feat: Add official Ethereum and Solana plugin surfaces
type: feat
status: completed
date: 2026-03-31
---

# feat: Add official Ethereum and Solana plugin surfaces

## Overview

Add official Ethereum and Solana capability surfaces on top of the existing ChainBot plugin, source install, external node, and external trigger runtime. V1 covers base chain reads, token reads, transfers, raw generic calls, and on-chain listeners while preserving the repository's package-centric install model and trigger durability guarantees.

## Problem Frame

ChainBot already has mature seams for plugin installation, discovery, execution, and listener supervision, but it has no chain-native semantics yet. This work is not just adding two plugin directories; it introduces Ethereum and Solana as first-class official capabilities without bypassing the current package, catalog, secret, and trigger-acceptance boundaries.

## Requirements Trace

- R1. Official capability must be presented separately by chain rather than as one undifferentiated multichain surface.
- R2. V1 must ship as an all-in toolkit covering read, write, and listener capabilities rather than a read-only or listener-only slice.
- R3. The product shape is a general chain toolkit, not a protocol-specific business workflow layer.
- R4. V1 must cover base chain reads, token reads, transfers, generic calls, and on-chain listeners.
- R5. Signed operations must use managed secrets.
- R6. Network connectivity must use user-supplied endpoints rather than a curated network whitelist.
- R7. Listener support must cover both state-change and event-log mental models.
- R8. Generic calls may expose raw passthrough capability, but raw writes must not bypass official safety boundaries.
- R9. Write operations must support both submit-only and confirm-wait semantics, defaulting to confirm-wait.
- R10. Preflight or simulation must remain optional rather than mandatory.

## Scope Boundaries

- No protocol-specific higher-level actions such as swap, LP, staking, or claim.
- No browser wallet or custody-only signing mode in V1.
- No curated network matrix or provider whitelist in V1.
- No attempt to collapse Ethereum and Solana into one shared payload or finality model.
- No signed catalog, publisher attestation, or provenance workflow in this feature.
- No effort to smooth over all provider quirks; V1 defines admission and failure policy only.
- No historical replay, reconnect backfill, or missed-event recovery for official chain listeners in V1; listeners emit live events only after startup.

## Context & Research

### Relevant Code and Patterns

- `chainbot-plugin-index.toml` defines the repository-local official plugin source index.
- `official-plugins/echo-official-plugin/config.toml` and `official-plugins/build-official-plugin/config.toml` show the canonical official package shape for direct-install and build-required packages.
- `crates/chainbot/src/infrastructure/config/mod.rs` defines root config decoding and root-owned stable config fields.
- `docs/decisions/CHAINBOT_OFFICIAL_PLUGIN_DESIGN.md` defines the official package implementation preference and thin runtime boundary.
- `docs/decisions/CHAINBOT_PLUGIN_ACTIVATION_CONFIG_DESIGN.md` defines operator-owned plugin activation secret bindings and execution-time injection semantics.
- `crates/chainbot/src/plugin/source/manifest.rs`, `discover.rs`, `prepare.rs`, and `install.rs` define the current source discovery and staged install boundary.
- `crates/chainbot/src/plugin/contract.rs` defines external node plugin manifest structure and catalog-facing metadata.
- `crates/chainbot/src/plugin/host.rs` and `crates/chainbot/src/app/runtime/execution.rs` define runtime node execution, execution-time secret resolution, and plugin error mapping.
- `crates/chainbot/src/domain/trigger/contract.rs`, `acceptance.rs`, `crates/chainbot/src/app/runtime/external_triggers/process_listener.rs`, and `supervisor.rs` define trigger lifecycle, checkpoints, dedup, and external listener supervision.
- `crates/chainbot/src/app/cli/view/catalog.rs` and `plugin_source.rs` define installed capability and remote source read models.

### Institutional Learnings

- Canonical plugin package identity is directory-based and package-local artifact containment is a hard safety boundary.
- `plugin source list/show` is intentionally separate from installed `catalog` discoverability and that split should remain intact.
- Listener correctness depends on durable accepted history and checkpoints rather than transient event buffers.
- Long-running listeners are lease-bound and must continue to route accepted events through `TriggerPlane` rather than creating a parallel truth source.
- Trigger extensibility should continue to use stable trigger core fields plus structured `params` for chain-specific configuration.

### External References

- Ethereum write flows should use managed local signing plus raw transaction broadcast rather than node-managed accounts.
- Solana write flows must treat recent blockhash expiry as a primary lifecycle state, not an edge case.
- Confirmation semantics must remain chain-specific: Ethereum uses included or safe or finalized style states while Solana uses processed or confirmed or finalized commitments.
- V1 official chain listeners may intentionally remain live-only and document that downtime gaps are out of scope.

## Key Technical Decisions

- **Use four official packages rather than one large package**: one node package and one trigger package per chain. This matches the repository's package-centric install and runtime model and avoids mixing node and trigger lifecycle assumptions.
- **Implement official packages as Rust build-to-bin packages by default**: repository-local official packages should use Rust, `cargo`, and package-local `bin/` artifacts unless a documented exception proves necessary.
- **Keep `chainbot runtime` thin**: runtime changes stay limited to generic host guarantees such as dispatch, execution-time secret resolution and injection, redaction, and accepted-event supervision; chain-specific RPC, signing algorithms, confirmation polling, and listener semantics remain package-local.
- **Keep plugin activation config root-owned**: operator-managed secret bindings live in `chainbot.toml`, keyed by `plugin_id`, rather than inside install-managed plugin package manifests.
- **Share runtime shell, not chain semantics**: Ethereum and Solana may reuse the same plugin and trigger host seams, but they must keep separate confirmation, listener cursor, signing, and payload semantics.
- **Define four node operation classes**: typed reads, typed transfers, raw reads, and raw writes. This preserves the requested passthrough capability while keeping signed writes inside a controlled execution path.
- **Place endpoints by lifecycle**: node operations receive endpoints as request inputs, while trigger listeners receive endpoints through trigger `params`. This fits the existing request-vs-listener boundary.
- **Keep managed secrets inside official signing paths**: raw writes cannot expose wallet management or remote signing methods that bypass signing, confirmation, or audit boundaries.
- **Inject activation secrets through a dedicated envelope**: host-resolved activation secrets stay separate from workflow node `input` and trigger `params`, so operator-owned bindings do not collide with request-scoped business input.
- **V1 official chain listeners stay live-only**: new listeners start from current live events and do not perform historical replay or reconnect backfill.
- **Ethereum `safe` remains fail-closed**: if a provider cannot satisfy requested `safe` semantics, the plugin returns a deterministic unsupported-confirmation failure rather than silently downgrading guarantees.
- **Raw passthrough output stays minimal**: `raw_read` returns `result` plus optional `metadata`; `raw_write` returns `status`, `transaction_id`, plus optional `metadata`.
- **Raw passthrough metadata remains plugin-defined in V1**: only top-level minimal fields are stable across chains; `metadata` subfields are documented per official plugin rather than standardized across chains.
- **Retry policy remains plugin-local**: transport retry and backoff constants are owned by each official Rust plugin rather than centralized in runtime.
- **Retry numbers remain implementation-tuned in V1**: the plan does not lock concrete retry or backoff numbers ahead of plugin-level validation.
- **Default confirmation remains chain-specific**: Ethereum defaults to `safe`; Solana defaults to `confirmed`; submit-only remains available as an explicit mode.
- **State-change and event-log listener surfaces both exist, but cursor and dedup rules stay chain-specific** rather than forcing one shared listener model.
- **Preflight remains optional** but becomes a first-class execution option rather than an implicit transport detail.

## Open Questions

### Resolved During Planning

- What is the package topology: Use one official node package and one official trigger package per chain.
- What implementation language and build flow should official packages prefer: use Rust crates built with `cargo` into package-local `bin/` artifacts by default.
- What is the signing boundary: `chainbot runtime` resolves configured secret references at execution time and injects plaintext secret values into plugin requests, while each official plugin owns the chain-specific signing algorithm.
- Where should plugin activation secret bindings live: keep them in root-owned `chainbot.toml` activation config keyed by `plugin_id`, separate from install-managed plugin manifests.
- How should activation secrets be delivered: inject them through a dedicated `activation.secrets` section shared by `ExternalNodePluginRequest` and `TriggerStartCommand`, instead of flattening them into node `input` or trigger `params`.
- What listener replay model should V1 use: none; official chain listeners emit live-only events after startup and do not perform historical replay or reconnect backfill.
- How should providers that do not expose Ethereum `safe` semantics behave: fail closed with a deterministic unsupported-confirmation error and require an explicit lower confirmation mode.
- What exact output field names should raw passthrough responses use: `raw_read` returns `result` plus optional `metadata`; `raw_write` returns `status`, `transaction_id`, plus optional `metadata`.
- How should raw passthrough metadata work in V1: keep `metadata` plugin-defined and document subfields per official plugin rather than standardizing them across chains.
- What retry and backoff constants should individual chain transports use: keep them plugin-local and tune the concrete numbers during implementation rather than locking them in the plan.
- How should raw passthrough be exposed: split it into `raw_read` and `raw_write` surfaces so writes stay inside official signer and confirmation policy.
- Where should endpoint configuration live: pass endpoints as node operation inputs and trigger listener `params`.
- What is the default confirmation behavior: keep submit-only and confirm-wait modes, with chain-specific confirm defaults.

### Deferred to Implementation

- What exact `metadata` subfields each official plugin should expose in examples and package-local docs.
- What concrete retry and backoff numbers each official plugin should ship with after implementation-time validation.

## High-Level Technical Design

> *This illustrates the intended approach and is directional guidance for review, not implementation specification. The implementing agent should treat it as context, not code to reproduce.*

Three existing repository lines remain intact and get extended in place:

1. **Official source and catalog line**
   - Register four official chain packages in `chainbot-plugin-index.toml`
   - Keep package-local `config.toml[source]` metadata
   - Continue to expose installable packages through `plugin source` and installed capabilities through `catalog`

2. **Official package implementation line**
   - Each official chain package is implemented as a Rust package-local binary crate by default
   - Source install uses `build_required` plus `cargo` and promotes package-local `bin/` artifacts
   - Chain-specific RPC, signing, confirmation, and listener behavior stays inside the package binary rather than the host runtime

3. **Runtime execution line**
   - Root-owned plugin activation config binds named secret refs to installed plugin ids
   - Node packages execute through `ExternalNodePluginHost`
   - Trigger packages execute through the existing external trigger runtime and `TriggerPlane`
   - Configured activation secret references are still resolved only at execution time and injected into plugin requests through a dedicated activation section
   - Chain-specific signing algorithms remain package-local
   - Listener delivery remains live-only in V1 while any stored checkpoint contents stay plugin-owned and host-opaque
   - Explicit backfill configuration, raw output metadata, and retry constants remain package-local concerns

## Implementation Units

- [x] **Unit 1: Add official Ethereum and Solana package topology**

**Goal:** Register the official chain packages and wire them through the existing source and install model.

**Requirements:** R1, R2, R4

**Dependencies:** None

**Files:**
- Modify: `chainbot-plugin-index.toml`
- Create: `official-plugins/eth-node-official-plugin/config.toml`
- Create: `official-plugins/eth-trigger-official-plugin/config.toml`
- Create: `official-plugins/solana-node-official-plugin/config.toml`
- Create: `official-plugins/solana-trigger-official-plugin/config.toml`
- Test: `crates/chainbot/tests/plugin_source_surface.rs`
- Test: `crates/chainbot/tests/plugin_install_surface.rs`
- Test: `crates/chainbot/tests/catalog_surface.rs`

**Approach:**
- Extend the official source index with four unambiguous packages.
- Keep node and trigger capabilities in separate packages to preserve lifecycle clarity.
- Reuse the current package-local source metadata and staged install model without special cases.

**Patterns to follow:**
- `official-plugins/echo-official-plugin/config.toml`
- `official-plugins/build-official-plugin/config.toml`
- `crates/chainbot/src/plugin/source/manifest.rs`

**Test scenarios:**
- Happy path: `plugin source list/show` exposes all four official chain packages with distinct summaries.
- Happy path: installing only one chain package updates installed `catalog` output without surfacing uninstalled packages.
- Error path: a post-swap validation failure rolls back the install cleanly.
- Integration: mixed install sets such as Ethereum-only or Solana-only still keep source and catalog output consistent.

**Verification:**
- Official source and installed catalog boundaries remain clear and package installation keeps the root valid.

- [x] **Unit 2: Extend plugin contract metadata for chain capability discoverability**

**Goal:** Introduce stable metadata for chain node and trigger capabilities without leaking internal manifest shape through CLI surfaces.

**Requirements:** R3, R4, R8, R9

**Dependencies:** Unit 1

**Files:**
- Modify: `crates/chainbot/src/plugin/contract.rs`
- Modify: `crates/chainbot/src/app/cli/view/catalog.rs`
- Modify: `crates/chainbot/src/app/cli/view/plugin_source.rs`
- Test: `crates/chainbot/tests/catalog_surface.rs`
- Test: `crates/chainbot/tests/cli_surface.rs`

**Approach:**
- Define capability metadata for typed reads, typed transfers, raw reads, raw writes, and trigger listener surfaces.
- Keep installed capability rendering on dedicated CLI read models rather than directly serializing internal manifest structures.
- Include enough metadata to distinguish state-change listeners from event-log listeners.

**Patterns to follow:**
- `crates/chainbot/src/plugin/contract.rs`
- `crates/chainbot/src/app/cli/view/catalog.rs`
- `docs/research/CHAINBOT_CLI_CATALOG_DISCOVERY_PROPOSAL.md`

**Test scenarios:**
- Happy path: installed catalog output shows chain-specific node and trigger capability summaries.
- Edge case: partial packages with incomplete capability metadata fail validation or render with explicit constraints.
- Error path: manifest metadata that disagrees with declared operations is rejected.
- Integration: `plugin source show` and `catalog show` describe the same package consistently at different lifecycle stages.

**Verification:**
- Chain package discoverability becomes stable and CLI-facing without exposing internal-only manifest contracts.

- [x] **Unit 3: Add thin host guardrails for managed-secret node execution**

**Goal:** Keep the runtime boundary thin while enforcing only generic host guarantees for official signed plugin operations.

**Requirements:** R5, R8, R9

**Dependencies:** Unit 2

**Files:**
- Modify: `crates/chainbot/src/plugin/host.rs`
- Modify: `crates/chainbot/src/app/runtime/execution.rs`
- Modify: `crates/chainbot/src/infrastructure/config/mod.rs`
- Modify: `crates/chainbot/src/secrets.rs`
- Create: `crates/chainbot/tests/chain_node_plugin_host.rs`
- Create: `crates/chainbot/tests/fixtures/ops/chain/`
- Test: `crates/chainbot/tests/config_loading.rs`
- Test: `crates/chainbot/tests/node_plugin_host.rs`

**Approach:**
- Enforce generic host-side rules for operations that declare managed signing rather than adding chain-specific branches to runtime.
- Load operator-owned activation secret bindings from `chainbot.toml` rather than plugin package manifests.
- Resolve configured signing secrets only at execution time, inject plaintext secret values into a dedicated activation section in the plugin request, and preserve redaction across outputs and errors.
- Reuse existing operation metadata such as `kind` and `default_confirmation` for generic dispatch only.
- Leave signing algorithms, endpoint validation, preflight, confirmation polling, and write lifecycle details inside the official Rust plugins.
- Keep retry and backoff constants package-local rather than adding shared runtime policy.

**Patterns to follow:**
- `crates/chainbot/src/plugin/host.rs`
- `crates/chainbot/src/app/runtime/execution.rs`
- `crates/chainbot/src/secrets.rs`
- `docs/decisions/CHAINBOT_OFFICIAL_PLUGIN_DESIGN.md`
- `docs/decisions/CHAINBOT_PLUGIN_ACTIVATION_CONFIG_DESIGN.md`

**Test scenarios:**
- Happy path: root config loads plugin activation secret bindings for an installed plugin.
- Error path: root config rejects activation bindings for an unknown plugin id.
- Error path: malformed activation `secret ref` fails root config validation.
- Happy path: a signed operation resolves an activation secret reference only at execution time.
- Happy path: the resolved plaintext secret is injected into the plugin request activation section without requiring runtime-side chain logic.
- Happy path: `default_confirmation` is injected when omitted by the caller.
- Error path: a managed-signing operation fails before plugin execution when a required activation binding is missing or cannot be resolved.
- Error path: plugin stderr or surfaced failure text that contains secret material is redacted.
- Integration: secret values never appear in user-visible output, stored payloads, or surfaced errors.

**Verification:**
- The runtime preserves secret-handling and dispatch invariants without owning chain RPC behavior.

- [x] **Unit 4: Reuse the existing external trigger runtime for live-only chain listeners**

**Goal:** Keep the external trigger runtime generic while validating that official chain listeners can emit live-only events through the existing acceptance and supervision model.

**Requirements:** R4, R6, R7

**Dependencies:** Unit 1, Unit 2

**Files:**
- Modify: `crates/chainbot/src/domain/trigger/contract.rs`
- Modify: `crates/chainbot/src/domain/trigger/acceptance.rs`
- Modify: `crates/chainbot/src/app/runtime/mod.rs`
- Modify: `crates/chainbot/src/app/runtime/external_triggers/process_listener.rs`
- Modify: `crates/chainbot/src/app/runtime/external_triggers/supervisor.rs`
- Create: `crates/chainbot/tests/chain_trigger_runtime.rs`
- Test: `crates/chainbot/tests/trigger_plane.rs`
- Test: `crates/chainbot/tests/ingress_runtime.rs`

**Approach:**
- Keep chain-specific listener configuration in trigger `params`.
- Represent state-change and event-log listeners as separate surfaces while preserving the same acceptance boundary.
- Treat checkpoint strings and dedup identities as plugin-owned opaque values that the host stores and replays.
- Allow V1 official chain trigger plugins to ignore `resume_checkpoint` and begin from current live subscription state.
- Keep host-side acceptance, dedup, and supervision generic without adding history replay semantics.

**Patterns to follow:**
- `docs/engineering/CHAINBOT_INGRESS_TRIGGER_IMPLEMENTATION.md`
- `crates/chainbot/src/domain/trigger/acceptance.rs`
- `examples/plugin-integrations/plugins/market-trigger-plugin/config.toml`

**Test scenarios:**
- Happy path: a fresh listener starts cleanly and emits only newly observed live events.
- Happy path: a restarted listener resumes live delivery without requiring historical replay.
- Error path: reconnect after endpoint outage restarts live delivery without attempting missed-event recovery.
- Error path: duplicate provider delivery is deduped rather than re-emitted.
- Integration: the accepted-but-not-started crash window behaves consistently with existing TriggerPlane acceptance semantics.
- Chain-specific: Ethereum and Solana listener plugins document that downtime gaps are out of scope for V1 live-only delivery.

**Verification:**
- Listener delivery remains rooted in live subscription plus TriggerPlane acceptance while historical replay stays out of scope for V1.

- [x] **Unit 5: Update CLI and read models for official chain discoverability**

**Goal:** Make the installable, installed, and runtime-visible official chain surfaces easy to understand through the existing CLI.

**Requirements:** R1, R2, R4

**Dependencies:** Unit 1, Unit 2

**Files:**
- Modify: `crates/chainbot/src/app/cli/parse.rs`
- Modify: `crates/chainbot/src/app/cli/commands.rs`
- Modify: `crates/chainbot/src/app/cli/help.rs`
- Modify: `crates/chainbot/src/app/cli/view/catalog.rs`
- Modify: `crates/chainbot/src/app/cli/view/status.rs`
- Modify: `crates/chainbot/src/app/cli/view/plugin_source.rs`
- Test: `crates/chainbot/tests/cli_surface.rs`
- Test: `crates/chainbot/tests/catalog_surface.rs`

**Approach:**
- Preserve the existing `plugin source` vs `catalog` split.
- Add chain-aware capability summaries to installed catalog and runtime status output.
- Update help text so official chain plugins follow the same source and install flow as all other packages.

**Patterns to follow:**
- `docs/decisions/CHAINBOT_CLI_DESIGN.md`
- `crates/chainbot/src/app/cli/view/plugin_source.rs`

**Test scenarios:**
- Happy path: source output shows official Ethereum and Solana packages clearly.
- Happy path: installed catalog output shows chain-specific node and trigger capabilities.
- Edge case: partial install sets such as Ethereum-only remain unambiguous.
- Integration: CLI help and read models keep the installed-vs-remote split intact.

**Verification:**
- Users can tell what is available remotely, what is installed locally, and what runtime surfaces each installed chain package exposes.

- [x] **Unit 6: Add canonical examples and minimal chain-facing docs**

**Goal:** Provide copyable examples and minimal docs for official chain plugin usage without introducing business-workflow abstractions.

**Requirements:** R2, R3, R4

**Dependencies:** Unit 3, Unit 4, Unit 5

**Files:**
- Create: `examples/eth-plugin-integrations/`
- Create: `examples/solana-plugin-integrations/`
- Modify: `examples/README.md`
- Modify: `docs/decisions/CHAINBOT_PLUGIN_SOURCE_INSTALL_DESIGN.md`
- Test: `crates/chainbot/tests/config_loading.rs`

**Approach:**
- Provide one node-oriented example and one trigger-oriented example per chain.
- Keep examples focused on toolkit primitives rather than protocol-specific business workflows.
- Document the raw surface boundary, optional preflight behavior, confirmation defaults, and user-supplied endpoint model.

**Patterns to follow:**
- `examples/plugin-integrations/`
- `examples/plugin-integrations/README.md`

**Test scenarios:**
- Happy path: example roots load under the canonical root contract.
- Edge case: examples remain valid when only one chain's packages are installed.
- Integration: examples, docs, and catalog naming remain consistent.

**Verification:**
- The repository includes a clear, copyable reference for how official chain packages are meant to be installed and used.

## System-Wide Impact

- **Interaction graph:** `plugin/source/*`, `plugin/contract.rs`, `plugin/host.rs`, `app/runtime/*`, `domain/trigger/*`, and `app/cli/*` all participate in the delivered surface.
- **Error propagation:** the plan must keep host-side secret handling and listener supervision generic while allowing package-local chain failures to surface through stable runtime error boundaries.
- **State lifecycle risks:** pending writes, listener checkpoints, accepted history, and secret redaction are the primary correctness-bearing state boundaries.
- **API surface parity:** source discovery, installed catalog output, runtime status, node execution, and trigger runtime all need compatible chain-facing terminology.
- **Integration coverage:** install rollback, write lifecycle recovery, live listener restart, accepted-but-not-started crash windows, and catalog/source parity all require integration tests, not only isolated unit tests.
- **Unchanged invariants:** source discovery remains separate from installed catalog; package-contained install safety remains intact; TriggerPlane acceptance remains the accepted-event truth source.

## Risks & Dependencies

| Risk | Mitigation |
|------|------------|
| Over-normalizing Ethereum and Solana hides critical semantics | Share only host and packaging shell; define chain semantics explicitly and separately |
| Raw writes bypass official safety boundaries | Split raw read and raw write surfaces and keep raw writes inside managed signing, audit, and confirmation policy |
| Managed secrets make official writes high-risk | Keep secrets execution-time only, redact all output, and make signer usage explicit in runtime metadata |
| Plugin upgrades overwrite operator secret bindings | Keep activation config in root-owned `chainbot.toml` instead of install-managed plugin package directories |
| Live-only listeners miss events during downtime | Keep V1 scope explicit, document that downtime recovery is out of scope, and verify restart resumes live delivery cleanly |
| Official package installation breaks root validity | Reuse staged install, post-swap validation, and rollback behavior already present in the repository |
| Runtime scope expands into chain business logic | Keep runtime changes limited to generic host guarantees and push provider-specific behavior into official Rust plugins |

## Documentation / Operational Notes

- Document that managed secrets are used for signing only within the official execution path.
- Document that raw passthrough is not an unrestricted RPC proxy.
- Document chain-specific confirmation defaults and configurable overrides.
- Document that endpoints are user-supplied operational inputs rather than an official network whitelist.
- Treat future signed catalog or provenance work as a separate follow-up rather than absorbing it into this feature.

## Sources & References

- Related code: `crates/chainbot/src/plugin/source/*`
- Related code: `crates/chainbot/src/plugin/contract.rs`
- Related code: `crates/chainbot/src/plugin/host.rs`
- Related code: `crates/chainbot/src/infrastructure/config/mod.rs`
- Related code: `crates/chainbot/src/domain/trigger/*`
- Related code: `crates/chainbot/src/app/cli/view/*`
- Related docs: `docs/decisions/CHAINBOT_OFFICIAL_PLUGIN_DESIGN.md`
- Related docs: `docs/decisions/CHAINBOT_PLUGIN_ACTIVATION_CONFIG_DESIGN.md`
- Related docs: `docs/decisions/CHAINBOT_PLUGIN_SOURCE_INSTALL_DESIGN.md`
- Related docs: `docs/engineering/CHAINBOT_INGRESS_TRIGGER_IMPLEMENTATION.md`
- Related docs: `docs/research/CHAINBOT_CLI_CATALOG_DISCOVERY_PROPOSAL.md`
