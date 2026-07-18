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
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError, TryRecvError};
use std::thread;
use std::time::{Duration, Instant};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::domain::state::StagedTriggerEventRecord;
use crate::domain::trigger::{
    TriggerAck, TriggerDefinition, TriggerEmission, TriggerHostMessage, TriggerPlaneError,
    TriggerPluginHostPolicy, TriggerPluginMessage, TriggerStartCommand, TriggerStateStore,
    TriggerStop, REQUIRED_TRIGGER_PLUGIN_CAPABILITY,
};
use crate::errors::ContractError;
use crate::plugin::{
    configure_plugin_subprocess_environment, configure_process_group, terminate_child_group,
    HostCancellation, PluginActivationEnvelope, PluginKind, PluginManifest,
    TriggerRuntimeLifecycle,
};
use crate::secrets::{
    redact_text, GpgSecretDecryptor, PlaintextSecretDecryptor, SecretProvider, SecretValue,
};

const MAX_TRIGGER_FRAME_BYTES: usize = 64 * 1024;
const MAX_TRIGGER_STDERR_BYTES: usize = 256 * 1024;
const MAX_PENDING_TRIGGER_FRAMES: usize = 256;
const MAX_SHORT_LIVED_TRIGGER_EVENTS: u64 = 256;
const SHORT_LIVED_TRIGGER_DEADLINE: Duration = Duration::from_secs(60);
const MANAGED_DRAIN_FRAME_BUDGET: usize = 256;
const MANAGED_DRAIN_TIME_BUDGET: Duration = Duration::from_millis(10);
const MANAGED_CONTROL_QUEUE_CAPACITY: usize = 16;
const MANAGED_CONTROL_WRITE_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Debug, Clone)]
struct ExternalTriggerPlugin {
    plugin_id: String,
    manifest: PluginManifest,
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
    Bytes(Vec<u8>),
    FrameTooLarge,
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
        TriggerRuntimeLifecycle::ProcessShortLived | TriggerRuntimeLifecycle::ProcessDaemonSession => {
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
        manifest: manifest.clone(),
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
        let activation = resolve_trigger_activation(&self.manifest, definition, policy)?;

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
        configure_process_group(&mut command);

        let mut child =
            command
                .spawn()
                .map_err(|source| ContractError::TriggerPluginSpawnFailed {
                    plugin_id: self.plugin_id.clone(),
                    path: executable_path.to_path_buf(),
                    source,
                })?;

        if let Some(mut stdin) = child.stdin.take() {
            if let Err(source) = stdin
                .write_all(encoded_input.as_bytes())
                .and_then(|_| stdin.write_all(b"\n"))
                .and_then(|_| stdin.flush())
            {
                terminate_child_group(&mut child, Duration::ZERO);
                let _ = child.wait();
                return Err(ContractError::TriggerPluginProcessIo {
                    plugin_id: self.plugin_id.clone(),
                    operation: "write stdin",
                    source,
                });
            }

            let stdout = match child.stdout.take() {
                Some(stdout) => stdout,
                None => {
                    terminate_child_group(&mut child, Duration::ZERO);
                    let _ = child.wait();
                    return Err(ContractError::TriggerPluginProcessIo {
                        plugin_id: self.plugin_id.clone(),
                        operation: "capture stdout",
                        source: std::io::Error::other("missing stdout pipe"),
                    });
                }
            };
            let stderr = match child.stderr.take() {
                Some(stderr) => stderr,
                None => {
                    terminate_child_group(&mut child, Duration::ZERO);
                    let _ = child.wait();
                    return Err(ContractError::TriggerPluginProcessIo {
                        plugin_id: self.plugin_id.clone(),
                        operation: "capture stderr",
                        source: std::io::Error::other("missing stderr pipe"),
                    });
                }
            };

            let mut stderr_handle = Some(thread::spawn(move || -> std::io::Result<String> {
                let mut reader = BufReader::new(stderr);
                let mut tail = Vec::new();
                let mut chunk = [0_u8; 8192];
                loop {
                    let read = reader.read(&mut chunk)?;
                    if read == 0 {
                        return Ok(String::from_utf8_lossy(&tail).into_owned());
                    }
                    tail.extend_from_slice(&chunk[..read]);
                    if tail.len() > MAX_TRIGGER_STDERR_BYTES {
                        let excess = tail.len().saturating_sub(MAX_TRIGGER_STDERR_BYTES);
                        tail.drain(..excess);
                    }
                }
            }));

            let (stdout_tx, stdout_rx) = mpsc::sync_channel(MAX_PENDING_TRIGGER_FRAMES);
            let mut stdout_handle = Some(
                thread::spawn(move || read_managed_stdout(stdout, stdout_tx)),
            );

            let mut state = ListenerSessionState::WaitingReady;
            let heartbeat_interval_ms = input_heartbeat_interval_ms(&input);
            let mut last_activity_ms = match contract_now_ms(definition) {
                Ok(now_ms) => now_ms,
                Err(error) => {
                    cleanup_short_lived_process(
                        &self.plugin_id,
                        &mut child,
                        &mut stdin,
                        &mut stdout_handle,
                        &mut stderr_handle,
                    );
                    return Err(error);
                }
            };
            let mut staged_sequence = 0_u64;
            let deadline = Instant::now() + SHORT_LIVED_TRIGGER_DEADLINE;

            loop {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    let _ = request_plugin_stop(
                        &self.plugin_id,
                        &mut stdin,
                        TriggerStop {
                            reason: String::from("deadline_exceeded"),
                        },
                    );
                    terminate_child_group(&mut child, Duration::from_secs(3));
                    let _ = stdout_handle.take().map(|handle| handle.join());
                    let _ = stderr_handle.take().map(|handle| handle.join());
                    return Err(ContractError::TriggerPluginProtocolContractViolation {
                        plugin_id: self.plugin_id.clone(),
                        detail: String::from("short-lived listener exceeded 60 second deadline"),
                    });
                }
                let now_ms = match contract_now_ms(definition) {
                    Ok(now_ms) => now_ms,
                    Err(error) => {
                        cleanup_short_lived_process(
                            &self.plugin_id,
                            &mut child,
                            &mut stdin,
                            &mut stdout_handle,
                            &mut stderr_handle,
                        );
                        return Err(error);
                    }
                };
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

                match stdout_rx.recv_timeout(
                    Duration::from_millis(timeout_ms as u64).min(remaining),
                ) {
                    Ok(ListenerFrame::Bytes(frame)) => {
                        if let Err(error) = on_progress().map_err(progress_to_contract_error(definition)) {
                            cleanup_short_lived_process(
                                &self.plugin_id,
                                &mut child,
                                &mut stdin,
                                &mut stdout_handle,
                                &mut stderr_handle,
                            );
                            return Err(error);
                        }
                        if frame.iter().all(u8::is_ascii_whitespace) {
                            continue;
                        }
                        let message: TriggerPluginMessage = match serde_json::from_slice(&frame) {
                            Ok(message) => message,
                            Err(source) => {
                                cleanup_short_lived_process(
                                    &self.plugin_id,
                                    &mut child,
                                    &mut stdin,
                                    &mut stdout_handle,
                                    &mut stderr_handle,
                                );
                                return Err(ContractError::TriggerPluginOutputDecode {
                                    plugin_id: self.plugin_id.clone(),
                                    source,
                                });
                            }
                        };
                        state = match state.handle_message(&self.plugin_id, &message) {
                            Ok(state) => state,
                            Err(error) => {
                                cleanup_short_lived_process(
                                    &self.plugin_id,
                                    &mut child,
                                    &mut stdin,
                                    &mut stdout_handle,
                                    &mut stderr_handle,
                                );
                                return Err(error);
                            }
                        };
                        last_activity_ms = match contract_now_ms(definition) {
                            Ok(now_ms) => now_ms,
                            Err(error) => {
                                cleanup_short_lived_process(
                                    &self.plugin_id,
                                    &mut child,
                                    &mut stdin,
                                    &mut stdout_handle,
                                    &mut stderr_handle,
                                );
                                return Err(error);
                            }
                        };

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
                                let _ = stdout_handle.take().map(|handle| handle.join());
                                let _ = stderr_handle.take().map(|handle| handle.join());
                                return Err(ContractError::TriggerPluginReturnedFailure {
                                    plugin_id: self.plugin_id.clone(),
                                    message: redact_text(
                                        &fatal.message,
                                        &activation.resolved_values,
                                    ),
                                });
                            }
                            TriggerPluginMessage::Event(event) => {
                                if staged_sequence >= MAX_SHORT_LIVED_TRIGGER_EVENTS {
                                    let _ = request_plugin_stop(
                                        &self.plugin_id,
                                        &mut stdin,
                                        TriggerStop {
                                            reason: String::from("event_budget_exceeded"),
                                        },
                                    );
                                    terminate_child_group(&mut child, Duration::from_secs(3));
                                    let _ = stdout_handle.take().map(|handle| handle.join());
                                    let _ = stderr_handle.take().map(|handle| handle.join());
                                    return Err(ContractError::TriggerPluginProtocolContractViolation {
                                        plugin_id: self.plugin_id.clone(),
                                        detail: format!(
                                            "short-lived listener exceeded {MAX_SHORT_LIVED_TRIGGER_EVENTS} event budget"
                                        ),
                                    });
                                }
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
                                    staged_at_ms: match contract_now_ms(definition) {
                                        Ok(now_ms) => now_ms,
                                        Err(error) => {
                                            cleanup_short_lived_process(
                                                &self.plugin_id,
                                                &mut child,
                                                &mut stdin,
                                                &mut stdout_handle,
                                                &mut stderr_handle,
                                            );
                                            return Err(error);
                                        }
                                    },
                                    checkpoint: Some(event.checkpoint.clone()),
                                    payload: event.payload,
                                    dedup_key: event.dedup_key,
                                    dedup_window_ms: event.dedup_window_ms,
                                    cooldown_key: event.cooldown_key,
                                    cooldown_ms: event.cooldown_ms,
                                    accepted_at_ms: None,
                                    last_error: None,
                                };
                                if let Err(error) = state_store
                                    .append_staged_trigger_event_record_for_acceptance(&staged_record)
                                    .map_err(progress_to_contract_error(definition))
                                {
                                    cleanup_short_lived_process(
                                        &self.plugin_id,
                                        &mut child,
                                        &mut stdin,
                                        &mut stdout_handle,
                                        &mut stderr_handle,
                                    );
                                    return Err(error);
                                }
                                staged_sequence = staged_sequence.saturating_add(1);

                                let ack = TriggerHostMessage::Ack(TriggerAck {
                                    checkpoint: event.checkpoint,
                                });
                                let encoded_ack = match serde_json::to_string(&ack) {
                                    Ok(encoded_ack) => encoded_ack,
                                    Err(source) => {
                                        cleanup_short_lived_process(
                                            &self.plugin_id,
                                            &mut child,
                                            &mut stdin,
                                            &mut stdout_handle,
                                            &mut stderr_handle,
                                        );
                                        return Err(ContractError::TriggerPluginProtocolEncode {
                                            plugin_id: self.plugin_id.clone(),
                                            source,
                                        });
                                    }
                                };
                                if let Err(source) = stdin
                                    .write_all(encoded_ack.as_bytes())
                                    .and_then(|_| stdin.write_all(b"\n"))
                                    .and_then(|_| stdin.flush())
                                    && source.kind() != std::io::ErrorKind::BrokenPipe
                                {
                                    let error = ContractError::TriggerPluginProcessIo {
                                        plugin_id: self.plugin_id.clone(),
                                        operation: "write ack to plugin stdin",
                                        source,
                                    };
                                    cleanup_short_lived_process(
                                        &self.plugin_id,
                                        &mut child,
                                        &mut stdin,
                                        &mut stdout_handle,
                                        &mut stderr_handle,
                                    );
                                    return Err(error);
                                }
                            }
                        }
                    }
                    Ok(ListenerFrame::FrameTooLarge) => {
                        terminate_child_group(&mut child, Duration::from_secs(3));
                        let _ = stdout_handle.take().map(|handle| handle.join());
                        let _ = stderr_handle.take().map(|handle| handle.join());
                        return Err(ContractError::TriggerPluginProtocolContractViolation {
                            plugin_id: self.plugin_id.clone(),
                            detail: format!("trigger frame exceeds {MAX_TRIGGER_FRAME_BYTES} bytes"),
                        });
                    }
                    Ok(ListenerFrame::StdoutClosed) => {
                        state = ListenerSessionState::Stopped;
                        break;
                    }
                    Ok(ListenerFrame::StdoutError(source)) => {
                        let error = ContractError::TriggerPluginProcessIo {
                            plugin_id: self.plugin_id.clone(),
                            operation: "read stdout",
                            source,
                        };
                        cleanup_short_lived_process(
                            &self.plugin_id,
                            &mut child,
                            &mut stdin,
                            &mut stdout_handle,
                            &mut stderr_handle,
                        );
                        return Err(error);
                    }
                    Err(RecvTimeoutError::Timeout) => {
                        if let Err(error) = on_progress().map_err(progress_to_contract_error(definition)) {
                            cleanup_short_lived_process(
                                &self.plugin_id,
                                &mut child,
                                &mut stdin,
                                &mut stdout_handle,
                                &mut stderr_handle,
                            );
                            return Err(error);
                        }
                        let now_ms = match contract_now_ms(definition) {
                            Ok(now_ms) => now_ms,
                            Err(error) => {
                                cleanup_short_lived_process(
                                    &self.plugin_id,
                                    &mut child,
                                    &mut stdin,
                                    &mut stdout_handle,
                                    &mut stderr_handle,
                                );
                                return Err(error);
                            }
                        };
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
                            let _ = stdout_handle.take().map(|handle| handle.join());
                            let _ = stderr_handle.take().map(|handle| handle.join());
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

            let _ = stdout_handle.take().map(|handle| handle.join());

            if state == ListenerSessionState::WaitingReady {
                let error = ContractError::TriggerPluginProtocolContractViolation {
                    plugin_id: self.plugin_id.clone(),
                    detail: String::from("listener exited before ready"),
                };
                cleanup_short_lived_process(
                    &self.plugin_id,
                    &mut child,
                    &mut stdin,
                    &mut stdout_handle,
                    &mut stderr_handle,
                );
                return Err(error);
            }

            let status = match child.wait() {
                Ok(status) => status,
                Err(source) => {
                    let error = ContractError::TriggerPluginProcessIo {
                        plugin_id: self.plugin_id.clone(),
                        operation: "wait for process",
                        source,
                    };
                    terminate_child_group(&mut child, Duration::ZERO);
                    let _ = child.wait();
                    let _ = stderr_handle.take().map(|handle| handle.join());
                    return Err(error);
                }
            };
            let stderr = stderr_handle
                .take()
                .ok_or_else(|| ContractError::TriggerPluginProcessIo {
                    plugin_id: self.plugin_id.clone(),
                    operation: "join stderr reader",
                    source: std::io::Error::other("stderr reader handle missing"),
                })?
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

pub(crate) struct ProcessTriggerSession {
    trigger_id: String,
    plugin_id: String,
    identity: String,
    child: Child,
    control_tx: Option<mpsc::SyncSender<Vec<u8>>>,
    control_error_rx: mpsc::Receiver<std::io::Error>,
    control_handle: Option<thread::JoinHandle<std::io::Result<()>>>,
    frame_rx: mpsc::Receiver<ListenerFrame>,
    stdout_handle: Option<thread::JoinHandle<()>>,
    stderr_handle: Option<thread::JoinHandle<std::io::Result<Vec<u8>>>>,
    state: ListenerSessionState,
    last_activity_ms: i64,
    heartbeat_interval_ms: i64,
    shutdown_grace: Duration,
    staged_sequence: u64,
    cancellation: HostCancellation,
    activation: ResolvedTriggerActivation,
    terminal: bool,
}

impl std::fmt::Debug for ProcessTriggerSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProcessTriggerSession")
            .field("trigger_id", &self.trigger_id)
            .field("plugin_id", &self.plugin_id)
            .field("identity", &self.identity)
            .field("state", &self.state)
            .field("terminal", &self.terminal)
            .finish()
    }
}

