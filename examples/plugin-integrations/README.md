# plugin-integrations

This example focuses on root-level external plugin registration.

It demonstrates:

- external trigger plugin packages for both lifecycle models:
  - `market-trigger-plugin` (`process_short_lived`)
  - `market-trigger-wasm-plugin` (`wasm_daemon_persistent_session`)
- an external node plugin package (`quote-node-plugin`)
- plugin-local `bin/` runtime artifacts:
  - executable shell adapters for process trigger and node plugins
  - a committed wasm module artifact for `market-trigger-wasm-plugin` (`bin/external_trigger.wasm`)
- a workflow and trigger package wired through those plugins

The example keeps workflow, trigger, and plugin packages separate so you can see exactly where root-level plugin registration begins.
