//! [INPUT]
//! Long-running runtime orchestration dependencies from CLI command handlers and runtime/state subsystems.
//!
//! [OUTPUT]
//! App-layer runtime controllers for daemon lifecycle and serve-loop orchestration.
//!
//! [ROLE]
//! Isolates runtime process orchestration from CLI parsing/help and read-model rendering.

pub(crate) mod daemon;
pub(crate) mod execution;
pub(crate) mod external_triggers;
