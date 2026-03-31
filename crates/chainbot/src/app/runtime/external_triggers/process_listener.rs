//! [INPUT]
//! External trigger definitions, plugin manifests, host-environment helpers, process I/O primitives, and trigger-plane contracts.
//!
//! [OUTPUT]
//! Starts and supervises process-backed external trigger listeners, translating plugin messages into domain trigger emissions and acknowledgements.
//!
//! [ROLE]
//! Owns the process-based external trigger runtime loop within the application execution layer.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::domain::state::StagedTriggerEventRecord;
use crate::domain::trigger::{
    TriggerAck, TriggerDefinition, TriggerEmission, TriggerHostMessage, TriggerPlaneError,
    TriggerPluginHostPolicy, TriggerPluginMessage, TriggerStartCommand, TriggerStateStore,
    TriggerStop, REQUIRED_TRIGGER_PLUGIN_CAPABILITY,
};
use crate::errors::ContractError;
use crate::plugin::{
    configure_plugin_subprocess_environment, PluginActivationEnvelope, PluginKind, PluginManifest,
    TriggerRuntimeLifecycle,
};
use crate::secrets::{
    redact_text, GpgSecretDecryptor, PlaintextSecretDecryptor, SecretProvider, SecretValue,
};

#[derive(Debug, Clone)]
struct ExternalTriggerPlugin {
    plugin_id: String,
    runtime: ExternalTriggerPluginRuntime,
}

#[derive(Debug, Clone)]
enum ExternalTriggerPluginRuntime {
    Process { executable_path: PathBuf },
    WasmPersistentSession,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ListenerSessionState {
    WaitingReady,
    Active,
    Draining,
    Stopped,
}

#[derive(Debug)]
enum ListenerFrame {
    Stdout(String),
    StdoutClosed,
    StdoutError(std::io::Error),
}

#[derive(Debug, Clone, Default)]
struct ResolvedTriggerActivation {
    activation: Option<PluginActivationEnvelope>,
    resolved_values: Vec<SecretValue>,
}

impl ListenerSessionState {
    fn protocol_error(self, plugin_id: &str, detail: impl Into<String>) -> ContractError {
        ContractError::TriggerPluginProtocolContractViolation {
            plugin_id: plugin_id.to_owned(),
            detail: detail.into(),
        }
    }