impl ProcessTriggerSession {
    pub(crate) fn start(
        state_store: &mut dyn TriggerStateStore,
        definition: &TriggerDefinition,
        manifest: &PluginManifest,
        policy: &TriggerPluginHostPolicy,
        identity: String,
        cancellation: HostCancellation,
    ) -> Result<Self, ContractError> {
        let plugin = validate_trigger_plugin_manifest(manifest, policy)?;
        if manifest
            .trigger_runtime
            .as_ref()
            .and_then(|runtime| runtime.lifecycle)
            != Some(TriggerRuntimeLifecycle::ProcessDaemonSession)
        {
            return Err(ContractError::NodePluginInvalidField {
                plugin_id: manifest.plugin_id.clone(),
                field: "plugin.trigger_runtime.lifecycle",
                detail: "managed process session requires process_daemon_session".to_owned(),
            });
        }
        let executable_path = match plugin.runtime {
            ExternalTriggerPluginRuntime::Process { executable_path } => executable_path,
            ExternalTriggerPluginRuntime::WasmPersistentSession => {
                return Err(ContractError::NodePluginInvalidField {
                    plugin_id: manifest.plugin_id.clone(),
                    field: "plugin.trigger_runtime.lifecycle",
                    detail: "managed process session cannot use wasm runtime".to_owned(),
                });
            }
        };
        validate_existing_executable(&plugin.plugin_id, &executable_path)?;
        let activation = resolve_trigger_activation(manifest, definition, policy)?;
        let input = TriggerHostMessage::Start(TriggerStartCommand {
            protocol_version: String::from("2.0.0"),
            trigger_id: definition.trigger_id.clone(),
            source: definition.source.clone(),
            params: definition.params.clone(),
            resume_checkpoint: state_store
                .read_trigger_checkpoint_for_acceptance(&definition.trigger_id)
                .map_err(progress_to_contract_error(definition))?
                .map(|record| record.checkpoint),
            activation: activation.activation.clone(),
            heartbeat_interval_ms: 5_000,
            shutdown_grace_ms: 10_000,
        });
        input.validate()?;
        let encoded_input = serde_json::to_vec(&input).map_err(|source| {
            ContractError::TriggerPluginProtocolEncode {
                plugin_id: plugin.plugin_id.clone(),
                source,
            }
        })?;

        let mut command = Command::new(&executable_path);
        command
            .arg("--trigger-id")
            .arg(&definition.trigger_id)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        configure_plugin_subprocess_environment(&mut command);
        configure_process_group(&mut command);
        let mut child = command.spawn().map_err(|source| ContractError::TriggerPluginSpawnFailed {
            plugin_id: plugin.plugin_id.clone(),
            path: executable_path,
            source,
        })?;

        let start_result = (|| {
            let stdin = child.stdin.take().ok_or_else(|| ContractError::TriggerPluginProcessIo {
                plugin_id: plugin.plugin_id.clone(),
                operation: "capture managed stdin",
                source: std::io::Error::other("missing stdin pipe"),
            })?;
            let stdout = child.stdout.take().ok_or_else(|| ContractError::TriggerPluginProcessIo {
                plugin_id: plugin.plugin_id.clone(),
                operation: "capture managed stdout",
                source: std::io::Error::other("missing stdout pipe"),
            })?;
            let stderr = child.stderr.take().ok_or_else(|| ContractError::TriggerPluginProcessIo {
                plugin_id: plugin.plugin_id.clone(),
                operation: "capture managed stderr",
                source: std::io::Error::other("missing stderr pipe"),
            })?;
            Ok::<_, ContractError>((stdin, stdout, stderr))
        })();
        let (mut stdin, stdout, stderr) = match start_result {
            Ok(pipes) => pipes,
            Err(error) => {
                terminate_child_group(&mut child, Duration::ZERO);
                let _ = child.wait();
                return Err(error);
            }
        };
        if let Err(source) = stdin
            .write_all(&encoded_input)
            .and_then(|_| stdin.write_all(b"\n"))
            .and_then(|_| stdin.flush())
        {
            terminate_child_group(&mut child, Duration::ZERO);
            let _ = child.wait();
            return Err(ContractError::TriggerPluginProcessIo {
                plugin_id: plugin.plugin_id,
                operation: "write managed start command",
                source,
            });
        }

        let (control_tx, control_error_rx, control_handle) = spawn_control_writer(stdin);
        let (frame_tx, frame_rx) = mpsc::sync_channel(MAX_PENDING_TRIGGER_FRAMES);
        let stdout_handle = thread::spawn(move || read_managed_stdout(stdout, frame_tx));
        let stderr_handle = thread::spawn(move || read_stderr_tail(stderr));
        let heartbeat_interval_ms = input_heartbeat_interval_ms(&input);
        let last_activity_ms = match contract_now_ms(definition) {
            Ok(now_ms) => now_ms,
            Err(error) => {
                terminate_child_group(&mut child, Duration::ZERO);
                let _ = child.wait();
                let _ = stdout_handle.join();
                let _ = stderr_handle.join();
                return Err(error);
            }
        };
        Ok(Self {
            trigger_id: definition.trigger_id.clone(),
            plugin_id: manifest.plugin_id.clone(),
            identity,
            child,
            control_tx: Some(control_tx),
            control_error_rx,
            control_handle: Some(control_handle),
            frame_rx,
            stdout_handle: Some(stdout_handle),
            stderr_handle: Some(stderr_handle),
            state: ListenerSessionState::WaitingReady,
            last_activity_ms,
            heartbeat_interval_ms,
            shutdown_grace: Duration::from_secs(10),
            staged_sequence: 0,
            cancellation,
            activation,
            terminal: false,
        })
    }

