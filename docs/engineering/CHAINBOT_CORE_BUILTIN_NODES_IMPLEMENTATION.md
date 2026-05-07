# ChainBot Core Builtin Nodes Implementation

## Scope

- Add the full agreed shortlist of low-dependency builtin workflow nodes focused on authoring efficiency.
- Keep heavy domain integrations on the plugin side of the architecture.
- Preserve the existing builtin-node registry and execution-plane contract without widening the scheduler surface.

## Files Changed

- `crates/chainbot/src/builtins/nodes/handlers/assert.rs`
- `crates/chainbot/src/builtins/nodes/handlers/fail.rs`
- `crates/chainbot/src/builtins/nodes/handlers/data_pick.rs`
- `crates/chainbot/src/builtins/nodes/handlers/data_merge.rs`
- `crates/chainbot/src/builtins/nodes/handlers/data_template.rs`
- `crates/chainbot/src/builtins/nodes/handlers/data_get.rs`
- `crates/chainbot/src/builtins/nodes/handlers/data_coalesce.rs`
- `crates/chainbot/src/builtins/nodes/handlers/data_compare.rs`
- `crates/chainbot/src/builtins/nodes/handlers/data_parse_json.rs`
- `crates/chainbot/src/builtins/nodes/handlers/data_stringify_json.rs`
- `crates/chainbot/src/builtins/nodes/handlers/data_math.rs`
- `crates/chainbot/src/builtins/nodes/handlers/mod.rs`
- `crates/chainbot/src/builtins/nodes/registry.rs`
- `crates/chainbot/tests/execution_scheduler.rs`
- `docs/engineering/CHAINBOT_CORE_BUILTIN_NODES_IMPLEMENTATION.md`
- `docs/engineering/AGENTS.md`
- `AGENTS.md`

## Added Builtin Node Families

- `builtin.flow.assert`
  - Supports `truthy`, `falsy`, `exists`, `equals`, and `not_equals` operations.
  - Fails with a user-facing node error when the assertion does not pass.
- `builtin.flow.fail`
  - Raises an explicit failure from workflow configuration with optional `message` and `code` inputs.
- `builtin.data.pick`
  - Selects a configured field list from an input object and returns the shaped object as `result`.
- `builtin.data.merge`
  - Merges either an `objects` array or the named `left` / `right` / `extra` object inputs into a single `result` object.
- `builtin.data.template`
  - Renders `{{placeholder}}` templates from an input values object with dotted-path lookup.
- `builtin.data.get`
  - Resolves a dotted `path` against the `input` value and returns the selected value as `result`.
- `builtin.data.coalesce`
  - Returns the first non-`null` entry from a `values` array.
- `builtin.data.compare`
  - Produces a boolean `result` for equality and ordered scalar comparisons without failing the workflow on a false result.
- `builtin.data.parse_json`
  - Parses a string `text` input into a JSON value result.
- `builtin.data.stringify_json`
  - Serializes the `value` input into a JSON string result.
- `builtin.data.math`
  - Supports `add`, `subtract`, `multiply`, `divide`, `min`, `max`, and `round` operations over numeric inputs.

## Architecture Notes

- All eleven builtins stay inside the existing trait-backed builtin registry and do not require executor changes beyond normal dispatch.
- The new nodes keep the core binary focused on workflow primitives and data shaping instead of domain SDK integration.
- Each new handler returns stable `outputs` and mirrors them into `run_scoped` so downstream nodes can bind through `run.*` without an extra bridge node.

## Locked Semantics

- `builtin.flow.assert`
  - Default operation is `truthy` when `operation` is empty or `run`.
  - Failed assertions return a node-scoped `CliUsage` error and do not publish outputs.
- `builtin.flow.fail`
  - Always fails the node with a user-facing message and does not publish outputs.
- `builtin.data.pick`
  - Only copies top-level fields from the `input` object.
  - Missing fields are ignored rather than materialized as `null`.
- `builtin.data.merge`
  - Merge is shallow.
  - Later objects win on key collision.
  - When `objects` is absent, merge order is `left -> right -> extra`.
- `builtin.data.template`
  - Supports `{{placeholder}}` with dotted-path lookup into the `values` object.
  - Missing placeholders, empty placeholders, and unterminated placeholders fail the node.
  - Arrays and objects are stringified as JSON when rendered.
- `builtin.data.get`
  - Supports dotted path lookup through objects and numeric array indices.
  - Missing paths fail the node.
  - Empty path returns the full input value.
- `builtin.data.coalesce`
  - Scans `values` in order and returns the first entry that is not `null`.
  - If every entry is `null`, the result is `null`.
- `builtin.data.compare`
  - Default operation is `equals` when `operation` is empty or `run`.
  - Ordered comparison is limited to matching scalar types (`number`, `string`, `bool`).
  - Ordered comparison over objects, arrays, or mismatched scalar types fails the node.
- `builtin.data.parse_json`
  - Only accepts string input via `text`.
  - Invalid JSON text fails the node.
- `builtin.data.stringify_json`
  - Accepts any JSON-compatible value and emits a JSON string result.
- `builtin.data.math`
  - Default operation is `round` when `operation` is empty or `run`.
  - Division by zero fails the node.
  - Non-finite outputs are rejected.
  - `round` uses `value` first and falls back to `left`, with optional numeric `precision`.

## Example Shapes

### `builtin.flow.assert`

```toml
[[nodes]]
manifest_version = "2.0.0"
id = "assert-ready"
kind = "builtin"
plugin = "builtin.flow.assert"
operation = "truthy"
depends_on = []

[[nodes.inputs]]
target = "value"
source = "run.ready"
```

### `builtin.data.template`

```toml
[[nodes]]
manifest_version = "2.0.0"
id = "render-message"
kind = "builtin"
plugin = "builtin.data.template"
operation = "render"
depends_on = []

[[nodes.inputs]]
target = "template"
source = "workflow.message_template"

[[nodes.inputs]]
target = "values"
source = "run.payload"
```

## Validation

- `lsp_diagnostics` on `crates/chainbot/src` returns zero Rust errors after the new handlers are added.
- `lsp_diagnostics` on `crates/chainbot/tests/execution_scheduler.rs` returns zero diagnostics.
- `crates/chainbot/tests/execution_scheduler.rs` adds coverage for the production builtin registry executing `merge -> template -> assert`, `parse_json -> stringify_json -> math`, `get -> coalesce -> compare`, and for explicit `builtin.flow.fail` failures.
- `crates/chainbot/tests/execution_scheduler.rs` also locks down shallow merge precedence, missing template placeholder failures, default math rounding, divide-by-zero rejection, invalid JSON parse failures, generic JSON stringify behavior, dotted-path extraction, null-only coalesce results, and ordered compare type guards.

## Notes

- The current implementation now covers the extended core shortlist: `assert`, `fail`, `pick`, `merge`, `template`, `get`, `coalesce`, `compare`, `parse_json`, `stringify_json`, and `math`.
- The plugin bridge remains the correct expansion path for heavy domain integrations and first-party plugin bundles.
