# core-builtins

This example demonstrates the builtin flow and data nodes that ship with the lean ChainBot core.

It demonstrates:

- JSON parsing and stringify nodes
- nested path extraction with `builtin.data.get`
- fallback selection with `builtin.data.coalesce`
- boolean comparison with `builtin.data.compare`
- object shaping with `builtin.data.pick` and `builtin.data.merge`
- lightweight string rendering with `builtin.data.template`
- numeric rounding with `builtin.data.math`
- non-plugin validation with `builtin.flow.assert`
