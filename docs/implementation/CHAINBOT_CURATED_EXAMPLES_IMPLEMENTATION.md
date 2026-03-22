# ChainBot Curated Examples Refactor

## Scope

- Replace the old three-root curated example set with a five-root progression from simple to advanced.
- Keep every example aligned with the current canonical ChainBot contract.
- Remove awkward names and over-combined scenarios from the user-facing examples entrypoint.

## Final Curated Roots

- `examples/single-workflow/`
- `examples/builtin-triggers/`
- `examples/workflow-composition/`
- `examples/plugin-integrations/`
- `examples/custom-paths/`

## Replaced Roots

- `examples/canonical-root/` was split into `workflow-composition/` and `plugin-integrations/`.
- `examples/builtin-cron-root/` was absorbed into `single-workflow/` and `builtin-triggers/`.
- `examples/path-overrides/` was renamed and flattened into `custom-paths/`.

## Validation

- `CHAINBOT_CONFIG_DIR=examples/single-workflow target/debug/chainbot validate`
- `CHAINBOT_CONFIG_DIR=examples/builtin-triggers target/debug/chainbot validate`
- `CHAINBOT_CONFIG_DIR=examples/workflow-composition target/debug/chainbot validate`
- `CHAINBOT_CONFIG_DIR=examples/plugin-integrations target/debug/chainbot validate`
- `CHAINBOT_CONFIG_DIR=examples/custom-paths target/debug/chainbot validate`
- `cargo test -p chainbot --test cli_surface`

## Notes

- The curated set is intentionally ordered from smallest standalone root to more specialized examples.
- `custom-paths/` remains a valid contract example, but it is positioned as an advanced override case rather than the default learning path.