    fn handle_message(
        self,
        plugin_id: &str,
        message: &TriggerPluginMessage,
    ) -> Result<Self, ContractError> {
        match (self, message) {
            (Self::WaitingReady, TriggerPluginMessage::Ready(_)) => Ok(Self::Active),
            (Self::WaitingReady, TriggerPluginMessage::Heartbeat(_)) => {
                Err(self.protocol_error(plugin_id, "received heartbeat before ready"))
            }
            (Self::WaitingReady, TriggerPluginMessage::Event(_)) => {
                Err(self.protocol_error(plugin_id, "received event before ready"))
            }
            (Self::WaitingReady, TriggerPluginMessage::Fatal(_)) => Ok(Self::Draining),
            (Self::Active, TriggerPluginMessage::Ready(_)) => {
                Err(self.protocol_error(plugin_id, "received duplicate ready message"))
            }
            (Self::Active, TriggerPluginMessage::Heartbeat(_))
            | (Self::Active, TriggerPluginMessage::Event(_)) => Ok(Self::Active),
            (Self::Active, TriggerPluginMessage::Fatal(_)) => Ok(Self::Draining),
            (Self::Draining | Self::Stopped, TriggerPluginMessage::Ready(_))
            | (Self::Draining | Self::Stopped, TriggerPluginMessage::Heartbeat(_))
            | (Self::Draining | Self::Stopped, TriggerPluginMessage::Event(_))
            | (Self::Draining | Self::Stopped, TriggerPluginMessage::Fatal(_)) => {
                Err(self.protocol_error(plugin_id, "received message after listener stopped"))
            }
        }
    }
}

pub(crate) fn collect_external_process_trigger_emissions(
    state_store: &mut dyn TriggerStateStore,
    definition: &TriggerDefinition,
    manifest: &PluginManifest,
    policy: &TriggerPluginHostPolicy,
    on_progress: &mut dyn FnMut() -> Result<(), TriggerPlaneError>,
) -> Result<Vec<TriggerEmission>, TriggerPlaneError> {
    let plugin_id = definition.plugin.as_deref().ok_or_else(|| {
        ContractError::InvalidTriggerDefinitionField {
            trigger_id: definition.trigger_id.clone(),
            field: "trigger.plugin",
            detail: "value cannot be empty".to_owned(),
        }
    })?;
    if manifest.plugin_id != plugin_id {
        return Err(TriggerPlaneError::Contract(
            ContractError::UnknownTriggerPlugin {
                trigger_id: definition.trigger_id.clone(),
                plugin_id: plugin_id.to_owned(),
            },
        ));
    }

    let plugin = validate_trigger_plugin_manifest(manifest, policy)?;
    plugin
        .stream_emissions(state_store, definition, policy, on_progress)
        .map_err(TriggerPlaneError::from)
}

pub(crate) fn validate_external_trigger_plugin_manifest(
    manifest: &PluginManifest,
    policy: &TriggerPluginHostPolicy,
) -> Result<(), ContractError> {
    let _ = validate_trigger_plugin_manifest(manifest, policy)?;
    Ok(())
}

fn validate_trigger_plugin_manifest(
    manifest: &PluginManifest,
    policy: &TriggerPluginHostPolicy,
) -> Result<ExternalTriggerPlugin, ContractError> {
    manifest.validate()?;

    if !policy.allowlisted_plugin_ids.contains(&manifest.plugin_id) {
        return Err(ContractError::TriggerPluginNotAllowlisted {
            plugin_id: manifest.plugin_id.clone(),
        });
    }

    if manifest.kind()? != PluginKind::ExternalTrigger {
        return Err(ContractError::TriggerPluginInvalidKind {
            plugin_id: manifest.plugin_id.clone(),
            kind: manifest.kind.clone(),
        });
    }

    let required_capability = REQUIRED_TRIGGER_PLUGIN_CAPABILITY.to_owned();
    if !manifest.capabilities.contains(&required_capability) {
        return Err(ContractError::TriggerPluginMissingCapability {
            plugin_id: manifest.plugin_id.clone(),
            capability: required_capability,
        });
    }

    for capability in &manifest.capabilities {
        if !policy.allowed_capabilities.contains(capability) {
            return Err(ContractError::TriggerPluginCapabilityNotAllowed {
                plugin_id: manifest.plugin_id.clone(),
                capability: capability.clone(),
            });
        }
    }

    let lifecycle = manifest
        .trigger_runtime
        .as_ref()
        .and_then(|runtime| runtime.lifecycle)
        .ok_or_else(|| ContractError::NodePluginInvalidField {
            plugin_id: manifest.plugin_id.clone(),
            field: "plugin.trigger_runtime.lifecycle",
            detail: "field is required".to_owned(),
        })?;

    let runtime = match lifecycle {
        TriggerRuntimeLifecycle::WasmDaemonPersistentSession => {
            ExternalTriggerPluginRuntime::WasmPersistentSession
        }
        TriggerRuntimeLifecycle::ProcessShortLived => {
            let executable = manifest.executable.as_deref().ok_or_else(|| {
                ContractError::NodePluginInvalidField {
                    plugin_id: manifest.plugin_id.clone(),
                    field: "plugin.executable",
                    detail: "value cannot be empty".to_owned(),
                }
            })?;
            let executable_path =
                resolve_executable_path(manifest, executable, &policy.plugin_root_dir)?;
            ExternalTriggerPluginRuntime::Process { executable_path }
        }
    };

    Ok(ExternalTriggerPlugin {
        plugin_id: manifest.plugin_id.clone(),
        runtime,
    })
}

fn resolve_executable_path(
    manifest: &PluginManifest,
    entrypoint: &str,
    plugin_root_dir: &Path,
) -> Result<PathBuf, ContractError> {
    let entrypoint_path = Path::new(entrypoint);
    if entrypoint_path.is_absolute()
        || entrypoint_path
            .components()
            .any(|component| matches!(component, Component::RootDir))
    {
        return Err(ContractError::TriggerPluginEntrypointMustBeRelative {
            plugin_id: manifest.plugin_id.clone(),
            entrypoint: entrypoint.to_owned(),
        });
    }

    let manifest_root_dir = manifest
        .manifest_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(plugin_root_dir);

    let manifest_root =
        std::fs::canonicalize(manifest_root_dir).map_err(|source| ContractError::Io {
            path: manifest_root_dir.to_path_buf(),
            operation: "canonicalize trigger plugin manifest root",
            source,
        })?;
    let plugin_root =
        std::fs::canonicalize(plugin_root_dir).map_err(|source| ContractError::Io {
            path: plugin_root_dir.to_path_buf(),
            operation: "canonicalize trigger plugin root",
            source,
        })?;
    if entrypoint_escapes_root(&manifest_root, &plugin_root, entrypoint_path) {
        return Err(ContractError::TriggerPluginEntrypointEscapesRoot {
            plugin_id: manifest.plugin_id.clone(),
            entrypoint: entrypoint.to_owned(),
            root: plugin_root,
        });
    }
    let executable_path = manifest_root.join(entrypoint_path);

    if !executable_path.exists() {
        return Err(ContractError::TriggerPluginExecutableMissing {
            plugin_id: manifest.plugin_id.clone(),
            path: executable_path,
        });
    }

    let executable_canonical =
        std::fs::canonicalize(&executable_path).map_err(|source| ContractError::Io {
            path: executable_path.clone(),
            operation: "canonicalize trigger plugin executable",
            source,
        })?;

    if !executable_canonical.starts_with(&plugin_root) {
        return Err(ContractError::TriggerPluginEntrypointEscapesRoot {
            plugin_id: manifest.plugin_id.clone(),
            entrypoint: entrypoint.to_owned(),
            root: plugin_root,
        });
    }

    let metadata =
        std::fs::metadata(&executable_canonical).map_err(|source| ContractError::Io {
            path: executable_canonical.clone(),
            operation: "read trigger plugin executable metadata",
            source,
        })?;
    if !metadata.is_file() {
        return Err(ContractError::TriggerPluginExecutableNotFile {
            plugin_id: manifest.plugin_id.clone(),
            path: executable_canonical,
        });
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err(ContractError::TriggerPluginExecutableNotExecutable {
                plugin_id: manifest.plugin_id.clone(),
                path: executable_canonical,
            });
        }
    }

