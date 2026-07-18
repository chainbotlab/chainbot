//! [INPUT]
//! Environment-derived root paths, on-disk package manifests, plugin manifests, and serialized contract payloads.
//!
//! [OUTPUT]
//! Provides infrastructure-owned root/storage contracts plus loader primitives for canonical root layout and package manifest decoding.
//!
//! [ROLE]
//! Defines the infrastructure configuration boundary consumed by app composition and CLI/runtime callers.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::domain::state::RunRecordSummary;
use crate::domain::trigger::TriggerDefinition;
use crate::domain::workflow::WorkflowDefinition;
use crate::errors::{assert_required_major, assert_supported_major, ContractError};
use crate::plugin::PluginManifest;
use crate::script_protocol::{WorkerRequestEnvelope, WorkerResponseEnvelope};
use crate::secrets::SecretReference;

pub mod loader;
pub mod package_loader;
pub mod root_layout;

pub use loader::{
    load_effective_root_layout, load_trigger_definitions, resolve_root_layout, set_trigger_enabled,
    TriggerToggleResult,
};
pub use package_loader::PACKAGE_CONFIG_FILE_NAME;
pub use root_layout::{
    RootLayout, CHAINBOT_CONFIG_DIR_ENV, DEFAULT_ROOT_DIR_NAME, ROOT_CONFIG_FILE_NAME,
};

