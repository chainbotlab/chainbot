# plugin-integrations

This example focuses on root-level external plugin registration.

It demonstrates:

- an external trigger plugin package (`market-trigger-plugin`)
- an external node plugin package (`quote-node-plugin`)
- plugin-local `bin/` executables
- a workflow and trigger package wired through those plugins

The example keeps workflow, trigger, and plugin packages separate so you can see exactly where root-level plugin registration begins.
