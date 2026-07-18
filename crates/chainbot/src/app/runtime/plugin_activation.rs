//! [INPUT]
//! Root plugin-activation bindings, secret runtime configuration, and plugin host errors.
//!
//! [OUTPUT]
//! Resolves one activation envelope and redaction context for node and trigger plugin hosts.
//!
//! [ROLE]
//! Centralizes execution-time plugin activation preparation inside the app runtime seam.

use std::collections::BTreeMap;
use std::path::Path;

use crate::builtins::nodes::SecretDecryptMode;
use crate::domain::trigger::TriggerPluginActivationBindings;
use crate::errors::ContractError;
use crate::infrastructure::config::RootConfigDefinition;
use crate::plugin::PluginActivationEnvelope;
use crate::secrets::{
    redact_text, GpgSecretDecryptor, PlaintextSecretDecryptor, SecretDecryptor, SecretProvider,
    SecretReference, SecretValue,
};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PluginActivationRuntime {
    pub secret_bindings: BTreeMap<String, SecretReference>,
    pub allowed_origins: Vec<String>,
}

pub(crate) trait PluginActivationBindings {
    fn secret_bindings(&self) -> &BTreeMap<String, SecretReference>;
    fn allowed_origins(&self) -> &[String];
}

impl PluginActivationBindings for PluginActivationRuntime {
    fn secret_bindings(&self) -> &BTreeMap<String, SecretReference> {
        &self.secret_bindings
    }

    fn allowed_origins(&self) -> &[String] {
        &self.allowed_origins
    }
}

impl PluginActivationBindings for TriggerPluginActivationBindings {
    fn secret_bindings(&self) -> &BTreeMap<String, SecretReference> {
        &self.secret_bindings
    }

    fn allowed_origins(&self) -> &[String] {
        &self.allowed_origins
    }
}

pub(crate) struct PluginActivationResolver<'a, B> {
    bindings: &'a BTreeMap<String, B>,
    secrets_root: &'a Path,
    secret_mode: SecretDecryptMode,
}

impl<'a, B> PluginActivationResolver<'a, B>
where
    B: PluginActivationBindings,
{
    pub(crate) fn new(
        bindings: &'a BTreeMap<String, B>,
        secrets_root: &'a Path,
        secret_mode: SecretDecryptMode,
    ) -> Self {
        Self {
            bindings,
            secrets_root,
            secret_mode,
        }
    }

    pub(crate) fn resolve(
        &self,
        plugin_id: &str,
    ) -> Result<ResolvedPluginActivation, ContractError> {
        let Some(bindings) = self.bindings.get(plugin_id) else {
            return Ok(ResolvedPluginActivation::default());
        };
        if bindings.secret_bindings().is_empty() && bindings.allowed_origins().is_empty() {
            return Ok(ResolvedPluginActivation::default());
        }

        match self.secret_mode {
            SecretDecryptMode::Gpg => self.resolve_with(
                bindings,
                SecretProvider::new(self.secrets_root.to_path_buf(), GpgSecretDecryptor::new()),
            ),
            SecretDecryptMode::Plaintext => self.resolve_with(
                bindings,
                SecretProvider::new(self.secrets_root.to_path_buf(), PlaintextSecretDecryptor),
            ),
        }
    }

    #[allow(clippy::result_large_err)]
    fn resolve_with<D>(
        &self,
        bindings: &B,
        provider: SecretProvider<D>,
    ) -> Result<ResolvedPluginActivation, ContractError>
    where
        D: SecretDecryptor,
    {
        let mut resolved = BTreeMap::new();
        let mut secrets = Vec::with_capacity(bindings.secret_bindings().len());
        for (slot, reference) in bindings.secret_bindings() {
            let value = provider.resolve_reference(reference)?;
            resolved.insert(slot.clone(), value.expose().to_owned());
            secrets.push(value);
        }

        Ok(ResolvedPluginActivation {
            envelope: Some(PluginActivationEnvelope {
                secrets: resolved,
                allowed_origins: bindings.allowed_origins().to_vec(),
            }),
            redactor: PluginActivationRedactor { secrets },
        })
    }
}

#[derive(Default)]
pub(crate) struct ResolvedPluginActivation {
    envelope: Option<PluginActivationEnvelope>,
    redactor: PluginActivationRedactor,
}

impl ResolvedPluginActivation {
    pub(crate) fn envelope(&self) -> Option<PluginActivationEnvelope> {
        self.envelope.clone()
    }

    pub(crate) fn is_configured(&self) -> bool {
        self.envelope.is_some()
    }

    pub(crate) fn redact_error(&self, error: ContractError) -> ContractError {
        self.redactor.redact_error(error)
    }

    pub(crate) fn into_redactor(self) -> PluginActivationRedactor {
        self.redactor
    }
}

#[derive(Clone, Default)]
pub(crate) struct PluginActivationRedactor {
    secrets: Vec<SecretValue>,
}

