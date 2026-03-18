//! [INPUT]
//! Worker request and response envelopes, subprocess runtime specifications, and host safety limits.
//!
//! [OUTPUT]
//! Defines versioned worker protocol types and executes Python or JavaScript workers with bounded subprocess cleanup.
//!
//! [ROLE]
//! Provides the script-worker protocol and host boundary for external runtime nodes.
//!
//! [INVARIANTS]
//! Timed-out workers are always reaped or reported as explicit failures, and protocol version checks happen before payload handling.


use std::collections::BTreeMap;
use std::fmt::{Display, Formatter};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::errors::{assert_supported_major, ContractError};

pub const CURRENT_PROTOCOL_MAJOR: u64 = 1;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptRuntime {
    Python,
    JavaScript,
}

#[derive(Debug, Clone)]
pub struct WorkerProcessSpec {
    pub runtime: ScriptRuntime,
    pub interpreter_path: PathBuf,
    pub script_path: PathBuf,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkerHostLimits {
    pub timeout: Duration,
    pub max_stdout_bytes: usize,
    pub max_stderr_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerHost {
    limits: WorkerHostLimits,
}

#[derive(Debug)]
pub enum WorkerHostError {
    SerializeRequest(serde_json::Error),
    Spawn {
        runtime: ScriptRuntime,
        interpreter: PathBuf,
        source: std::io::Error,
    },
    MissingPipe {
        stream: &'static str,
    },
    WriteRequest(std::io::Error),
    ReadStream {
        stream: &'static str,
        source: std::io::Error,
    },
    Wait(std::io::Error),
    Kill(std::io::Error),
    CleanupWait(std::io::Error),
    Timeout {
        runtime: ScriptRuntime,
        timeout: Duration,
    },
    OversizedOutput {
        stream: &'static str,
        max_bytes: usize,
        actual_bytes: usize,
    },
    NonZeroExit {
        runtime: ScriptRuntime,
        exit_code: Option<i32>,
        stderr: String,
    },
    EmptyResponse,
    MalformedResponse(serde_json::Error),
    InvalidEnvelope(ContractError),
    ReportedFailure {
        runtime: ScriptRuntime,
        detail: String,
    },
}

#[derive(Debug)]
struct StreamCapture {
    bytes: Vec<u8>,
    total_bytes: usize,
}

impl ScriptRuntime {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Python => "python",
            Self::JavaScript => "javascript",
        }
    }
}

impl WorkerProcessSpec {
    pub fn new(runtime: ScriptRuntime, interpreter_path: PathBuf, script_path: PathBuf) -> Self {
        Self {
            runtime,
            interpreter_path,
            script_path,
            args: Vec::new(),
            env: BTreeMap::new(),
        }
    }

    pub fn with_args(mut self, args: impl IntoIterator<Item = String>) -> Self {
        self.args = args.into_iter().collect();
        self
    }

    pub fn with_env(mut self, env: BTreeMap<String, String>) -> Self {
        self.env = env;
        self
    }
}

impl Default for WorkerHostLimits {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(10),
            max_stdout_bytes: 1024 * 1024,
            max_stderr_bytes: 256 * 1024,
        }
    }
}

impl WorkerHost {
    pub fn new(limits: WorkerHostLimits) -> Self {
        Self { limits }
    }

    pub fn limits(&self) -> WorkerHostLimits {
        self.limits
    }

