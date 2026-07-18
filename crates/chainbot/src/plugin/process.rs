//! [INPUT]
//! Plugin subprocess commands, bounded stdin/stdout/stderr streams, execution limits, and cancellation signals.
//!
//! [OUTPUT]
//! Runs plugin child process groups with bounded collection and deadline-aware cleanup.
//!
//! [ROLE]
//! Centralizes the crate-private process lifecycle contract shared by plugin hosts.

use std::io::{self, Read, Write};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

const POLL_INTERVAL: Duration = Duration::from_millis(25);
const NO_STREAM_LIMIT: u8 = 0;
const STDOUT_LIMIT: u8 = 1;
const STDERR_LIMIT: u8 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PluginProcessLimits {
    pub(crate) wall_timeout: Duration,
    pub(crate) shutdown_grace: Duration,
    pub(crate) max_stdout_bytes: usize,
    pub(crate) max_stderr_bytes: usize,
    pub(crate) max_frame_bytes: usize,
}

impl PluginProcessLimits {
    pub(crate) const fn node() -> Self {
        Self {
            wall_timeout: Duration::from_secs(30),
            shutdown_grace: Duration::from_secs(3),
            max_stdout_bytes: 1024 * 1024,
            max_stderr_bytes: 256 * 1024,
            max_frame_bytes: 0,
        }
    }
}