impl PluginActivationRedactor {
    pub(crate) fn redact_error(&self, error: ContractError) -> ContractError {
        match error {
            ContractError::NodePluginProcessFailed {
                plugin_id,
                exit_code,
                stderr,
            } => ContractError::NodePluginProcessFailed {
                plugin_id,
                exit_code,
                stderr: redact_text(&stderr, &self.secrets),
            },
            ContractError::NodePluginProtocolContractViolation { plugin_id, detail } => {
                ContractError::NodePluginProtocolContractViolation {
                    plugin_id,
                    detail: redact_text(&detail, &self.secrets),
                }
            }
            ContractError::NodePluginReturnedFailure { plugin_id, message } => {
                ContractError::NodePluginReturnedFailure {
                    plugin_id,
                    message: redact_text(&message, &self.secrets),
                }
            }
            ContractError::TriggerPluginProcessFailed {
                plugin_id,
                status,
                stderr,
            } => ContractError::TriggerPluginProcessFailed {
                plugin_id,
                status,
                stderr: redact_text(&stderr, &self.secrets),
            },
            ContractError::TriggerPluginProtocolContractViolation { plugin_id, detail } => {
                ContractError::TriggerPluginProtocolContractViolation {
                    plugin_id,
                    detail: redact_text(&detail, &self.secrets),
                }
            }
            ContractError::TriggerPluginReturnedFailure { plugin_id, message } => {
                ContractError::TriggerPluginReturnedFailure {
                    plugin_id,
                    message: redact_text(&message, &self.secrets),
                }
            }
            other => other,
        }
    }
}

#[allow(clippy::result_large_err)]
pub(crate) fn parse_plugin_activation_bindings(
    root_config: &RootConfigDefinition,
) -> Result<BTreeMap<String, PluginActivationRuntime>, ContractError> {
    root_config
        .plugin_activation
        .iter()
        .map(|(plugin_id, activation)| {
            let secret_bindings = activation
                .secret_bindings
                .iter()
                .map(|(slot, secret_ref)| {
                    SecretReference::parse(secret_ref)
                        .map(|reference| (slot.clone(), reference))
                })
                .collect::<Result<_, _>>()?;
            Ok((
                plugin_id.clone(),
                PluginActivationRuntime {
                    secret_bindings,
                    allowed_origins: activation.allowed_origins.clone(),
                },
            ))
        })
        .collect()
}

#[allow(clippy::result_large_err)]
pub(crate) fn parse_trigger_plugin_activation_bindings(
    root_config: &RootConfigDefinition,
) -> Result<BTreeMap<String, TriggerPluginActivationBindings>, ContractError> {
    parse_plugin_activation_bindings(root_config).map(|bindings| {
        bindings
            .into_iter()
            .map(|(plugin_id, activation)| {
                (
                    plugin_id,
                    TriggerPluginActivationBindings {
                        secret_bindings: activation.secret_bindings,
                        allowed_origins: activation.allowed_origins,
                    },
                )
            })
            .collect()
    })
}

pub(crate) fn trigger_plugin_secret_mode() -> SecretDecryptMode {
    if std::env::var("CHAINBOT_SECRET_DECRYPTOR").ok().as_deref() == Some("plaintext") {
        SecretDecryptMode::Plaintext
    } else {
        SecretDecryptMode::Gpg
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn resolves_one_activation_shape_and_redacts_node_and_trigger_errors() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("test clock should follow UNIX epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("chainbot-plugin-activation-{unique}"));
        let secret_dir = root.join("providers");
        fs::create_dir_all(&secret_dir).expect("secret directory should be created");
        fs::write(secret_dir.join("example.gpg"), "token = super-secret\n")
            .expect("plaintext secret fixture should be written");

        let bindings = BTreeMap::from([(
            String::from("example-plugin"),
            PluginActivationRuntime {
                secret_bindings: BTreeMap::from([(
                    String::from("authorization"),
                    SecretReference::parse("secret://providers/example#token")
                        .expect("secret reference should parse"),
                )]),
                allowed_origins: vec![String::from("https://api.example.test")],
            },
        )]);
        let resolver = PluginActivationResolver::new(
            &bindings,
            &root,
            SecretDecryptMode::Plaintext,
        );

        let activation = resolver
            .resolve("example-plugin")
            .expect("activation should resolve");
        let envelope = activation.envelope().expect("activation should be configured");
        assert_eq!(
            envelope.secrets.get("authorization").map(String::as_str),
            Some("super-secret")
        );
        assert_eq!(
            envelope.allowed_origins,
            vec![String::from("https://api.example.test")]
        );

        let node_error = activation.redact_error(ContractError::NodePluginReturnedFailure {
            plugin_id: String::from("example-plugin"),
            message: String::from("rejected super-secret"),
        });
        assert!(node_error.to_string().contains("[REDACTED_SECRET]"));
        assert!(!node_error.to_string().contains("super-secret"));

        let trigger_error = activation.redact_error(ContractError::TriggerPluginProcessFailed {
            plugin_id: String::from("example-plugin"),
            status: 1,
            stderr: String::from("failed with super-secret"),
        });
        assert!(trigger_error.to_string().contains("[REDACTED_SECRET]"));
        assert!(!trigger_error.to_string().contains("super-secret"));

        fs::remove_dir_all(root).expect("activation fixture should be removed");
    }
}