    pub fn execute(
        &self,
        process: &WorkerProcessSpec,
        request: &WorkerRequestEnvelope,
    ) -> Result<WorkerResponseEnvelope, WorkerHostError> {
        request
            .validate()
            .map_err(WorkerHostError::InvalidEnvelope)?;

        let request_payload =
            serde_json::to_vec(request).map_err(WorkerHostError::SerializeRequest)?;

        let mut command = Command::new(&process.interpreter_path);
        command
            .arg(&process.script_path)
            .args(&process.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env_clear();

        for (key, value) in &process.env {
            command.env(key, value);
        }

        let mut child = command.spawn().map_err(|source| WorkerHostError::Spawn {
            runtime: process.runtime,
            interpreter: process.interpreter_path.clone(),
            source,
        })?;

        let stdin = child
            .stdin
            .take()
            .ok_or(WorkerHostError::MissingPipe { stream: "stdin" })?;
        let stdout = child
            .stdout
            .take()
            .ok_or(WorkerHostError::MissingPipe { stream: "stdout" })?;
        let stderr = child
            .stderr
            .take()
            .ok_or(WorkerHostError::MissingPipe { stream: "stderr" })?;

        let request_writer = thread::spawn(move || -> Result<(), std::io::Error> {
            let mut stdin = stdin;
            stdin.write_all(&request_payload)?;
            stdin.flush()?;
            Ok(())
        });

        let stdout_limit = self.limits.max_stdout_bytes;
        let stdout_reader = thread::spawn(move || read_stream_with_limit(stdout, stdout_limit));

        let stderr_limit = self.limits.max_stderr_bytes;
        let stderr_reader = thread::spawn(move || read_stream_with_limit(stderr, stderr_limit));

        let wait_result =
            wait_for_child_or_timeout(&mut child, self.limits.timeout, process.runtime);

        let request_write_result = request_writer
            .join()
            .map_err(|_| {
                WorkerHostError::WriteRequest(std::io::Error::other("stdin writer panicked"))
            })?
            .map_err(WorkerHostError::WriteRequest);

        let stdout_capture_result = stdout_reader
            .join()
            .map_err(|_| WorkerHostError::ReadStream {
                stream: "stdout",
                source: std::io::Error::other("stdout reader panicked"),
            })?
            .map_err(|source| WorkerHostError::ReadStream {
                stream: "stdout",
                source,
            });

        let stderr_capture_result = stderr_reader
            .join()
            .map_err(|_| WorkerHostError::ReadStream {
                stream: "stderr",
                source: std::io::Error::other("stderr reader panicked"),
            })?
            .map_err(|source| WorkerHostError::ReadStream {
                stream: "stderr",
                source,
            });

        let exit_status = wait_result?;
        request_write_result?;
        let stdout_capture = stdout_capture_result?;
        let stderr_capture = stderr_capture_result?;

        if stdout_capture.total_bytes > self.limits.max_stdout_bytes {
            return Err(WorkerHostError::OversizedOutput {
                stream: "stdout",
                max_bytes: self.limits.max_stdout_bytes,
                actual_bytes: stdout_capture.total_bytes,
            });
        }

        if stderr_capture.total_bytes > self.limits.max_stderr_bytes {
            return Err(WorkerHostError::OversizedOutput {
                stream: "stderr",
                max_bytes: self.limits.max_stderr_bytes,
                actual_bytes: stderr_capture.total_bytes,
            });
        }

        if !exit_status.success() {
            return Err(WorkerHostError::NonZeroExit {
                runtime: process.runtime,
                exit_code: exit_status.code(),
                stderr: format_stream_output(&stderr_capture.bytes),
            });
        }

        if stdout_capture.bytes.is_empty() {
            return Err(WorkerHostError::EmptyResponse);
        }

        let response: WorkerResponseEnvelope = serde_json::from_slice(&stdout_capture.bytes)
            .map_err(WorkerHostError::MalformedResponse)?;
        response
            .validate()
            .map_err(WorkerHostError::InvalidEnvelope)?;

        if response.request_id != request.request_id {
            return Err(WorkerHostError::InvalidEnvelope(
                ContractError::WorkerRequestIdMismatch {
                    expected: request.request_id.clone(),
                    actual: response.request_id,
                },
            ));
        }

        if !response.success {
            return Err(WorkerHostError::ReportedFailure {
                runtime: process.runtime,
                detail: format_worker_failure_detail(&response.output),
            });
        }

        Ok(response)
    }
}

impl WorkerRequestEnvelope {
    pub fn validate(&self) -> Result<(), ContractError> {
        assert_supported_major(
            "worker_request.protocol_version",
            &self.protocol_version,
            CURRENT_PROTOCOL_MAJOR,
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
            CURRENT_PROTOCOL_MAJOR,
        )
    }

