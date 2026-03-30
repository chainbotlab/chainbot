//! [INPUT]
//! Materialized source repositories, discovered plugin packages, and source install metadata.
//!
//! [OUTPUT]
//! Produces a prepared plugin package tree with validated artifacts ready for transactional install.
//!
//! [ROLE]
//! Owns package preparation, optional build execution, and artifact validation for plugin installs.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::errors::ContractError;
use crate::plugin::configure_plugin_subprocess_environment;

use super::discover::DiscoveredPlugin;
use super::fs::{copy_tree_strict, create_temp_dir, read_shebang};
use super::manifest::{SourceInstallMode, SourceRuntime};
use super::transport::MaterializedSource;

const SOURCE_BUILD_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug)]
pub(crate) struct PreparedPlugin {
    pub(crate) plugin_id: String,
    pub(crate) package_root: PathBuf,
}

pub(crate) fn prepare_installable_plugin(
    source: &MaterializedSource,
    plugin: &DiscoveredPlugin,
) -> Result<PreparedPlugin, ContractError> {
    let temp_root = create_temp_dir("plugin-prepare")?;
    let workspace_root = temp_root.join("workspace");
    copy_tree_strict(&source.repo_root, &workspace_root)?;
    let package_root = if plugin.path == "." {
        workspace_root.clone()
    } else {
        workspace_root.join(&plugin.path)
    };
    match plugin.source_manifest.install_mode {
        SourceInstallMode::Direct => {}
        SourceInstallMode::BuildRequired => {
            run_build(&workspace_root, &package_root, plugin)?;
        }
    }
    validate_artifact(&package_root, plugin)?;
    Ok(PreparedPlugin {
        plugin_id: plugin.plugin_id.clone(),
        package_root,
    })
}

fn run_build(
    workspace_root: &Path,
    package_root: &Path,
    plugin: &DiscoveredPlugin,
) -> Result<(), ContractError> {
    let build = plugin
        .source_manifest
        .build
        .as_ref()
        .ok_or_else(|| ContractError::CliUsage {
            message: format!("plugin {} is missing source.build", plugin.plugin_id),
        })?;
    let workdir = plugin
        .source_manifest
        .build_workdir_path(package_root, workspace_root)?
        .ok_or_else(|| ContractError::CliUsage {
            message: format!(
                "plugin {} build workdir could not be resolved",
                plugin.plugin_id
            ),
        })?;
    let mut command = Command::new(&build.command[0]);
    if build.command.len() > 1 {
        command.args(&build.command[1..]);
    }
    command.current_dir(&workdir);
    configure_plugin_subprocess_environment(&mut command);
    let output =
        run_build_command_with_timeout(&mut command, source_build_timeout()).map_err(|source| {
            ContractError::CliUsage {
                message: format!(
                    "failed to start build command for plugin {}: {source}",
                    plugin.plugin_id
                ),
            }
        })?;
    if !output.status.success() {
        return Err(ContractError::CliUsage {
            message: format!(
                "build command failed for plugin {}: {}",
                plugin.plugin_id,
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        });
    }
    for (from, to) in plugin
        .source_manifest
        .output_paths(package_root, workspace_root)?
    {
        let metadata = fs::metadata(&from).map_err(|source| ContractError::CliUsage {
            message: format!(
                "declared build output missing for plugin {} at {}: {source}",
                plugin.plugin_id,
                from.display()
            ),
        })?;
        if !metadata.is_file() {
            return Err(ContractError::CliUsage {
                message: format!(
                    "declared build output is not a file for plugin {}: {}",
                    plugin.plugin_id,
                    from.display()
                ),
            });
        }
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent).map_err(|source| ContractError::Io {
                path: parent.to_path_buf(),
                operation: "create build output parent directory",
                source,
            })?;
        }
        fs::copy(&from, &to).map_err(|source| ContractError::Io {
            path: to.clone(),
            operation: "copy build output",
            source,
        })?;
        fs::set_permissions(&to, metadata.permissions()).map_err(|source| ContractError::Io {
            path: to,
            operation: "set build output permissions",
            source,
        })?;
    }
    Ok(())
}

