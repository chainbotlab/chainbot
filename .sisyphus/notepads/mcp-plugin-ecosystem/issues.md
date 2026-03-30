## 2026-03-29 Task 1 scope correction

- Verification flagged scope creep in `crates/chainbot/src/plugin/source/*` while implementing Task 1.
- Corrective action: reverted all `plugin/source/*` edits and retained only Task 1 contract-layer changes (`plugin/contract.rs`, `plugin/mod.rs`) plus compile-required test struct-field adaptations.