    pub(crate) fn identity(&self) -> &str {
        &self.identity
    }

    pub(crate) fn is_active(&self) -> bool {
        !self.terminal
    }

    pub(crate) fn drain(
        &mut self,
        state_store: &mut dyn TriggerStateStore,
        definition: &TriggerDefinition,
        now_ms: i64,
    ) -> Result<usize, ContractError> {
        if self.cancellation.is_cancelled() {
            self.shutdown("lease_lost")?;
            return Err(ContractError::TriggerPluginProtocolContractViolation {
                plugin_id: self.plugin_id.clone(),
                detail: "managed listener cancelled after lease loss".to_owned(),
            });
        }

        let mut staged = 0usize;
        let drain_deadline = Instant::now() + MANAGED_DRAIN_TIME_BUDGET;
        let mut frames_processed = 0usize;
        loop {
            if frames_processed >= MANAGED_DRAIN_FRAME_BUDGET || Instant::now() >= drain_deadline {
                break;
            }
            if let Some(source) = self.take_control_error() {
                return Err(self.control_error(source));
            }
            match self.frame_rx.try_recv() {
                Ok(ListenerFrame::Bytes(frame)) => {
                    frames_processed = frames_processed.saturating_add(1);
                    if frame.iter().all(u8::is_ascii_whitespace) {
                        continue;
                    }
                    let message: TriggerPluginMessage = serde_json::from_slice(&frame).map_err(|source| {
                        ContractError::TriggerPluginOutputDecode {
                            plugin_id: self.plugin_id.clone(),
                            source,
                        }
                    })?;
                    self.state = self.state.handle_message(&self.plugin_id, &message)?;
                    self.last_activity_ms = now_ms;
                    match message {
                        TriggerPluginMessage::Ready(_) | TriggerPluginMessage::Heartbeat(_) => {}
                        TriggerPluginMessage::Fatal(fatal) => {
                            self.shutdown("plugin_fatal")?;
                            return Err(ContractError::TriggerPluginReturnedFailure {
                                plugin_id: self.plugin_id.clone(),
                                message: redact_text(&fatal.message, &self.activation.resolved_values),
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
                                    self.staged_sequence
                                ),
                                trigger_id: definition.trigger_id.clone(),
                                workflow_id: definition.workflow_id.clone(),
                                event_id: format!("{}:{}", definition.trigger_id, event.event_key),
                                source: definition.source.clone(),
                                occurred_at_ms: event.occurred_at_ms,
                                staged_at_ms: now_ms,
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
                                .append_staged_trigger_event_record_for_acceptance(&staged_record)
                                .map_err(progress_to_contract_error(definition))?;
                            self.staged_sequence = self.staged_sequence.saturating_add(1);
                            self.write_message(&TriggerHostMessage::Ack(TriggerAck {
                                checkpoint: event.checkpoint,
                            }))?;
                            staged = staged.saturating_add(1);
                        }
                    }
                }
                Ok(ListenerFrame::FrameTooLarge) => {
                    self.shutdown("frame_limit_exceeded")?;
                    return Err(ContractError::TriggerPluginProtocolContractViolation {
                        plugin_id: self.plugin_id.clone(),
                        detail: format!("trigger frame exceeds {MAX_TRIGGER_FRAME_BYTES} bytes"),
                    });
                }
                Ok(ListenerFrame::StdoutClosed) => {
                    self.terminal = true;
                    self.state = ListenerSessionState::Stopped;
                    return Err(ContractError::TriggerPluginProtocolContractViolation {
                        plugin_id: self.plugin_id.clone(),
                        detail: "managed listener stdout closed".to_owned(),
                    });
                }
                Ok(ListenerFrame::StdoutError(source)) => {
                    self.terminal = true;
                    return Err(ContractError::TriggerPluginProcessIo {
                        plugin_id: self.plugin_id.clone(),
                        operation: "read managed stdout",
                        source,
                    });
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.terminal = true;
                    break;
                }
            }
        }

        if heartbeat_timed_out(
            self.state,
            self.last_activity_ms,
            now_ms,
            self.heartbeat_interval_ms,
        ) {
            self.shutdown("heartbeat_timeout")?;
            return Err(ContractError::TriggerPluginProtocolContractViolation {
                plugin_id: self.plugin_id.clone(),
                detail: "heartbeat timed out after ready".to_owned(),
            });
        }
        Ok(staged)
    }

    pub(crate) fn shutdown(&mut self, reason: &str) -> Result<(), ContractError> {
        if self.terminal
            && self.stdout_handle.is_none()
            && self.stderr_handle.is_none()
            && self.control_handle.is_none()
        {
            return Ok(());
        }
        let stop_error = self
            .write_message(&TriggerHostMessage::Stop(TriggerStop {
                reason: reason.to_owned(),
            }))
            .err();
        let deadline = Instant::now() + self.shutdown_grace;
        while Instant::now() < deadline {
            while self.frame_rx.try_recv().is_ok() {}
            match self.child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) => thread::sleep(Duration::from_millis(25)),
                Err(source) => {
                    terminate_child_group(&mut self.child, Duration::ZERO);
                    let _ = self.child.wait();
                    self.control_tx.take();
                    let _ = self.control_handle.take().map(|handle| handle.join());
                    let _ = self.stdout_handle.take().map(|handle| handle.join());
                    let _ = self.stderr_handle.take().map(|handle| handle.join());
                    return Err(ContractError::TriggerPluginProcessIo {
                        plugin_id: self.plugin_id.clone(),
                        operation: "wait for managed listener shutdown",
                        source,
                    });
                }
            }
        }
        if self.child.try_wait().ok().flatten().is_none() {
            terminate_child_group(&mut self.child, Duration::ZERO);
        }
        let _ = self.child.wait();
        if let Some(handle) = self.stdout_handle.as_ref() {
            while !handle.is_finished() {
                while self.frame_rx.try_recv().is_ok() {}
                thread::sleep(Duration::from_millis(1));
            }
        }
        while self.frame_rx.try_recv().is_ok() {}
        self.control_tx.take();
        if let Some(handle) = self.control_handle.take() {
            let writer_result = handle.join().map_err(|_| {
                ContractError::TriggerPluginProcessIo {
                    plugin_id: self.plugin_id.clone(),
                    operation: "join managed stdin writer",
                    source: std::io::Error::other("stdin writer panicked"),
                }
            })?;
            if let Err(source) = writer_result
                && stop_error.is_none()
            {
                return Err(self.control_error(source));
            }
        }
        if let Some(handle) = self.stdout_handle.take() {
            handle.join().map_err(|_| ContractError::TriggerPluginProcessIo {
                plugin_id: self.plugin_id.clone(),
                operation: "join managed stdout reader",
                source: std::io::Error::other("stdout reader panicked"),
            })?;
        }
        if let Some(handle) = self.stderr_handle.take() {
            let _stderr = handle
                .join()
                .map_err(|_| ContractError::TriggerPluginProcessIo {
                    plugin_id: self.plugin_id.clone(),
                    operation: "join managed stderr reader",
                    source: std::io::Error::other("stderr reader panicked"),
                })?
                .map_err(|source| ContractError::TriggerPluginProcessIo {
                    plugin_id: self.plugin_id.clone(),
                    operation: "read managed stderr",
                    source,
                })?;
        }
        self.terminal = true;
        self.state = ListenerSessionState::Stopped;
        if let Some(error) = stop_error {
            return Err(error);
        }
        if let Ok(source) = self.control_error_rx.try_recv() {
            return Err(self.control_error(source));
        }
        Ok(())
    }

    fn take_control_error(&self) -> Option<std::io::Error> {
        self.control_error_rx.try_recv().ok()
    }

    fn control_error(&self, source: std::io::Error) -> ContractError {
        ContractError::TriggerPluginProcessIo {
            plugin_id: self.plugin_id.clone(),
            operation: "write managed listener control message",
            source,
        }
    }

    fn write_message(&mut self, message: &TriggerHostMessage) -> Result<(), ContractError> {
        if let Some(source) = self.take_control_error() {
            return Err(self.control_error(source));
        }
        let mut encoded = serde_json::to_vec(message).map_err(|source| {
            ContractError::TriggerPluginProtocolEncode {
                plugin_id: self.plugin_id.clone(),
                source,
            }
        })?;
        encoded.push(b'\n');
        let sender = self.control_tx.as_ref().ok_or_else(|| self.control_error(
            std::io::Error::new(std::io::ErrorKind::BrokenPipe, "managed stdin writer is closed"),
        ))?;
        sender.try_send(encoded).map_err(|error| {
            let source = match error {
                mpsc::TrySendError::Full(_) => std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "managed stdin writer queue is full",
                ),
                mpsc::TrySendError::Disconnected(_) => std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "managed stdin writer has stopped",
                ),
            };
            self.control_error(source)
        })
    }
}

