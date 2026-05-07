# ChainBot V2 MVP Implementation

## Scope

- Rename `.sisyphus/plans/chainbot-v2.md` to `.sisyphus/plans/chainbot-v2-mvp.md`.
- Rename `.sisyphus/notepads/chainbot-v2/` to `.sisyphus/notepads/chainbot-v2-mvp/`.
- Remove temporary `.sisyphus/evidence/` artifacts generated during task execution and review waves.
- Bump `crates/chainbot/Cargo.toml` package version to `2.0.0` for Cargo-compatible semver.

## Artifact Map

- Plan of record: `.sisyphus/plans/chainbot-v2-mvp.md`
- Implementation notes: `.sisyphus/notepads/chainbot-v2-mvp/`
- Durable bootstrap record: `docs/engineering/WORKSPACE_BOOTSTRAP_IMPLEMENTATION.md`
- Cargo package boundary: `crates/chainbot/Cargo.toml`

## Documentation Alignment

- Keep the bootstrap document focused on repository/workspace initialization.
- Record MVP-specific rename and release alignment in this document to avoid overloading the bootstrap checklist.
- Keep repository indexes aligned with the new MVP implementation record and plan path.

## Validation Matrix

- Confirm `.sisyphus/plans/chainbot-v2-mvp.md` exists.
- Confirm `.sisyphus/notepads/chainbot-v2-mvp/` exists with the existing decision, issue, learning, and problem notes.
- Confirm `.sisyphus/evidence/` has been removed.
- Confirm `crates/chainbot/Cargo.toml` declares version `2.0.0`.

## Execution Notes

- Treat `.sisyphus/evidence/` as disposable task output rather than durable repository knowledge.
- Keep durable implementation history in `docs/engineering/` and `.sisyphus/notepads/` instead of evidence logs.
- Preserve the existing workspace bootstrap record as a separate implementation concern from the MVP rename/release pass.
