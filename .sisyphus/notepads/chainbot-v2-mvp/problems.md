
- 2026-03-16: Blocking review defect — `crates/chainbot/src/cli.rs:515` only runs the first accepted trigger request from a snapshot even though `crates/chainbot/tests/trigger_plane.rs:353` proves multiple accepted requests are possible.
- 2026-03-16: Blocking review defect — `crates/chainbot/src/trigger.rs:124` defaults unknown trigger kinds to builtin behavior, leaving `UnknownTriggerKind` unused and weakening validation guarantees.
- 2026-03-16: Blocking review defect — `crates/chainbot/src/trigger.rs:262` commits dedup/cooldown tokens before `crates/chainbot/src/trigger.rs:323` persists the trigger record, so file-backed and SQLite state can diverge after crash/write failure.