impl Drop for ProcessTriggerSession {
    fn drop(&mut self) {
        let _ = self.shutdown("supervisor_drop");
    }
}

fn spawn_control_writer(
    stdin: ChildStdin,
) -> (
    mpsc::SyncSender<Vec<u8>>,
    mpsc::Receiver<std::io::Error>,
    thread::JoinHandle<std::io::Result<()>>,
) {
    let (control_tx, control_rx): (
        mpsc::SyncSender<Vec<u8>>,
        mpsc::Receiver<Vec<u8>>,
    ) = mpsc::sync_channel(MANAGED_CONTROL_QUEUE_CAPACITY);
    let (error_tx, error_rx) = mpsc::channel();
    let control_handle = thread::spawn(move || {
        if let Err(source) = set_control_stdin_nonblocking(&stdin) {
            let _ = error_tx.send(std::io::Error::new(source.kind(), source.to_string()));
            return Err(source);
        }
        let mut stdin = stdin;
        while let Ok(message) = control_rx.recv() {
            if let Err(source) = write_control_message(&mut stdin, &message) {
                let _ = error_tx.send(std::io::Error::new(source.kind(), source.to_string()));
                return Err(source);
            }
        }
        Ok(())
    });
    (control_tx, error_rx, control_handle)
}

fn write_control_message(stdin: &mut ChildStdin, message: &[u8]) -> std::io::Result<()> {
    let deadline = Instant::now() + MANAGED_CONTROL_WRITE_TIMEOUT;
    let mut offset = 0usize;
    while offset < message.len() {
        match stdin.write(&message[offset..]) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WriteZero,
                    "managed stdin closed while writing",
                ));
            }
            Ok(written) => offset = offset.saturating_add(written),
            Err(source) if source.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "managed stdin write exceeded deadline",
                    ));
                }
                thread::sleep(Duration::from_millis(1));
            }
            Err(source) => return Err(source),
        }
    }
    stdin.flush()

}

