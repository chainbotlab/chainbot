use std::collections::BTreeMap;
use std::path::Path;

use crate::builtins::nodes::context::SecretDecryptMode;
use crate::errors::ContractError;
use crate::secrets::{
    GpgSecretDecryptor, PlaintextSecretDecryptor, SecretProvider, SecretReference, SecretValue,
};

#[derive(Debug, Clone)]
pub(crate) struct ResolvedNodeInputs {
    pub(crate) values: BTreeMap<String, serde_json::Value>,
    pub(crate) resolved_secrets: Vec<SecretValue>,
}

pub(crate) fn resolve_node_inputs(
    secrets_root: &Path,
    mode: SecretDecryptMode,
    values: &BTreeMap<String, serde_json::Value>,
) -> Result<ResolvedNodeInputs, ContractError> {
    let mut resolved_values = BTreeMap::new();
    let mut resolved_secrets = Vec::new();
    for (key, value) in values {
        let resolved_value =
            resolve_value_with_secrets(secrets_root, mode, value, &mut resolved_secrets)?;
        resolved_values.insert(key.clone(), resolved_value);
    }
    Ok(ResolvedNodeInputs {
        values: resolved_values,
        resolved_secrets,
    })
}

fn resolve_value_with_secrets(
    secrets_root: &Path,
    mode: SecretDecryptMode,
    value: &serde_json::Value,
    resolved_secrets: &mut Vec<SecretValue>,
) -> Result<serde_json::Value, ContractError> {
    match value {
        serde_json::Value::String(raw) if raw.starts_with("secret://") => {
            let reference = SecretReference::parse(raw)?;
            let secret_value = match mode {
                SecretDecryptMode::Gpg => {
                    SecretProvider::new(secrets_root.to_path_buf(), GpgSecretDecryptor::new())
                        .resolve_reference(&reference)?
                }
                SecretDecryptMode::Plaintext => {
                    SecretProvider::new(secrets_root.to_path_buf(), PlaintextSecretDecryptor)
                        .resolve_reference(&reference)?
                }
            };
            resolved_secrets.push(secret_value.clone());
            Ok(serde_json::Value::String(secret_value.expose().to_owned()))
        }
        serde_json::Value::Array(items) => {
            let mut resolved = Vec::with_capacity(items.len());
            for item in items {
                resolved.push(resolve_value_with_secrets(
                    secrets_root,
                    mode,
                    item,
                    resolved_secrets,
                )?);
            }
            Ok(serde_json::Value::Array(resolved))
        }
        serde_json::Value::Object(map) => {
            let mut resolved = serde_json::Map::with_capacity(map.len());
            for (key, item) in map {
                resolved.insert(
                    key.clone(),
                    resolve_value_with_secrets(secrets_root, mode, item, resolved_secrets)?,
                );
            }
            Ok(serde_json::Value::Object(resolved))
        }
        _ => Ok(value.clone()),
    }
}