    Ok(executable_canonical)
}

impl ExternalTriggerPlugin {
    fn stream_emissions(
        &self,
        state_store: &mut dyn TriggerStateStore,
        definition: &TriggerDefinition,
        policy: &TriggerPluginHostPolicy,
        on_progress: &mut dyn FnMut() -> Result<(), TriggerPlaneError>,
    ) -> Result<Vec<TriggerEmission>, ContractError> {
        let executable_path = match &self.runtime {
            ExternalTriggerPluginRuntime::Process { executable_path } => executable_path,
            ExternalTriggerPluginRuntime::WasmPersistentSession => {
                return Ok(Vec::new());
            }
        };

        validate_existing_executable(&self.plugin_id, executable_path)?;
        let activation = resolve_trigger_activation(definition, policy)?;

        let input = TriggerHostMessage::Start(TriggerStartCommand {
            protocol_version: String::from("2.0.0"),
            trigger_id: definition.trigger_id.clone(),
            source: definition.source.clone(),
            params: definition.params.clone(),
            resume_checkpoint: state_store
                .read_trigger_checkpoint_for_acceptance(&definition.trigger_id)
                .map_err(|error| ContractError::InvalidTriggerEmission {
                    trigger_id: definition.trigger_id.clone(),
                    detail: error.to_string(),
                })?
                .map(|record| record.checkpoint),
            activation: activation.activation.clone(),
            heartbeat_interval_ms: 5_000,
            shutdown_grace_ms: 10_000,
        });
        input.validate()?;
        let encoded_input = serde_json::to_string(&input).map_err(|source| {
            ContractError::TriggerPluginProtocolEncode {
                plugin_id: self.plugin_id.clone(),
                source,
            }
        })?;

        let mut command = Command::new(executable_path);
        command
            .arg("--trigger-id")
            .arg(&definition.trigger_id)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        configure_plugin_subprocess_environment(&mut command);

        let mut child =
            command
                .spawn()
                .map_err(|source| ContractError::TriggerPluginSpawnFailed {
                    plugin_id: self.plugin_id.clone(),
                    path: executable_path.to_path_buf(),
                    source,
                })?;

        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(encoded_input.as_bytes())
                .map_err(|source| ContractError::TriggerPluginProcessIo {
                    plugin_id: self.plugin_id.clone(),
                    operation: "write stdin",
                    source,
                })?;
            stdin
                .write_all(b"\n")
                .map_err(|source| ContractError::TriggerPluginProcessIo {
                    plugin_id: self.plugin_id.clone(),
                    operation: "write stdin delimiter",
                    source,
                })?;
            stdin
                .flush()
                .map_err(|source| ContractError::TriggerPluginProcessIo {
                    plugin_id: self.plugin_id.clone(),
                    operation: "flush stdin",
                    source,
                })?;

            let stdout =
                child
                    .stdout
                    .take()
                    .ok_or_else(|| ContractError::TriggerPluginProcessIo {
                        plugin_id: self.plugin_id.clone(),
                        operation: "capture stdout",
                        source: std::io::Error::other("missing stdout pipe"),
                    })?;
            let stderr =
                child
                    .stderr
                    .take()
                    .ok_or_else(|| ContractError::TriggerPluginProcessIo {
                        plugin_id: self.plugin_id.clone(),
                        operation: "capture stderr",
                        source: std::io::Error::other("missing stderr pipe"),
                    })?;

            let stderr_handle = thread::spawn(move || -> std::io::Result<String> {
                let mut reader = BufReader::new(stderr);
                let mut output = String::new();
                reader.read_to_string(&mut output)?;
                Ok(output)
            });

            let (stdout_tx, stdout_rx) = mpsc::channel();
            let stdout_handle = thread::spawn(move || {
                let mut reader = BufReader::new(stdout);
                loop {
                    let mut line = String::new();
                    match reader.read_line(&mut line) {
                        Ok(0) => {
                            let _ = stdout_tx.send(ListenerFrame::StdoutClosed);
                            break;
                        }
                        Ok(_) => {
                            let _ = stdout_tx.send(ListenerFrame::Stdout(line));
                        }
                        Err(source) => {
                            let _ = stdout_tx.send(ListenerFrame::StdoutError(source));
                            break;
                        }
                    }
                }
            });

            let mut state = ListenerSessionState::WaitingReady;
            let heartbeat_interval_ms = input_heartbeat_interval_ms(&input);
            let mut last_activity_ms = contract_now_ms(definition)?;
            let mut staged_sequence = 0_u64;

            loop {
                let now_ms = contract_now_ms(definition)?;
                let timeout_ms = match state {
                    ListenerSessionState::WaitingReady => heartbeat_interval_ms,
                    ListenerSessionState::Active => {
                        let deadline = last_activity_ms.saturating_add(heartbeat_interval_ms * 2);
                        deadline
                            .saturating_sub(now_ms)
                            .clamp(1, heartbeat_interval_ms)
                    }
                    ListenerSessionState::Draining | ListenerSessionState::Stopped => 1,
                };

                match stdout_rx.recv_timeout(Duration::from_millis(timeout_ms as u64)) {
                    Ok(ListenerFrame::Stdout(line)) => {
                        on_progress().map_err(progress_to_contract_error(definition))?;
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        let message: TriggerPluginMessage =
                            serde_json::from_str(trimmed).map_err(|source| {
                                ContractError::TriggerPluginOutputDecode {
                                    plugin_id: self.plugin_id.clone(),
                                    source,
                                }
                            })?;
                        state = state.handle_message(&self.plugin_id, &message)?;
                        last_activity_ms = contract_now_ms(definition)?;

                        match message {
                            TriggerPluginMessage::Ready(_) | TriggerPluginMessage::Heartbeat(_) => {
                            }
                            TriggerPluginMessage::Fatal(fatal) => {
                                let _ = request_plugin_stop(
                                    &self.plugin_id,
                                    &mut stdin,
                                    TriggerStop {
                                        reason: String::from("plugin_fatal"),
                                    },
                                );
                                let _ = child.kill();
                                let _ = stdout_handle.join();
                                return Err(ContractError::TriggerPluginReturnedFailure {
                                    plugin_id: self.plugin_id.clone(),
                                    message: redact_text(
                                        &fatal.message,
                                        &activation.resolved_values,
                                    ),
                                });
                            }
                            TriggerPluginMessage::Event(event) => {
                                let staged_record = StagedTriggerEventRecord {
                                    schema_version: String::from("1.0.0"),
                                    staging_id: format!(
                                        "process:{}:{}:{}:{}",
                                        definition.trigger_id,
                                        event.event_key,
                                        event.occurred_at_ms,
                                        staged_sequence
                                    ),
                                    trigger_id: definition.trigger_id.clone(),
                                    workflow_id: definition.workflow_id.clone(),
                                    event_id: format!(
                                        "{}:{}",
                                        definition.trigger_id, event.event_key
                                    ),
                                    source: definition.source.clone(),
                                    occurred_at_ms: event.occurred_at_ms,
                                    staged_at_ms: contract_now_ms(definition)?,
                                    checkpoint: Some(event.checkpoint.clone()),
                                    payload: event.payload,
                                    dedup_key: event.dedup_key,
                                    dedup_window_ms: event.dedup_window_ms,
                                    cooldown_key: event.cooldown_key,
                                    cooldown_ms: event.cooldown_ms,
                                    accepted_at_ms: None,
                                    last_error: None,
                                };
                                state_store
                                    .append_staged_trigger_event_record_for_acceptance(
                                        &staged_record,
                                    )
                                    .map_err(progress_to_contract_error(definition))?;
                                staged_sequence = staged_sequence.saturating_add(1);

                                let ack = TriggerHostMessage::Ack(TriggerAck {
                                    checkpoint: event.checkpoint,
                                });
                                let encoded_ack =
                                    serde_json::to_string(&ack).map_err(|source| {
                                        ContractError::TriggerPluginProtocolEncode {
                                            plugin_id: self.plugin_id.clone(),
                                            source,
                                        }
                                    })?;
                                if let Err(source) = stdin
                                    .write_all(encoded_ack.as_bytes())
                                    .and_then(|_| stdin.write_all(b"\n"))
                                    .and_then(|_| stdin.flush())
                                {
                                    if source.kind() != std::io::ErrorKind::BrokenPipe {
                                        return Err(ContractError::TriggerPluginProcessIo {
                                            plugin_id: self.plugin_id.clone(),
                                            operation: "write ack to plugin stdin",
                                            source,
                                        });
                                    }
                                }
                            }
                        }
                    }
                    Ok(ListenerFrame::StdoutClosed) => {
                        state = ListenerSessionState::Stopped;
                        break;
                    }
                    Ok(ListenerFrame::StdoutError(source)) => {
                        return Err(ContractError::TriggerPluginProcessIo {
                            plugin_id: self.plugin_id.clone(),
                            operation: "read stdout",
                            source,
                        });
                    }
                    Err(RecvTimeoutError::Timeout) => {
                        on_progress().map_err(progress_to_contract_error(definition))?;
                        let now_ms = contract_now_ms(definition)?;
                        if heartbeat_timed_out(
                            state,
                            last_activity_ms,
                            now_ms,
                            heartbeat_interval_ms,
                        ) {
                            let _ = request_plugin_stop(
                                &self.plugin_id,
                                &mut stdin,
                                TriggerStop {
                                    reason: String::from("heartbeat_timeout"),
                                },
                            );
                            let _ = child.kill();
                            let _ = stdout_handle.join();
                            return Err(ContractError::TriggerPluginProtocolContractViolation {
                                plugin_id: self.plugin_id.clone(),
                                detail: String::from("heartbeat timed out after ready"),
                            });
                        }
                    }
                    Err(RecvTimeoutError::Disconnected) => {
                        state = ListenerSessionState::Stopped;
                        break;
                    }
                }
            }

            let _ = stdout_handle.join();

            if state == ListenerSessionState::WaitingReady {
                return Err(ContractError::TriggerPluginProtocolContractViolation {
                    plugin_id: self.plugin_id.clone(),
                    detail: String::from("listener exited before ready"),
                });
            }

            let status = child
                .wait()
                .map_err(|source| ContractError::TriggerPluginProcessIo {
                    plugin_id: self.plugin_id.clone(),
                    operation: "wait for process",
                    source,
                })?;
            let stderr = stderr_handle
                .join()
                .map_err(|_| ContractError::TriggerPluginProcessIo {
                    plugin_id: self.plugin_id.clone(),
                    operation: "join stderr reader",
                    source: std::io::Error::other("stderr reader panicked"),
                })?
                .map_err(|source| ContractError::TriggerPluginProcessIo {
                    plugin_id: self.plugin_id.clone(),
                    operation: "read stderr",
                    source,
                })?;

            if !status.success() {
                return Err(ContractError::TriggerPluginProcessFailed {
                    plugin_id: self.plugin_id.clone(),
                    status: status.code().unwrap_or(-1),
                    stderr: redact_text(stderr.trim(), &activation.resolved_values),
                });
            }

            return Ok(Vec::<TriggerEmission>::new());
        }