#[cfg(unix)]
fn set_control_stdin_nonblocking(stdin: &ChildStdin) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;

    let fd = stdin.as_raw_fd();
    // SAFETY: fcntl only reads and updates flags for this owned pipe descriptor.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags == -1 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: fcntl updates flags for the same valid pipe descriptor.
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(unix))]
fn set_control_stdin_nonblocking(_: &ChildStdin) -> std::io::Result<()> {
    Ok(())
}

fn cleanup_short_lived_process(
    plugin_id: &str,
    child: &mut Child,
    stdin: &mut ChildStdin,
    stdout_handle: &mut Option<thread::JoinHandle<()>>,
    stderr_handle: &mut Option<thread::JoinHandle<std::io::Result<String>>>,
) {
    let _ = request_plugin_stop(
        plugin_id,
        stdin,
        TriggerStop {
            reason: String::from("host_error"),
        },
    );
    terminate_child_group(child, Duration::from_secs(3));
    let _ = child.wait();
    let _ = stdout_handle.take().map(|handle| handle.join());
    let _ = stderr_handle.take().map(|handle| handle.join());
}

fn read_managed_stdout(stdout: std::process::ChildStdout, frame_tx: mpsc::SyncSender<ListenerFrame>) {
    let mut reader = BufReader::new(stdout);
    loop {
        match read_bounded_frame(&mut reader) {
            Ok(Some(frame)) => {
                if frame_tx.send(ListenerFrame::Bytes(frame)).is_err() {
                    return;
                }
            }
            Ok(None) => {
                let _ = frame_tx.send(ListenerFrame::StdoutClosed);
                return;
            }
            Err(source) if source.kind() == std::io::ErrorKind::InvalidData => {
                let _ = frame_tx.send(ListenerFrame::FrameTooLarge);
                return;
            }
            Err(source) => {
                let _ = frame_tx.send(ListenerFrame::StdoutError(source));
                return;
            }
        }
    }
}

