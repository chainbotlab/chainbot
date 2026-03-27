//! [INPUT]
//! Backend-agnostic runtime-state record contracts and deterministic pure update helpers.
//!
//! [OUTPUT]
//! Provides stable run/trigger/inbox/lease model types plus pure token and snapshot mutation helpers.
//!
//! [ROLE]
//! Owns domain-level runtime-state contracts independent from filesystem and database backends.
//!
//! ```text
//! +-- domain::state (this module) --+
//! |                                  |
//! |  model.rs                       |  schema/version/path/token helpers + statuses
//! |  lease.rs                       |  lease contracts (ServeLeaseState, etc.)
//! |  records.rs                     |  run/trigger/inbox/staged/checkpoint/snapshot records
//! |                                  |
//! |  NO backend/file/db ownership   |
//! +----------------------------------+
//! ```

pub mod lease;
pub mod model;
pub mod records;

pub use lease::{LeaseAcquireResult, ServeLeaseSnapshot, ServeLeaseState, SERVE_OWNER_ID_PREFIX};
pub use model::{
    accepted_trigger_key, default_schema_version, retain_unexpired_tokens, sanitize_path_component,
    upsert_token_snapshot, RunStatus, CURRENT_SCHEMA_MAJOR,
};
pub use records::{
    IngressInboxRecord, RunRecordSummary, StagedTriggerEventRecord, TriggerCheckpointRecord,
    TriggerEventRecord, TriggerSnapshotRecord, TriggerTokenSnapshot, WorkflowRuntimeLogEntry,
};
