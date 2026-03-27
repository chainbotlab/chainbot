//! [INPUT]
//! Lease state semantics for the `serve` daemon and serde support for persisted lease snapshots.
//!
//! [OUTPUT]
//! Defines daemon lease result and snapshot types used by runtime-state coordination adapters.
//!
//! [ROLE]
//! Encodes the backend-agnostic domain contract for daemon ownership and lease visibility.

use serde::{Deserialize, Serialize};

pub const SERVE_OWNER_ID_PREFIX: &str = "chainbot-serve-pid-";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeaseAcquireResult {
    Acquired,
    Renewed,
    Rejected {
        current_owner: String,
        expires_at_ms: i64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServeLeaseState {
    Idle,
    Active,
    Stale,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServeLeaseSnapshot {
    pub state: ServeLeaseState,
    pub owner_id: Option<String>,
    pub expires_at_ms: Option<i64>,
}
