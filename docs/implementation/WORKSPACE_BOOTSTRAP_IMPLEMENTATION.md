# Workspace Bootstrap Implementation

## Scope

- Install project-local rust-skills instructions.
- Convert the root crate into a workspace with `crates/chainbot`.
- Add root and local fractal manifests.
- Initialize the fractal-repo documentation buckets.
- Update Rust `.gitignore` for workspace use.

## Validation Matrix

- `cargo metadata --no-deps`
- `cargo check --workspace`
- `cargo test --workspace`

## Execution Notes

- Keep `Cargo.lock` at the repository root.
- Do not run formatting commands.
- Prefer minimal structural changes over speculative scaffolding.