        Err(ContractError::TriggerPluginProcessIo {
            plugin_id: self.plugin_id.clone(),
            operation: "open stdin",
            source: std::io::Error::other("missing stdin pipe"),
        })
    }
}

fn input_heartbeat_interval_ms(input: &TriggerHostMessage) -> i64 {
    match input {
        TriggerHostMessage::Start(command) => command.heartbeat_interval_ms.max(1),
        TriggerHostMessage::Ack(_) | TriggerHostMessage::Stop(_) => 1,
    }
}

fn heartbeat_timed_out(
    state: ListenerSessionState,
    last_activity_ms: i64,
    now_ms: i64,
    heartbeat_interval_ms: i64,
) -> bool {
    state == ListenerSessionState::Active
        && now_ms >= last_activity_ms.saturating_add(heartbeat_interval_ms.saturating_mul(2))
}

fn contract_now_ms(definition: &TriggerDefinition) -> Result<i64, ContractError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|source| ContractError::InvalidTriggerEmission {
            trigger_id: definition.trigger_id.clone(),
            detail: format!("system time before UNIX_EPOCH: {source}"),
        })?;
    i64::try_from(duration.as_millis()).map_err(|source| ContractError::InvalidTriggerEmission {
        trigger_id: definition.trigger_id.clone(),
        detail: format!("system time overflowed i64 millis: {source}"),
    })
}

