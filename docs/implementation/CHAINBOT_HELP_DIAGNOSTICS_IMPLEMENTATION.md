# ChainBot Help and Diagnostics Implementation

## Scope

- Expand CLI help into richer skill-style command cards.
- Add canonical root/workflow/trigger/plugin config examples to help and README.
- Improve CLI diagnostics so invalid TOML and argv misuse point to precise locations.

## Files Changed

- `crates/chainbot/src/cli.rs`
- `crates/chainbot/src/config.rs`
- `crates/chainbot/src/errors.rs`
- `crates/chainbot/tests/cli_surface.rs`
- `README.md`
- `docs/design/CHAINBOT_CLI_DESIGN.md`
- `docs/implementation/CHAINBOT_HELP_DIAGNOSTICS_IMPLEMENTATION.md`
- `docs/implementation/AGENTS.md`
- `AGENTS.md`

## Runtime Changes

- `chainbot help <command>` now renders stable sectioned help cards with `Usage`, `Outputs`, `Root resolution`, `Failure navigation`, and command-specific config examples when relevant.
- `validate`, `trigger`, `run`, `serve`, and `init` help now embed canonical config snippets so operators and agents can infer the expected on-disk contract without opening fixture files.
- CLI argv misuse now reports the command path plus argument position.
- TOML decode failures now render the failing file path, line, column, and highlighted source line.
- `status` human output no longer prints the stray `Legacy Layout` heading.

## Validation

- `crates/chainbot/tests/cli_surface.rs` covers embedded config examples, argument-position diagnostics, invalid TOML line context, and status output cleanup.
- LSP diagnostics on touched Rust files remain clean.

## Notes

- The implementation intentionally stays inside the current handwritten CLI instead of introducing `clap` or a diagnostic crate.
- TOML diagnostics reuse the existing `toml` dependency and compute source context from the already-loaded document text.
- Canonical config examples now also live under the top-level `examples/` folder so users can browse complete roots instead of only inline snippets.
