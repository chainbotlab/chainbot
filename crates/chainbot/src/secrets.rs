/*
[INPUT]:  Secret references, pass-style encrypted files, and runtime redaction targets.
[OUTPUT]: Parsed secret references plus late runtime secret resolution and redaction helpers.
[POS]:    Secrets boundary for pass-style runtime decryption without durable plaintext persistence.
[UPDATE]: 2026-03-16 - Add secret reference syntax parser and serializer.
[UPDATE]: 2026-03-16 - Add pass-style secret provider, decryptor seam, and redaction helpers.
*/

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{Debug, Display, Formatter};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::errors::ContractError;

pub const SECRET_REDACTION_TOKEN: &str = "[REDACTED_SECRET]";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretReference {
    pub namespace: String,
    pub name: String,
    pub key: Option<String>,
}

impl SecretReference {
    pub fn parse(value: &str) -> Result<Self, ContractError> {
        value.parse()
    }

    pub fn to_uri(&self) -> String {
        match &self.key {
            Some(key) => format!("secret://{}/{}#{key}", self.namespace, self.name),
            None => format!("secret://{}/{}", self.namespace, self.name),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct SecretValue {
    value: String,
}

impl SecretValue {
    pub fn new(value: String) -> Self {
        Self { value }
    }

    pub fn expose(&self) -> &str {
        &self.value
    }
}

impl Debug for SecretValue {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretValue([REDACTED_SECRET])")
    }
}

impl Display for SecretValue {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(SECRET_REDACTION_TOKEN)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretDecryptError {
    Io,
    ProcessFailed { exit_code: Option<i32> },
    InvalidUtf8,
}

pub trait SecretDecryptor {
    fn decrypt(&self, encrypted_file: &Path) -> Result<String, SecretDecryptError>;
}

#[derive(Debug, Clone, Default)]
pub struct GpgSecretDecryptor {
    executable: PathBuf,
}

impl GpgSecretDecryptor {
    pub fn new() -> Self {
        Self {
            executable: PathBuf::from("gpg"),
        }
    }

    pub fn with_executable(executable: PathBuf) -> Self {
        Self { executable }
    }
}

impl SecretDecryptor for GpgSecretDecryptor {
    fn decrypt(&self, encrypted_file: &Path) -> Result<String, SecretDecryptError> {
        let output = Command::new(&self.executable)
            .arg("--quiet")
            .arg("--batch")
            .arg("--decrypt")
            .arg(encrypted_file)
            .output()
            .map_err(|_| SecretDecryptError::Io)?;

        if !output.status.success() {
            return Err(SecretDecryptError::ProcessFailed {
                exit_code: output.status.code(),
            });
        }

        String::from_utf8(output.stdout).map_err(|_| SecretDecryptError::InvalidUtf8)
    }
}

#[derive(Debug, Clone, Default)]
pub struct PlaintextSecretDecryptor;

impl SecretDecryptor for PlaintextSecretDecryptor {
    fn decrypt(&self, encrypted_file: &Path) -> Result<String, SecretDecryptError> {
        std::fs::read_to_string(encrypted_file).map_err(|_| SecretDecryptError::Io)
    }
}

#[derive(Debug, Clone)]
pub struct SecretProvider<D>
where
    D: SecretDecryptor,
{
    secrets_root: PathBuf,
    decryptor: D,
}

impl<D> SecretProvider<D>
where
    D: SecretDecryptor,
{
    pub fn new(secrets_root: PathBuf, decryptor: D) -> Self {
        Self {
            secrets_root,
            decryptor,
        }
    }

    pub fn secrets_root(&self) -> &Path {
        &self.secrets_root
    }

    pub fn encrypted_path_for(
        &self,
        reference: &SecretReference,
    ) -> Result<PathBuf, ContractError> {
        let mut full_path = self.secrets_root.clone();

        for segment in reference.namespace.split('/') {
            if !is_safe_segment(segment) {
                return Err(ContractError::InvalidSecretReferenceSyntax {
                    value: reference.to_uri(),
                });
            }
            full_path.push(segment);
        }

        for segment in reference.name.split('/') {
            if !is_safe_segment(segment) {
                return Err(ContractError::InvalidSecretReferenceSyntax {
                    value: reference.to_uri(),
                });
            }
            full_path.push(segment);
        }

        Ok(full_path.with_extension("gpg"))
    }

    pub fn resolve_reference(
        &self,
        reference: &SecretReference,
    ) -> Result<SecretValue, ContractError> {
        let encrypted_file = self.encrypted_path_for(reference)?;
        let reference_uri = reference.to_uri();

        match std::fs::metadata(&encrypted_file) {
            Ok(metadata) if metadata.is_file() => {}
            Ok(_) => {
                return Err(ContractError::SecretFileNotFound {
                    reference: reference_uri,
                    path: encrypted_file,
                });
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Err(ContractError::SecretFileNotFound {
                    reference: reference_uri,
                    path: encrypted_file,
                });
            }
            Err(source) => {
                return Err(ContractError::Io {
                    path: encrypted_file,
                    operation: "inspect secret file",
                    source,
                });
            }
        }

        let decrypted_payload = self.decryptor.decrypt(&encrypted_file).map_err(|_| {
            ContractError::SecretDecryptFailed {
                reference: reference.to_uri(),
                path: encrypted_file,
            }
        })?;

        extract_secret_value(reference, &decrypted_payload)
    }
}

pub fn redact_text(input: &str, secrets: &[SecretValue]) -> String {
    let mut redacted = input.to_owned();
    let mut values: BTreeSet<&str> = BTreeSet::new();

    for secret in secrets {
        if !secret.expose().is_empty() {
            values.insert(secret.expose());
        }
    }

    for value in values {
        redacted = redacted.replace(value, SECRET_REDACTION_TOKEN);
    }

    redacted
}

pub fn redact_json_snapshot(
    value: &serde_json::Value,
    secrets: &[SecretValue],
) -> serde_json::Value {
    match value {
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {
            value.clone()
        }
        serde_json::Value::String(inner) => serde_json::Value::String(redact_text(inner, secrets)),
        serde_json::Value::Array(items) => serde_json::Value::Array(
            items
                .iter()
                .map(|item| redact_json_snapshot(item, secrets))
                .collect(),
        ),
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.iter()
                .map(|(key, item)| (key.clone(), redact_json_snapshot(item, secrets)))
                .collect(),
        ),
    }
}

fn extract_secret_value(
    reference: &SecretReference,
    decrypted_payload: &str,
) -> Result<SecretValue, ContractError> {
    let reference_uri = reference.to_uri();
    let first_line = decrypted_payload
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .ok_or_else(|| ContractError::SecretPayloadEmpty {
            reference: reference_uri.clone(),
        })?;

    if let Some(key) = &reference.key {
        if key == "password" {
            return Ok(SecretValue::new(first_line.to_owned()));
        }

        let keyed_values = parse_keyed_secret_payload(decrypted_payload);
        let value =
            keyed_values
                .get(key)
                .cloned()
                .ok_or_else(|| ContractError::SecretKeyNotFound {
                    reference: reference_uri,
                    key: key.clone(),
                })?;
        return Ok(SecretValue::new(value));
    }

    Ok(SecretValue::new(first_line.to_owned()))
}

fn parse_keyed_secret_payload(decrypted_payload: &str) -> BTreeMap<String, String> {
    let mut keyed_values = BTreeMap::new();

    for line in decrypted_payload.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let pair = trimmed.split_once('=').or_else(|| trimmed.split_once(':'));
        let Some((raw_key, raw_value)) = pair else {
            continue;
        };

        let key = raw_key.trim();
        let value = raw_value.trim();
        if key.is_empty() || value.is_empty() {
            continue;
        }

        keyed_values.insert(key.to_owned(), value.to_owned());
    }

    keyed_values
}

fn is_safe_segment(value: &str) -> bool {
    !value.is_empty() && value != "." && value != ".."
}

impl FromStr for SecretReference {
    type Err = ContractError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let Some(path_and_anchor) = value.strip_prefix("secret://") else {
            return Err(ContractError::InvalidSecretReferenceSyntax {
                value: value.to_owned(),
            });
        };

        let (path_part, key) = match path_and_anchor.split_once('#') {
            Some((path, anchor)) if !anchor.trim().is_empty() => {
                (path, Some(anchor.trim().to_owned()))
            }
            Some(_) => {
                return Err(ContractError::InvalidSecretReferenceSyntax {
                    value: value.to_owned(),
                });
            }
            None => (path_and_anchor, None),
        };

        let Some((namespace_raw, name_raw)) = path_part.split_once('/') else {
            return Err(ContractError::InvalidSecretReferenceSyntax {
                value: value.to_owned(),
            });
        };

        let namespace = namespace_raw.trim();
        let name = name_raw.trim();

        if namespace.is_empty() || name.is_empty() || name.ends_with('/') {
            return Err(ContractError::InvalidSecretReferenceSyntax {
                value: value.to_owned(),
            });
        }

        Ok(Self {
            namespace: namespace.to_owned(),
            name: name.to_owned(),
            key,
        })
    }
}
