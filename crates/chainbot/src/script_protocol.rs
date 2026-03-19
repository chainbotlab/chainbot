//! [INPUT]
//! Serialized worker payloads, protocol-version validation, and contract error helpers.
//!
//! [OUTPUT]
//! Defines the versioned request and response envelopes for ChainBot script workers.
//!
//! [ROLE]
//! Owns the script-worker protocol contract shared by config loading, builtin script execution, and protocol tests.

use serde::{Deserialize, Serialize};

use crate::errors::{assert_supported_major, ContractError};

pub const CURRENT_SCRIPT_PROTOCOL_MAJOR: u64 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkerRequestEnvelope {
    pub protocol_version: String,
    pub request_id: String,
    pub worker_id: String,
    pub workflow_id: String,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkerResponseEnvelope {
    pub protocol_version: String,
    pub request_id: String,
    pub success: bool,
    pub output: serde_json::Value,
}

impl WorkerRequestEnvelope {
    pub fn validate(&self) -> Result<(), ContractError> {
        assert_supported_major(
            "worker_request.protocol_version",
            &self.protocol_version,
            CURRENT_SCRIPT_PROTOCOL_MAJOR,
        )
    }

    pub fn from_json_str(input: &str) -> Result<Self, ContractError> {
        let envelope: Self = serde_json::from_str(input)?;
        envelope.validate()?;
        Ok(envelope)
    }
}

impl WorkerResponseEnvelope {
    pub fn validate(&self) -> Result<(), ContractError> {
        assert_supported_major(
            "worker_response.protocol_version",
            &self.protocol_version,
            CURRENT_SCRIPT_PROTOCOL_MAJOR,
        )
    }

    pub fn from_json_str(input: &str) -> Result<Self, ContractError> {
        let envelope: Self = serde_json::from_str(input)?;
        envelope.validate()?;
        Ok(envelope)
    }
}