fn resolve_trigger_activation(
    definition: &TriggerDefinition,
    policy: &TriggerPluginHostPolicy,
) -> Result<ResolvedTriggerActivation, ContractError> {
    let Some(plugin_id) = definition.plugin.as_deref() else {
        return Ok(ResolvedTriggerActivation::default());
    };
    let Some(bindings) = policy.plugin_activation.get(plugin_id) else {
        return Ok(ResolvedTriggerActivation::default());
    };
    if bindings.is_empty() {
        return Ok(ResolvedTriggerActivation::default());
    }

    let mut secrets = Vec::with_capacity(bindings.len());
    let mut resolved = std::collections::BTreeMap::new();
    if std::env::var("CHAINBOT_SECRET_DECRYPTOR").ok().as_deref() == Some("plaintext") {
        let provider =
            SecretProvider::new(policy.secrets_root_dir.clone(), PlaintextSecretDecryptor);
        for (slot, reference) in bindings {
            let value = provider.resolve_reference(reference)?;
            resolved.insert(slot.clone(), value.expose().to_owned());
            secrets.push(value);
        }
    } else {
        let provider =
            SecretProvider::new(policy.secrets_root_dir.clone(), GpgSecretDecryptor::new());
        for (slot, reference) in bindings {
            let value = provider.resolve_reference(reference)?;
            resolved.insert(slot.clone(), value.expose().to_owned());
            secrets.push(value);
        }
    }

    Ok(ResolvedTriggerActivation {
        activation: Some(PluginActivationEnvelope { secrets: resolved }),
        resolved_values: secrets,
    })
}