pub const CURRENT_SCHEMA_MAJOR: u64 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageMode {
    Local,
    Postgres,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageDefinition {
    pub mode: StorageMode,
    #[serde(default)]
    pub local: Option<LocalStorageDefinition>,
    #[serde(default)]
    pub postgres: Option<PostgresStorageDefinition>,
    #[serde(default)]
    pub retention: RuntimeHistoryRetentionDefinition,
    #[serde(default)]
    pub raw_debug: RawDebugArtifactsDefinition,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalStorageDefinition {
    #[serde(default)]
    pub database_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PostgresStorageDefinition {
    #[serde(default)]
    pub database_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RawDebugArtifactsDefinition {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub artifacts_dir: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RuntimeHistoryRetentionDefinition {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub run_retention_days: Option<u64>,
    #[serde(default)]
    pub workflow_log_retention_days: Option<u64>,
    #[serde(default)]
    pub trigger_event_retention_days: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeStorageBackend {
    Local { database_path: PathBuf },
    Postgres { database_url: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeHistoryRetentionPolicy {
    pub run_retention_ms: Option<i64>,
    pub workflow_log_retention_ms: Option<i64>,
    pub trigger_event_retention_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeStorageConfig {
    pub backend: RuntimeStorageBackend,
    pub history_retention: Option<RuntimeHistoryRetentionPolicy>,
    pub raw_debug_enabled: bool,
    pub raw_debug_artifacts_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfigRoot {
    pub schema_version: String,
    pub workflows: Vec<WorkflowDefinition>,
    pub plugins: Vec<PluginManifest>,
    pub worker_templates: Vec<WorkerTemplate>,
    pub run_defaults: Option<RunRecordSummary>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkerTemplate {
    pub name: String,
    pub request: WorkerRequestEnvelope,
    pub response: WorkerResponseEnvelope,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RootConfigDefinition {
    #[serde(rename = "manifest_version")]
    pub schema_version: String,
    #[serde(default)]
    pub chainbot_version: Option<String>,
    #[serde(default)]
    pub profile: Option<String>,
    #[serde(default)]
    pub secret_refs: Vec<String>,
    #[serde(default)]
    pub runtime_defaults: BTreeMap<String, serde_json::Value>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub plugin_activation: BTreeMap<String, PluginActivationDefinition>,
    #[serde(default)]
    pub paths: RootPathOverrides,
    pub storage: StorageDefinition,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PluginActivationDefinition {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub secret_bindings: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_origins: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RootPathOverrides {
    #[serde(default)]
    pub workflows_dir: Option<String>,
    #[serde(default)]
    pub triggers_dir: Option<String>,
    #[serde(default)]
    pub plugins_dir: Option<String>,
    #[serde(default)]
    pub secrets_dir: Option<String>,
    #[serde(default)]
    pub state_dir: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RootDefinitionBundle {
    pub root_config: RootConfigDefinition,
    pub workflows: Vec<WorkflowDefinition>,
    pub triggers: Vec<TriggerDefinition>,
    pub plugins: Vec<PluginManifest>,
}

impl ConfigRoot {
    pub fn validate(&self) -> Result<(), ContractError> {
        assert_supported_major(
            "config.schema_version",
            &self.schema_version,
            CURRENT_SCHEMA_MAJOR,
        )?;

        for workflow in &self.workflows {
            workflow.validate()?;
        }

        for plugin in &self.plugins {
            plugin.validate()?;
        }

        for template in &self.worker_templates {
            template.request.validate()?;
            template.response.validate()?;
        }

        if let Some(summary) = &self.run_defaults {
            summary.validate()?;
        }

        Ok(())
    }

    pub fn from_json_str(input: &str) -> Result<Self, ContractError> {
        let config: Self = serde_json::from_str(input)?;
        config.validate()?;
        Ok(config)
    }
}

impl RootConfigDefinition {
    pub fn validate(&self) -> Result<(), ContractError> {
        assert_required_major(
            "root_config.manifest_version",
            &self.schema_version,
            CURRENT_SCHEMA_MAJOR,
        )?;

        for secret_ref in &self.secret_refs {
            let _ = SecretReference::parse(secret_ref)?;
        }

        for (plugin_id, activation) in &self.plugin_activation {
            if plugin_id.trim().is_empty() {
                return Err(ContractError::InvalidRootConfigField {
                    field: "root_config.plugin_activation",
                    detail: "plugin_activation keys must not be empty".to_owned(),
                });
            }
            for (slot, secret_ref) in &activation.secret_bindings {
                if slot.trim().is_empty() {
                    return Err(ContractError::InvalidRootConfigField {
                        field: "root_config.plugin_activation.secret_bindings",
                        detail: format!(
                            "plugin_activation.{}.secret_bindings keys must not be empty",
                            plugin_id
                        ),
                    });
                }
                let _ = SecretReference::parse(secret_ref)?;
            }
            for origin in &activation.allowed_origins {
                if origin.trim().is_empty() {
                    return Err(ContractError::InvalidRootConfigField {
                        field: "root_config.plugin_activation.allowed_origins",
                        detail: format!(
                            "plugin_activation.{}.allowed_origins entries must not be empty",
                            plugin_id
                        ),
                    });
                }
                validate_allowed_origin(
                    plugin_id,
                    origin,
                    "root_config.plugin_activation.allowed_origins",
                )?;
            }
        }

        self.storage.validate()?;

        Ok(())
    }

    pub fn resolve_runtime_storage(
        &self,
        root: &Path,
    ) -> Result<RuntimeStorageConfig, ContractError> {
        let backend = match self.storage.mode {
            StorageMode::Local => {
                let local = self.storage.local.as_ref().ok_or_else(|| {
                    ContractError::InvalidRootConfigField {
                        field: "root_config.storage.local.database_path",
                        detail:
                            "storage.local.database_path is required when storage.mode is local"
                                .to_owned(),
                    }
                })?;
                let database_path = local.database_path.as_deref().ok_or_else(|| {
                    ContractError::InvalidRootConfigField {
                        field: "root_config.storage.local.database_path",
                        detail:
                            "storage.local.database_path is required when storage.mode is local"
                                .to_owned(),
                    }
                })?;
                RuntimeStorageBackend::Local {
                    database_path: package_loader::resolve_root_relative_dir(
                        root,
                        "root_config.storage.local.database_path",
                        database_path,
                    )?,
                }
            }
            StorageMode::Postgres => {
                let postgres = self.storage.postgres.as_ref().ok_or_else(|| {
                    ContractError::InvalidRootConfigField {
                        field: "root_config.storage.postgres.database_url",
                        detail: "storage.postgres.database_url is required when storage.mode is postgres"
                            .to_owned(),
                    }
                })?;
                let database_url = postgres.database_url.as_ref().ok_or_else(|| {
                    ContractError::InvalidRootConfigField {
                        field: "root_config.storage.postgres.database_url",
                        detail: "storage.postgres.database_url is required when storage.mode is postgres"
                            .to_owned(),
                    }
                })?;
                RuntimeStorageBackend::Postgres {
                    database_url: database_url.trim().to_owned(),
                }
            }
        };

        let raw_debug_artifacts_dir = match self.storage.raw_debug.artifacts_dir.as_deref() {
            Some(value) => Some(package_loader::resolve_root_relative_dir(
                root,
                "root_config.storage.raw_debug.artifacts_dir",
                value,
            )?),
            None => None,
        };

        Ok(RuntimeStorageConfig {
            backend,
            history_retention: self.storage.retention.resolve_policy()?,
            raw_debug_enabled: self.storage.raw_debug.enabled,
            raw_debug_artifacts_dir,
        })
    }
}

fn validate_allowed_origin(
    plugin_id: &str,
    origin: &str,
    field: &'static str,
) -> Result<(), ContractError> {
    let parsed =
        reqwest::Url::parse(origin).map_err(|source| ContractError::InvalidRootConfigField {
            field,
            detail: format!(
                "plugin_activation.{}.allowed_origins contains invalid origin {}: {}",
                plugin_id, origin, source
            ),
        })?;
    if !matches!(parsed.scheme(), "http" | "https" | "ws" | "wss") {
        return Err(ContractError::InvalidRootConfigField {
            field,
            detail: format!(
                "plugin_activation.{}.allowed_origins origin {} must use http, https, ws, or wss",
                plugin_id, origin
            ),
        });
    }
    if parsed.host_str().is_none() {
        return Err(ContractError::InvalidRootConfigField {
            field,
            detail: format!(
                "plugin_activation.{}.allowed_origins origin {} must include a host",
                plugin_id, origin
            ),
        });
    }
    if parsed.query().is_some() || parsed.fragment().is_some() || parsed.path() != "/" {
        return Err(ContractError::InvalidRootConfigField {
            field,
            detail: format!(
                "plugin_activation.{}.allowed_origins origin {} must be an origin without path, query, or fragment",
                plugin_id, origin
            ),
        });
    }
    Ok(())
}

impl StorageDefinition {
    fn validate(&self) -> Result<(), ContractError> {
        match self.mode {
            StorageMode::Local => {
                let database_path = self
                    .local
                    .as_ref()
                    .and_then(|local| local.database_path.as_ref())
                    .map(String::as_str)
                    .ok_or_else(|| ContractError::InvalidRootConfigField {
                        field: "root_config.storage.local.database_path",
                        detail:
                            "storage.local.database_path is required when storage.mode is local"
                                .to_owned(),
                    })?;
                if database_path.trim().is_empty() {
                    return Err(ContractError::InvalidRootConfigField {
                        field: "root_config.storage.local.database_path",
                        detail: "value cannot be empty".to_owned(),
                    });
                }
            }
            StorageMode::Postgres => {
                let database_url = self
                    .postgres
                    .as_ref()
                    .and_then(|postgres| postgres.database_url.as_ref())
                    .map(String::as_str)
                    .ok_or_else(|| ContractError::InvalidRootConfigField {
                        field: "root_config.storage.postgres.database_url",
                        detail:
                            "storage.postgres.database_url is required when storage.mode is postgres"
                                .to_owned(),
                    })?;
                if database_url.trim().is_empty() {
                    return Err(ContractError::InvalidRootConfigField {
                        field: "root_config.storage.postgres.database_url",
                        detail: "value cannot be empty".to_owned(),
                    });
                }
            }
        }

        self.retention.validate()?;

        Ok(())
    }
}

impl RuntimeHistoryRetentionDefinition {
    fn validate(&self) -> Result<(), ContractError> {
        if !self.enabled {
            return Ok(());
        }

        let has_any_window = self.run_retention_days.is_some()
            || self.workflow_log_retention_days.is_some()
            || self.trigger_event_retention_days.is_some();
        if !has_any_window {
            return Err(ContractError::InvalidRootConfigField {
                field: "root_config.storage.retention",
                detail: "at least one retention window is required when storage.retention.enabled is true"
                    .to_owned(),
            });
        }

        for (field, value) in [
            (
                "root_config.storage.retention.run_retention_days",
                self.run_retention_days,
            ),
            (
                "root_config.storage.retention.workflow_log_retention_days",
                self.workflow_log_retention_days,
            ),
            (
                "root_config.storage.retention.trigger_event_retention_days",
                self.trigger_event_retention_days,
            ),
        ] {
            if matches!(value, Some(0)) {
                return Err(ContractError::InvalidRootConfigField {
                    field,
                    detail: "retention days must be greater than zero".to_owned(),
                });
            }
        }

        Ok(())
    }

    fn resolve_policy(&self) -> Result<Option<RuntimeHistoryRetentionPolicy>, ContractError> {
        if !self.enabled {
            return Ok(None);
        }

        self.validate()?;
        Ok(Some(RuntimeHistoryRetentionPolicy {
            run_retention_ms: retention_days_to_ms(self.run_retention_days)?,
            workflow_log_retention_ms: retention_days_to_ms(self.workflow_log_retention_days)?,
            trigger_event_retention_ms: retention_days_to_ms(self.trigger_event_retention_days)?,
        }))
    }
}

fn retention_days_to_ms(days: Option<u64>) -> Result<Option<i64>, ContractError> {
    let Some(days) = days else {
        return Ok(None);
    };
    let ms = days
        .checked_mul(24)
        .and_then(|value| value.checked_mul(60))
        .and_then(|value| value.checked_mul(60))
        .and_then(|value| value.checked_mul(1_000))
        .ok_or_else(|| ContractError::InvalidRootConfigField {
            field: "root_config.storage.retention",
            detail: "retention window is too large to fit into runtime millisecond bounds"
                .to_owned(),
        })?;
    Ok(Some(i64::try_from(ms).unwrap_or(i64::MAX)))
}