const HOST_RUNNING: u8 = 0;
const HOST_LEASE_LOST: u8 = 1;
const HOST_SHUTTING_DOWN: u8 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HostCancellationReason {
    LeaseLost,
    ShuttingDown,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct HostCancellation(Arc<AtomicU8>);

impl HostCancellation {
    #[cfg(test)]
    pub(crate) fn cancel(&self) {
        self.shutdown();
    }

    pub(crate) fn lease_lost(&self) {
        let _ = self.0.compare_exchange(
            HOST_RUNNING,
            HOST_LEASE_LOST,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }

    pub(crate) fn shutdown(&self) {
        let _ = self.0.compare_exchange(
            HOST_RUNNING,
            HOST_SHUTTING_DOWN,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }

    pub(crate) fn reason(&self) -> Option<HostCancellationReason> {
        match self.0.load(Ordering::Acquire) {
            HOST_LEASE_LOST => Some(HostCancellationReason::LeaseLost),
            HOST_SHUTTING_DOWN => Some(HostCancellationReason::ShuttingDown),
            _ => None,
        }
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.reason().is_some()
    }
}

#[derive(Debug)]
pub(crate) enum PluginProcessFailure {
    TimedOut,
    Cancelled,
    StdoutLimitExceeded,
    StderrLimitExceeded,
    FrameLimitExceeded,
    Spawn(io::Error),
    Io(io::Error),
    Exit { code: Option<i32>, stderr: String },
}

#[derive(Debug)]
pub(crate) struct PluginProcessOutput {
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
    pub(crate) status: ExitStatus,
}

pub(crate) fn run_bounded_process(
    command: &mut Command,
    stdin_payload: &[u8],
    limits: PluginProcessLimits,
    cancellation: &HostCancellation,
) -> Result<PluginProcessOutput, PluginProcessFailure> {
    if limits.max_frame_bytes > 0 && stdin_payload.len() > limits.max_frame_bytes {
        return Err(PluginProcessFailure::FrameLimitExceeded);
    }
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    configure_process_group(command);

    let mut child = command.spawn().map_err(PluginProcessFailure::Spawn)?;
    if let Some(mut stdin) = child.stdin.take()
        && let Err(source) = stdin.write_all(stdin_payload)
    {
        terminate_child_group(&mut child, limits.shutdown_grace);
        let _ = child.wait();
        return Err(PluginProcessFailure::Io(source));
    }

    let stream_limit = Arc::new(AtomicU8::new(NO_STREAM_LIMIT));
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| PluginProcessFailure::Io(io::Error::other("child stdout was not piped")))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| PluginProcessFailure::Io(io::Error::other("child stderr was not piped")))?;
    let stdout_limit = Arc::clone(&stream_limit);
    let stderr_limit = Arc::clone(&stream_limit);
    let stdout_reader = thread::spawn(move || {
        read_bounded(stdout, limits.max_stdout_bytes, STDOUT_LIMIT, stdout_limit)
    });
    let stderr_reader = thread::spawn(move || {
        read_bounded(stderr, limits.max_stderr_bytes, STDERR_LIMIT, stderr_limit)
    });

    let deadline = Instant::now() + limits.wall_timeout;
    let failure = loop {
        match child.try_wait() {
            Ok(Some(_)) => break None,
            Ok(None) if cancellation.is_cancelled() => {
                break Some(PluginProcessFailure::Cancelled)
            }
            Ok(None) if Instant::now() >= deadline => {
                break Some(PluginProcessFailure::TimedOut)
            }
            Ok(None) => match stream_limit.load(Ordering::Acquire) {
                STDOUT_LIMIT => break Some(PluginProcessFailure::StdoutLimitExceeded),
                STDERR_LIMIT => break Some(PluginProcessFailure::StderrLimitExceeded),
                _ => thread::sleep(POLL_INTERVAL),
            },
            Err(source) => break Some(PluginProcessFailure::Io(source)),
        }
    };

    if failure.is_some() {
        terminate_child_group(&mut child, limits.shutdown_grace);
    }
    let status = child.wait().map_err(PluginProcessFailure::Io)?;
    let stdout = join_reader(stdout_reader)?;
    let stderr = join_reader(stderr_reader)?;

    if let Some(failure) = failure {
        return Err(failure);
    }
    if !status.success() {
        return Err(PluginProcessFailure::Exit {
            code: status.code(),
            stderr: String::from_utf8_lossy(&stderr).trim().to_owned(),
        });
    }

    Ok(PluginProcessOutput {
        stdout,
        stderr,
        status,
    })
}

fn read_bounded<R: Read>(
    mut reader: R,
    max_bytes: usize,
    failure_code: u8,
    stream_limit: Arc<AtomicU8>,
) -> io::Result<Vec<u8>> {
    let mut collected = Vec::new();
    let mut chunk = [0_u8; 8192];
    loop {
        let read = reader.read(&mut chunk)?;
        if read == 0 {
            return Ok(collected);
        }
        let remaining = max_bytes.saturating_sub(collected.len());
        if read > remaining {
            collected.extend_from_slice(&chunk[..remaining]);
            stream_limit.compare_exchange(
                NO_STREAM_LIMIT,
                failure_code,
                Ordering::AcqRel,
                Ordering::Acquire,
            ).ok();
            continue;
        }
        collected.extend_from_slice(&chunk[..read]);
    }
}

fn join_reader(
    reader: thread::JoinHandle<io::Result<Vec<u8>>>,
) -> Result<Vec<u8>, PluginProcessFailure> {
    reader
        .join()
        .map_err(|_| PluginProcessFailure::Io(io::Error::other("plugin stream reader panicked")))?
        .map_err(PluginProcessFailure::Io)
}

#[cfg(unix)]
pub(crate) fn configure_process_group(command: &mut Command) {
    use std::os::unix::process::CommandExt;

    // SAFETY: pre_exec runs in the child immediately before exec; setpgid only changes
    // the child process group and uses no Rust state after fork.
    unsafe {
        command.pre_exec(|| {
            if libc::setpgid(0, 0) == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[cfg(not(unix))]
pub(crate) fn configure_process_group(_: &mut Command) {}

#[cfg(unix)]
pub(crate) fn terminate_child_group(child: &mut std::process::Child, grace: Duration) {
    let process_group = -(child.id() as i32);
    // SAFETY: process_group is the negative PID configured by setpgid for this child.
    unsafe {
        libc::kill(process_group, libc::SIGTERM);
    }
    let deadline = Instant::now() + grace;
    while Instant::now() < deadline {
        if child.try_wait().ok().flatten().is_some() {
            return;
        }
        thread::sleep(POLL_INTERVAL);
    }
    // SAFETY: process_group still identifies only the child process group.
    unsafe {
        libc::kill(process_group, libc::SIGKILL);
    }
}

#[cfg(not(unix))]
pub(crate) fn terminate_child_group(child: &mut std::process::Child, _: Duration) {
    let _ = child.kill();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn cancellation_terminates_a_running_process_group() {
        let cancellation = HostCancellation::default();
        let cancellation_for_thread = cancellation.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(50));
            cancellation_for_thread.cancel();
        });

        let mut command = Command::new("sh");
        command.args(["-c", "sleep 5 & wait"]);
        let started_at = Instant::now();
        let result = run_bounded_process(
            &mut command,
            b"",
            PluginProcessLimits {
                wall_timeout: Duration::from_secs(5),
                shutdown_grace: Duration::from_millis(50),
                max_stdout_bytes: 1024,
                max_stderr_bytes: 1024,
                max_frame_bytes: 0,
            },
            &cancellation,
        );

        assert!(matches!(result, Err(PluginProcessFailure::Cancelled)));
        assert!(started_at.elapsed() < Duration::from_secs(1));
    }

    #[cfg(unix)]
    #[test]
    fn stdout_limit_stops_the_process() {
        let mut command = Command::new("sh");
        command.args(["-c", "yes x"]);
        let result = run_bounded_process(
            &mut command,
            b"",
            PluginProcessLimits {
                wall_timeout: Duration::from_secs(5),
                shutdown_grace: Duration::from_millis(50),
                max_stdout_bytes: 32,
                max_stderr_bytes: 1024,
                max_frame_bytes: 0,
            },
            &HostCancellation::default(),
        );

        assert!(matches!(
            result,
            Err(PluginProcessFailure::StdoutLimitExceeded)
        ));
    }
}
