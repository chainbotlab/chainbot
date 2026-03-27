# Local Rules

## Scope
- Position: CLI command surface for the application layer.
- Logic: Owns argument intent mapping, command-level parsing, help rendering, and read-model wiring for CLI output modes.
- Constraints: Keep transport-independent workflow logic in `../runtime/` and domain contracts in `../../domain/`.

## Members
- `mod.rs`: CLI boundary exports and shared CLI request types.
- `commands.rs`: Declarative command and help-topic definitions.
- `parse.rs`: Argument parsing into validated CLI requests.
- `help.rs`: Stable help text and topic rendering content.
- `view/`: User-facing status, observe, and catalog read-model rendering.