    pub fn from_json_str(input: &str) -> Result<Self, ContractError> {
        let envelope: Self = serde_json::from_str(input)?;
        envelope.validate()?;
        Ok(envelope)
    }
}

impl Display for WorkerHostError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SerializeRequest(source) => {
                write!(f, "failed to serialize worker request envelope: {source}")
            }
            Self::Spawn {
                runtime,
                interpreter,
                source,
            } => write!(
                f,
                "failed to spawn {} worker interpreter {}: {source}",
                runtime.as_str(),
                interpreter.display()
            ),
            Self::MissingPipe { stream } => {
                write!(f, "worker child is missing required {stream} pipe")
            }
            Self::WriteRequest(source) => write!(f, "failed to write worker request: {source}"),
            Self::ReadStream { stream, source } => {
                write!(f, "failed to read worker {stream}: {source}")
            }
            Self::Wait(source) => write!(f, "failed while waiting for worker process: {source}"),
            Self::Kill(source) => write!(f, "failed to kill timed-out worker process: {source}"),
            Self::CleanupWait(source) => {
                write!(f, "failed to reap timed-out worker process: {source}")
            }
            Self::Timeout { runtime, timeout } => write!(
                f,
                "{} worker exceeded timeout of {}ms",
                runtime.as_str(),
                timeout.as_millis()
            ),
            Self::OversizedOutput {
                stream,
                max_bytes,
                actual_bytes,
            } => write!(
                f,
                "worker {stream} exceeded limit: {actual_bytes} bytes > {max_bytes} bytes"
            ),
            Self::NonZeroExit {
                runtime,
                exit_code,
                stderr,
            } => {
                let code = exit_code
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "terminated by signal".to_owned());
                if stderr.is_empty() {
                    write!(
                        f,
                        "{} worker exited unsuccessfully with status {code}",
                        runtime.as_str()
                    )
                } else {
                    write!(
                        f,
                        "{} worker exited unsuccessfully with status {}: {}",
                        runtime.as_str(),
                        code,
                        stderr
                    )
                }
            }
            Self::EmptyResponse => {
                write!(
                    f,
                    "worker produced empty stdout; expected JSON response envelope"
                )
            }
            Self::MalformedResponse(source) => write!(
                f,
                "worker stdout did not contain a valid JSON response envelope: {source}"
            ),
            Self::InvalidEnvelope(source) => {
                write!(
                    f,
                    "worker response envelope failed protocol validation: {source}"
                )
            }
            Self::ReportedFailure { runtime, detail } => {
                write!(f, "{} worker reported failure: {detail}", runtime.as_str())
            }
        }
    }
}

impl std::error::Error for WorkerHostError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::SerializeRequest(source) => Some(source),
            Self::Spawn { source, .. } => Some(source),
            Self::WriteRequest(source) => Some(source),
            Self::ReadStream { source, .. } => Some(source),
            Self::Wait(source) => Some(source),
            Self::Kill(source) => Some(source),
            Self::CleanupWait(source) => Some(source),
            Self::MalformedResponse(source) => Some(source),
            Self::InvalidEnvelope(source) => Some(source),
            Self::MissingPipe { .. }
            | Self::Timeout { .. }
            | Self::OversizedOutput { .. }
            | Self::NonZeroExit { .. }
            | Self::EmptyResponse
            | Self::ReportedFailure { .. } => None,
        }
    }
}

fn format_stream_output(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).trim().to_owned()
}

fn format_worker_failure_detail(output: &serde_json::Value) -> String {
    if let Some(message) = output.get("error").and_then(serde_json::Value::as_str) {
        let trimmed = message.trim();
        if !trimmed.is_empty() {
            return trimmed.to_owned();
        }
    }

    if let Some(message) = output.get("message").and_then(serde_json::Value::as_str) {
        let trimmed = message.trim();
        if !trimmed.is_empty() {
            return trimmed.to_owned();
        }
    }

    match serde_json::to_string(output) {
        Ok(value) => format!("success=false with output={value}"),
        Err(_) => "success=false".to_owned(),
    }
}

fn read_stream_with_limit<R>(
    mut reader: R,
    max_bytes: usize,
) -> Result<StreamCapture, std::io::Error>
where
    R: Read,
{
    let mut buffer = [0_u8; 8192];
    let mut captured = Vec::new();
    let mut total_bytes = 0_usize;

    loop {
        let read_bytes = reader.read(&mut buffer)?;
        if read_bytes == 0 {
            break;
        }

        total_bytes = total_bytes.saturating_add(read_bytes);
        if captured.len() < max_bytes {
            let remaining = max_bytes - captured.len();
            let copy_len = remaining.min(read_bytes);
            captured.extend_from_slice(&buffer[..copy_len]);
        }
    }

    Ok(StreamCapture {
        bytes: captured,
        total_bytes,
    })
}

fn wait_for_child_or_timeout(
    child: &mut std::process::Child,
    timeout: Duration,
    runtime: ScriptRuntime,
) -> Result<ExitStatus, WorkerHostError> {
    let started_at = Instant::now();
    loop {
        match child.try_wait().map_err(WorkerHostError::Wait)? {
            Some(status) => return Ok(status),
            None => {
                if started_at.elapsed() >= timeout {
                    match child.kill() {
                        Ok(()) => {}
                        Err(source) if source.kind() == std::io::ErrorKind::InvalidInput => {}
                        Err(source) => return Err(WorkerHostError::Kill(source)),
                    }
                    child.wait().map_err(WorkerHostError::CleanupWait)?;
                    return Err(WorkerHostError::Timeout { runtime, timeout });
                }
                thread::sleep(Duration::from_millis(10));
            }
        }
    }
}
