# ChainBot V2.1.2 CLI Implementation

## Scope

- Align the documented CLI root-resolution contract around `CHAINBOT_CONFIG_DIR` with fallback to `~/.chainbot`.
- Remove `--root` from the stable CLI design surface and examples.
- Bump the crate release version to `2.1.2`.
- Keep the existing `status`, `trigger`, `validate`, `list-runs`, `run`, and `serve` runtime semantics unchanged in this documentation step.

## Files Changed

- `docs/decisions/CHAINBOT_CLI_DESIGN.md`
- `docs/research/CHAINBOT_V212_CLI_PROPOSAL.md`
- `docs/engineering/CHAINBOT_V212_CLI_IMPLEMENTATION.md`
- `AGENTS.md`
- `README.md`
- `crates/chainbot/Cargo.toml`
- `Cargo.lock`

## Documented Release Changes

- Updated the stable CLI design surface so root selection is environment-driven instead of flag-driven.
- Documented `CHAINBOT_CONFIG_DIR` as the explicit root override and `~/.chainbot` as the fallback root.
- Rewrote release-facing references from `v2.1.1` to `v2.1.2` where they describe the current CLI track.
- Bumped the crate manifest and lockfile package entry to `2.1.2`.

## Root Resolution Contract

The documented CLI contract now resolves the effective root with this precedence:

- use `CHAINBOT_CONFIG_DIR` when it is present and non-empty
- otherwise fall back to `~/.chainbot`
- treat the resolved path as the ChainBot root directory, not as a path to `config/root.toml`

This implementation note records both the release-contract update and the runtime alignment that moved public root selection to `CHAINBOT_CONFIG_DIR` with `~/.chainbot` fallback.

## Error Guidance

- Missing root guidance now points users toward `CHAINBOT_CONFIG_DIR` or the default root path.
- Missing root config guidance now points users toward validating the resolved root before running commands.
- Versioned documentation references now resolve to the `v2.1.2` CLI track.

## Validation

- documentation consistency checks via targeted repository search
- crate manifest version alignment in `crates/chainbot/Cargo.toml`
- lockfile package version alignment in `Cargo.lock`

## Follow-Up Notes

- The parser behavior (formerly in `crates/chainbot/src/cli.rs`, now in `app::cli`) is current and aligns with the new design.
- `status` remains the non-executing operational snapshot command in the release narrative.
- `init` remains planned work for the CLI usability track.