fn read_bounded_frame(
    reader: &mut impl BufRead,
) -> std::io::Result<Option<Vec<u8>>> {
    let mut frame = Vec::new();
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return if frame.is_empty() { Ok(None) } else { Ok(Some(frame)) };
        }
        let newline = available.iter().position(|byte| *byte == b'\n');
        let take = newline.unwrap_or(available.len());
        if frame.len().saturating_add(take) > MAX_TRIGGER_FRAME_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "managed trigger frame exceeded limit",
            ));
        }
        frame.extend_from_slice(&available[..take]);
        reader.consume(take.saturating_add(usize::from(newline.is_some())));
        if newline.is_some() {
            return Ok(Some(frame));
        }
    }
}

fn read_stderr_tail(stderr: std::process::ChildStderr) -> std::io::Result<Vec<u8>> {
    let mut reader = BufReader::new(stderr);
    let mut tail = Vec::new();
    let mut chunk = [0_u8; 8192];
    loop {
        let read = reader.read(&mut chunk)?;
        if read == 0 {
            return Ok(tail);
        }
        tail.extend_from_slice(&chunk[..read]);
        if tail.len() > MAX_TRIGGER_STDERR_BYTES {
            let excess = tail.len().saturating_sub(MAX_TRIGGER_STDERR_BYTES);
            tail.drain(..excess);
        }
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
    manifest: &PluginManifest,
    definition: &TriggerDefinition,
    policy: &TriggerPluginHostPolicy,
) -> Result<ResolvedTriggerActivation, ContractError> {
    let Some(plugin_id) = definition.plugin.as_deref() else {
        return Ok(ResolvedTriggerActivation::default());
    };
    let Some(bindings) = policy.plugin_activation.get(plugin_id) else {
        enforce_required_trigger_activation(manifest, definition, plugin_id)?;
        return Ok(ResolvedTriggerActivation::default());
    };
    if bindings.secret_bindings.is_empty() && bindings.allowed_origins.is_empty() {
        enforce_required_trigger_activation(manifest, definition, plugin_id)?;
        return Ok(ResolvedTriggerActivation::default());
    }

    let mut secrets = Vec::with_capacity(bindings.secret_bindings.len());
    let mut resolved = std::collections::BTreeMap::new();
    if std::env::var("CHAINBOT_SECRET_DECRYPTOR").ok().as_deref() == Some("plaintext") {
        let provider =
            SecretProvider::new(policy.secrets_root_dir.clone(), PlaintextSecretDecryptor);
        for (slot, reference) in &bindings.secret_bindings {
            let value = provider.resolve_reference(reference)?;
            resolved.insert(slot.clone(), value.expose().to_owned());
            secrets.push(value);
        }
    } else {
        let provider =
            SecretProvider::new(policy.secrets_root_dir.clone(), GpgSecretDecryptor::new());
        for (slot, reference) in &bindings.secret_bindings {
            let value = provider.resolve_reference(reference)?;
            resolved.insert(slot.clone(), value.expose().to_owned());
            secrets.push(value);
        }
    }

    Ok(ResolvedTriggerActivation {
        activation: Some(PluginActivationEnvelope {
            secrets: resolved,
            allowed_origins: bindings.allowed_origins.clone(),
        }),
        resolved_values: secrets,
    })
}

fn enforce_required_trigger_activation(
    manifest: &PluginManifest,
    definition: &TriggerDefinition,
    plugin_id: &str,
) -> Result<(), ContractError> {
    let Some(contract) = manifest.activation.as_ref() else {
        return Ok(());
    };
    let mut requirements = Vec::new();
    if !contract.required_secret_slots.is_empty() {
        requirements.push(format!(
            "secret_bindings must provide required slots: {}",
            contract.required_secret_slots.join(", ")
        ));
    }
    if contract.requires_allowed_origins {
        requirements.push("allowed_origins must be configured".to_owned());
    }
    if requirements.is_empty() {
        return Ok(());
    }
    Err(ContractError::InvalidTriggerDefinitionField {
        trigger_id: definition.trigger_id.clone(),
        field: "root_config.plugin_activation",
        detail: format!(
            "plugin `{plugin_id}` requires plugin_activation because {}",
            requirements.join("; ")
        ),
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

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::io::Cursor;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;
    use crate::infrastructure::config::{
        RuntimeStorageBackend, RuntimeStorageConfig,
    };
    use crate::infrastructure::state::RuntimeStateStore;

    #[cfg(unix)]
    #[test]
    fn managed_session_stages_before_ack_and_stays_owned_until_shutdown() {
        use std::os::unix::fs::PermissionsExt;

        let root = unique_test_root("managed-process-session");
        let plugin_root = root.join("plugins").join("managed-trigger");
        let executable = plugin_root.join("bin").join("listener.sh");
        let ack_path = root.join("ack.json");
        std::fs::create_dir_all(executable.parent().expect("executable should have parent"))
            .expect("plugin bin directory should be creatable");
        std::fs::create_dir_all(root.join("secrets"))
            .expect("secrets directory should be creatable");
        std::fs::write(
            &executable,
            format!(
                "#!/bin/sh\nIFS= read -r start\nprintf '%s\\n' '{{\"type\":\"ready\",\"protocol_version\":\"2.0.0\"}}'\nprintf '%s\\n' '{{\"type\":\"event\",\"checkpoint\":\"cp-managed\",\"event_key\":\"evt-managed\",\"occurred_at_ms\":1711200000000,\"payload\":{{\"ok\":true}}}}'\nIFS= read -r ack\nprintf '%s' \"$ack\" > '{}'\nwhile IFS= read -r control; do case \"$control\" in *'\"type\":\"stop\"'*) exit 0;; esac; done\n",
                ack_path.display()
            ),
        )
        .expect("managed listener fixture should be writable");
        let mut permissions = std::fs::metadata(&executable)
            .expect("fixture metadata should exist")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&executable, permissions)
            .expect("fixture should be executable");

        let sqlite_path = root.join("runtime.sqlite3");
        let storage_config = RuntimeStorageConfig {
            backend: RuntimeStorageBackend::Local {
                database_path: sqlite_path,
            },
            history_retention: None,
            raw_debug_enabled: false,
            raw_debug_artifacts_dir: None,
        };
        let now_ms = contract_now_ms(&managed_definition())
            .expect("managed test clock should be available");
        let mut state_store = RuntimeStateStore::open(&storage_config, now_ms)
            .expect("managed test store should open");
        let definition = managed_definition();
        let manifest = managed_manifest(&plugin_root);
        let policy = TriggerPluginHostPolicy {
            allowlisted_plugin_ids: BTreeSet::from([String::from("managed-trigger")]),
            allowed_capabilities: BTreeSet::from([String::from(
                REQUIRED_TRIGGER_PLUGIN_CAPABILITY,
            )]),
            plugin_root_dir: root.join("plugins"),
            plugin_activation: BTreeMap::new(),
            secrets_root_dir: root.join("secrets"),
        };
        let mut session = ProcessTriggerSession::start(
            &mut state_store,
            &definition,
            &manifest,
            &policy,
            String::from("identity-v1"),
            HostCancellation::default(),
        )
        .expect("managed process session should start");

        let deadline = Instant::now() + Duration::from_secs(2);
        let staged = loop {
            let staged = session
                .drain(&mut state_store, &definition, now_ms)
                .expect("managed frames should drain");
            if staged > 0 {
                break staged;
            }
            assert!(Instant::now() < deadline, "managed event should arrive");
            thread::sleep(Duration::from_millis(10));
        };

        assert_eq!(staged, 1);
        assert_eq!(
            state_store
                .list_pending_staged_trigger_event_records("tr-managed", 10)
                .expect("staged rows should be queryable")
                .len(),
            1
        );
        while !std::fs::read_to_string(&ack_path)
            .ok()
            .is_some_and(|ack| ack.contains("cp-managed"))
        {
            assert!(Instant::now() < deadline, "durable ACK should reach listener");
            thread::sleep(Duration::from_millis(10));
        }
        assert!(session.is_active());
        session
            .shutdown("test_complete")
            .expect("managed session should stop and join readers");
        assert!(!session.is_active());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn managed_frame_limit_is_enforced_before_json_decode() {
        let mut bytes = vec![b'x'; MAX_TRIGGER_FRAME_BYTES + 1];
        bytes.push(b'\n');
        let mut reader = BufReader::new(Cursor::new(bytes));

        let error = read_bounded_frame(&mut reader)
            .expect_err("oversized frame should be rejected while reading");

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn managed_frame_reader_preserves_following_frames() {
        let mut reader = BufReader::new(Cursor::new(b"first\nsecond\n"));

        assert_eq!(
            read_bounded_frame(&mut reader).expect("first frame should read"),
            Some(b"first".to_vec())
        );
        assert_eq!(
            read_bounded_frame(&mut reader).expect("second frame should read"),
            Some(b"second".to_vec())
        );
    }

    fn managed_definition() -> TriggerDefinition {
        TriggerDefinition {
            api_version: String::from("2.0.0"),
            trigger_id: String::from("tr-managed"),
            kind: String::from("external_plugin"),
            source: String::from("managed.source"),
            plugin: Some(String::from("managed-trigger")),
            workflow_id: String::from("wf-managed"),
            enabled: true,
            params: BTreeMap::new(),
            input_mapping: BTreeMap::new(),
            package_root: PathBuf::new(),
        }
    }

    fn managed_manifest(plugin_root: &Path) -> PluginManifest {
        let mut manifest: PluginManifest = serde_json::from_value(serde_json::json!({
            "manifest_version": "2.0.0",
            "plugin_id": "managed-trigger",
            "kind": "external_trigger",
            "entrypoint": "trigger.exec.v1",
            "capabilities": ["trigger.listen.event"],
            "executable": "bin/listener.sh",
            "trigger_runtime": {
                "lifecycle": "process_daemon_session",
                "push_callback": "inline_response",
                "durable_ack": "after_store_persist",
                "host_error_categories": ["transport", "protocol_contract", "plugin_fatal"]
            },
            "event_schema": {
                "fields": ["ok"]
            }
        }))
        .expect("managed manifest should deserialize");
        manifest.manifest_path = plugin_root.join("config.toml");
        manifest
    }

    fn unique_test_root(prefix: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after UNIX epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("chainbot-{prefix}-{nonce}"))
    }
}