fn validate_artifact(package_root: &Path, plugin: &DiscoveredPlugin) -> Result<(), ContractError> {
    let artifact_path = plugin.source_manifest.entry_artifact_path(package_root)?;
    let metadata = fs::metadata(&artifact_path).map_err(|source| ContractError::CliUsage {
        message: format!(
            "entry artifact missing for plugin {} at {}: {source}",
            plugin.plugin_id,
            artifact_path.display()
        ),
    })?;
    if !metadata.is_file() {
        return Err(ContractError::CliUsage {
            message: format!(
                "entry artifact must be a file for plugin {}: {}",
                plugin.plugin_id,
                artifact_path.display()
            ),
        });
    }
    if plugin.manifest.is_streamable_http_mcp_entrypoint() {
        return Ok(());
    }
    match plugin.source_manifest.runtime {
        SourceRuntime::Python => {
            validate_script_artifact(&artifact_path, "python", &plugin.plugin_id)?
        }
        SourceRuntime::Node => validate_script_artifact(&artifact_path, "node", &plugin.plugin_id)?,
        SourceRuntime::Bin => validate_executable_artifact(&artifact_path, &plugin.plugin_id)?,
        SourceRuntime::Wasm => {}
    }
    Ok(())
}

fn validate_script_artifact(
    path: &Path,
    expected_runtime: &str,
    plugin_id: &str,
) -> Result<(), ContractError> {
    let shebang = read_shebang(path)?.ok_or_else(|| ContractError::CliUsage {
        message: format!(
            "plugin {plugin_id} entry artifact is missing a shebang: {}",
            path.display()
        ),
    })?;
    if !shebang.contains(expected_runtime) {
        return Err(ContractError::CliUsage {
            message: format!(
                "plugin {plugin_id} entry artifact shebang must reference {expected_runtime}: {shebang}"
            ),
        });
    }
    validate_executable_artifact(path, plugin_id)
}

fn validate_executable_artifact(path: &Path, plugin_id: &str) -> Result<(), ContractError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(path)
            .map_err(|source| ContractError::Io {
                path: path.to_path_buf(),
                operation: "inspect entry artifact permissions",
                source,
            })?
            .permissions()
            .mode();
        if mode & 0o111 == 0 {
            return Err(ContractError::CliUsage {
                message: format!(
                    "plugin {plugin_id} entry artifact is not executable: {}",
                    path.display()
                ),
            });
        }
    }
    Ok(())
}

fn run_build_command_with_timeout(
    command: &mut Command,
    timeout: Duration,
) -> Result<std::process::Output, std::io::Error> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    configure_command_process_group(command);
    let mut child = command.spawn()?;
    let started_at = Instant::now();

    loop {
        match child.try_wait()? {
            Some(_status) => return child.wait_with_output(),
            None => {
                if started_at.elapsed() >= timeout {
                    terminate_child_process_group(&mut child)?;
                    let _ = child.wait();
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        format!("build command timed out after {timeout:?}"),
                    ));
                }
                thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

fn configure_command_process_group(command: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;

        unsafe {
            command.pre_exec(|| {
                if libc::setpgid(0, 0) == 0 {
                    Ok(())
                } else {
                    Err(std::io::Error::last_os_error())
                }
            });
        }
    }
}

fn terminate_child_process_group(child: &mut std::process::Child) -> Result<(), std::io::Error> {
    #[cfg(unix)]
    {
        let pid = child.id() as i32;
        let result = unsafe { libc::killpg(pid, libc::SIGKILL) };
        if result == 0 {
            return Ok(());
        }
        let error = std::io::Error::last_os_error();
        if matches!(error.raw_os_error(), Some(libc::ESRCH)) {
            return Ok(());
        }
        if error.kind() != std::io::ErrorKind::InvalidInput {
            return Err(error);
        }
        return Ok(());
    }

    #[cfg(not(unix))]
    {
        match child.kill() {
            Ok(()) => Ok(()),
            Err(source) if source.kind() == std::io::ErrorKind::InvalidInput => Ok(()),
            Err(source) => Err(source),
        }
    }
}

fn source_build_timeout() -> Duration {
    std::env::var("CHAINBOT_SOURCE_BUILD_TIMEOUT_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .map(Duration::from_millis)
        .unwrap_or(SOURCE_BUILD_TIMEOUT)
}
