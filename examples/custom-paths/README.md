# custom-paths

This example focuses on non-default root-relative directories.

It demonstrates how workflows, triggers, plugins, secrets, and state can move away from the default names while staying inside one valid ChainBot root.

It demonstrates:

- `chainbot.toml` path overrides through `[paths]`
- a workflow package under `workflow-pkgs/`
- a trigger package under `trigger-pkgs/`
- a plugin package under `plugin-pkgs/`
- relocated secret and state roots under `vault/` and `runtime-state/`

Use this last, after the canonical examples, because it changes directory names without changing the underlying contract.
