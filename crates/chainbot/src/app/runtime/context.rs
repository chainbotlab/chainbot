//! [INPUT]
//! Effective root layouts, validated definitions, runtime storage, worker limits, and secret mode environment.
//!
//! [OUTPUT]
//! Loads execution contexts with recovered runtime state and configured history retention, and maps shared runtime errors.
//!
//! [ROLE]
//! Owns application runtime initialization independently of CLI request and output types.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::app::definitions::load_root_definition_bundle;
use crate::builtins::nodes::script_worker::{WorkerHost, WorkerHostLimits};
use crate::builtins::SecretDecryptMode;
use crate::errors::UserFacingError;
use crate::infrastructure::config::{
    load_effective_root_layout, RootDefinitionBundle, RootLayout, RuntimeStorageConfig,
};
use crate::infrastructure::state::{RuntimeStateError, RuntimeStateStore};
use crate::ingress::IngressRuntimeError;

const CHAINBOT_SECRET_DECRYPTOR_ENV: &str = "CHAINBOT_SECRET_DECRYPTOR";
const SECRET_DECRYPTOR_PLAINTEXT: &str = "plaintext";

#[derive(Debug)]
pub(super) struct RuntimeContext {
    pub(super) root_layout: RootLayout,
    pub(super) definitions: RootDefinitionBundle,
    pub(super) storage_config: RuntimeStorageConfig,
    pub(super) state_store: RuntimeStateStore,
    pub(super) secret_mode: SecretDecryptMode,
    pub(super) worker_host: WorkerHost,
}

impl RuntimeContext {
    pub(super) fn load(root_layout: RootLayout) -> Result<RuntimeContext, UserFacingError> {
        let definitions =
            load_root_definition_bundle(&root_layout).map_err(UserFacingError::from_contract)?;
        let storage_config = definitions
            .root_config
            .resolve_runtime_storage(&root_layout.root)
            .map_err(UserFacingError::from_contract)?;
        let recovered_at_ms = current_time_ms()?;
        let mut state_store = RuntimeStateStore::open(&storage_config, recovered_at_ms)
            .map_err(|error| map_runtime_state_error("open runtime state store", error))?;
        let _ = state_store
            .recover_runtime_state(recovered_at_ms)
            .map_err(|error| map_runtime_state_error("recover runtime state", error))?;
        if let Some(policy) = storage_config.history_retention.as_ref() {
            let _ = state_store
                .apply_history_retention(policy, recovered_at_ms)
                .map_err(|error| {
                    map_runtime_state_error("apply runtime history retention", error)
                })?;
        }

        Ok(RuntimeContext {
            root_layout,
            definitions,
            storage_config,
            state_store,
            secret_mode: secret_decrypt_mode_from_env(),
            worker_host: WorkerHost::new(WorkerHostLimits::default()),
        })
    }
}

pub(crate) fn resolve_existing_root(base_layout: &RootLayout) -> Result<RootLayout, UserFacingError> {
    let layout = load_effective_root_layout(base_layout).map_err(UserFacingError::from_contract)?;
    layout.validate_paths_exist().map_err(UserFacingError::from_contract)?;
    Ok(layout)
}

pub(crate) fn current_time_ms() -> Result<i64, UserFacingError> {
    let duration = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|_| {
        UserFacingError::state("System clock is before UNIX_EPOCH; cannot continue.")
    })?;
    Ok(duration.as_millis().min(i64::MAX as u128) as i64)
}

pub(crate) fn map_runtime_state_error(action: &str, error: RuntimeStateError) -> UserFacingError {
    UserFacingError::state(format!("Failed to {action}: {error}"))
}

pub(super) fn map_ingress_error(error: IngressRuntimeError) -> UserFacingError {
    match error {
        IngressRuntimeError::RuntimeState(source) => {
            map_runtime_state_error("operate ingress runtime state", source)
        }
        other => UserFacingError::state_with_code("ingress_runtime_error", other.to_string()),
    }
}

fn secret_decrypt_mode_from_env() -> SecretDecryptMode {
    match std::env::var(CHAINBOT_SECRET_DECRYPTOR_ENV)
        .ok()
        .map(|value| value.to_lowercase())
        .as_deref()
    {
        Some(SECRET_DECRYPTOR_PLAINTEXT) => SecretDecryptMode::Plaintext,
        _ => SecretDecryptMode::Gpg,
    }
}

