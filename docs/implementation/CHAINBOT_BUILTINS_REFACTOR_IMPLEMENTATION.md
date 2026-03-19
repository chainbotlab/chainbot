# ChainBot Builtins Refactor Implementation

## Scope

- Consolidate workflow builtins and trigger builtins under a unified `builtins` namespace.
- Replace ad-hoc builtin registration with trait-backed handler and emitter registries.
- Separate builtin runtime context, dispatch helpers, registry assembly, and per-kind implementations into explicit submodules.
- Clarify the final canonical public API around `builtins::{nodes,triggers}` and test-seeded registry helpers.

## Files Changed

- `crates/chainbot/src/lib.rs`
- `crates/chainbot/src/cli.rs`
- `crates/chainbot/src/executor.rs`
- `crates/chainbot/src/builtins/mod.rs`
- `crates/chainbot/src/builtins/AGENTS.md`
- `crates/chainbot/src/builtins/nodes/`
- `crates/chainbot/src/builtins/triggers/`
- `crates/chainbot/src/AGENTS.md`
- `crates/chainbot/tests/execution_scheduler.rs`
- `crates/chainbot/tests/trigger_plane.rs`
- `docs/implementation/CHAINBOT_BUILTINS_REFACTOR_IMPLEMENTATION.md`
- `docs/implementation/AGENTS.md`
- `AGENTS.md`

## Architecture Changes

- `builtins/` is now the canonical namespace for both workflow builtin nodes and builtin triggers.
- Workflow builtin nodes moved into `builtins/nodes/` with explicit modules for `context`, `contract`, `dispatch`, `input_resolver`, `registry`, and `handlers/*`.
- Builtin triggers moved into `builtins/triggers/` with explicit modules for `context`, `contract`, `dispatch`, `registry`, and `emitters/*`.
- The removed legacy trigger builtin module path is no longer part of the public API; `builtins::triggers` is the supported trigger builtin entry surface.

## Trait Standardization

- Node builtins now implement `BuiltinNodeHandler` and are stored behind `Arc<dyn BuiltinNodeHandler>` in `BuiltinNodeRegistry`.
- Trigger builtins now implement `BuiltinTriggerHandler` and are stored behind `Arc<dyn BuiltinTriggerHandler>` in `BuiltinTriggerRegistry`.
- The node registry exposes `with_test_handlers()` for scheduler tests and keeps `register(kind, closure)` as an extension adapter for test or custom wiring.
- The trigger registry keeps `register(kind, fn)` as a compatibility adapter for test fanout coverage.

## Public API

- Canonical imports now use `chainbot::builtins::triggers::*`.
- Trigger builtin callers should use `chainbot::builtins::triggers::build_builtin_trigger_emissions`.
- Workflow builtin assembly still uses `chainbot::builtins::{build_builtin_registry, BuiltinRuntimeContext, SecretDecryptMode}`.
- `executor.rs` now depends on `crate::builtins::nodes::{contract,dispatch}` instead of the old flat builtin modules.
- `cli.rs` now assembles both builtin registries from the unified `crate::builtins` namespace.

## Validation

- Integration coverage in `crates/chainbot/tests/execution_scheduler.rs` now locks down the test-seeded node registry surface plus closure-based registry extension behavior.
- Integration coverage in `crates/chainbot/tests/trigger_plane.rs` now locks down canonical builtin-trigger emissions only.
- `lsp_diagnostics` on `crates/chainbot/src` returns zero Rust errors after the namespace move.
- Targeted Cargo test and check commands should verify the refactor without running `cargo fmt`.

## Notes

- The new layout isolates per-kind implementations so future builtin expansion should only require adding a handler or emitter file plus one registry registration point.