fn progress_to_contract_error<'a>(
    definition: &'a TriggerDefinition,
) -> impl FnOnce(TriggerPlaneError) -> ContractError + 'a {
    move |error| ContractError::InvalidTriggerEmission {
        trigger_id: definition.trigger_id.clone(),
        detail: error.to_string(),
    }
}

fn request_plugin_stop(
    plugin_id: &str,
    stdin: &mut dyn Write,
    stop: TriggerStop,
) -> Result<(), ContractError> {
    let encoded_stop =
        serde_json::to_string(&TriggerHostMessage::Stop(stop)).map_err(|source| {
            ContractError::TriggerPluginProtocolEncode {
                plugin_id: plugin_id.to_owned(),
                source,
            }
        })?;
    stdin
        .write_all(encoded_stop.as_bytes())
        .and_then(|_| stdin.write_all(b"\n"))
        .and_then(|_| stdin.flush())
        .map_err(|source| ContractError::TriggerPluginProcessIo {
            plugin_id: plugin_id.to_owned(),
            operation: "write stop to plugin stdin",
            source,
        })
}

fn entrypoint_escapes_root(base_dir: &Path, root_dir: &Path, entrypoint_path: &Path) -> bool {
    let mut depth = match base_dir.strip_prefix(root_dir) {
        Ok(relative) => relative
            .components()
            .filter(|component| matches!(component, Component::Normal(_)))
            .count() as isize,
        Err(_) => 0,
    };

    for component in entrypoint_path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(_) => depth += 1,
            Component::ParentDir => {
                depth -= 1;
                if depth < 0 {
                    return true;
                }
            }
            Component::Prefix(_) | Component::RootDir => return true,
        }
    }

    false
}

fn validate_existing_executable(
    plugin_id: &str,
    executable_path: &Path,
) -> Result<(), ContractError> {
    if !executable_path.exists() {
        return Err(ContractError::TriggerPluginExecutableMissing {
            plugin_id: plugin_id.to_owned(),
            path: executable_path.to_path_buf(),
        });
    }

    let metadata = std::fs::metadata(executable_path).map_err(|source| ContractError::Io {
        path: executable_path.to_path_buf(),
        operation: "read trigger plugin executable metadata",
        source,
    })?;
    if !metadata.is_file() {
        return Err(ContractError::TriggerPluginExecutableNotFile {
            plugin_id: plugin_id.to_owned(),
            path: executable_path.to_path_buf(),
        });
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err(ContractError::TriggerPluginExecutableNotExecutable {
                plugin_id: plugin_id.to_owned(),
                path: executable_path.to_path_buf(),
            });
        }
    }

    Ok(())
}
