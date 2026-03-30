//! [INPUT]
//! Plugin source locators, local git or GitHub repository references, and temporary workspace roots.
//!
//! [OUTPUT]
//! Materializes a repository snapshot into a temporary source tree with a stable resolved ref when available.
//!
//! [ROLE]
//! Owns remote source transport fetching and local-repository snapshot rules for plugin source discovery and install.

use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, USER_AGENT};
use zip::ZipArchive;

use crate::errors::ContractError;

use super::fs::copy_tree_strict;
use super::fs::create_temp_dir;
use super::locator::PluginSourceLocator;

const SOURCE_GIT_TIMEOUT: Duration = Duration::from_secs(30);
const SOURCE_HTTP_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug)]
pub(crate) struct MaterializedSource {
    pub(crate) repo_root: PathBuf,
    pub(crate) resolved_ref: Option<String>,
}

pub(crate) fn materialize_source(
    locator: &PluginSourceLocator,
) -> Result<MaterializedSource, ContractError> {
    match locator {
        PluginSourceLocator::Git { remote, git_ref } => materialize_git(remote, git_ref.as_deref()),
        PluginSourceLocator::GitHub {
            owner,
            repo,
            git_ref,
        } => materialize_github(owner, repo, git_ref.as_deref()),
    }
}

fn materialize_git(
    remote: &str,
    git_ref: Option<&str>,
) -> Result<MaterializedSource, ContractError> {
    let temp_root = create_temp_dir("plugin-source-git")?;
    let repo_root = temp_root.join("repo");
    let remote_path = Path::new(remote);
    if remote_path.exists() && git_ref.is_none() {
        ensure_clean_local_git_source(remote_path)?;
        copy_tree_strict(remote_path, &repo_root)?;
        let resolved_ref = if remote_path.join(".git").exists() {
            Some(git_head(Some(remote_path))?)
        } else {
            None
        };
        return Ok(MaterializedSource {
            repo_root,
            resolved_ref,
        });
    }
    run_git(
        None,
        &["clone", remote, repo_root.to_string_lossy().as_ref()],
    )?;
    if let Some(value) = git_ref {
        run_git(Some(&repo_root), &["checkout", value])?;
    }
    let resolved_ref = git_head(Some(&repo_root))?;
    Ok(MaterializedSource {
        repo_root,
        resolved_ref: Some(resolved_ref),
    })
}

fn materialize_github(
    owner: &str,
    repo: &str,
    git_ref: Option<&str>,
) -> Result<MaterializedSource, ContractError> {
    let temp_root = create_temp_dir("plugin-source-github")?;
    let archive_url = match git_ref {
        Some(value) => format!("https://api.github.com/repos/{owner}/{repo}/zipball/{value}"),
        None => format!("https://api.github.com/repos/{owner}/{repo}/zipball"),
    };
    let mut headers = HeaderMap::new();
    headers.insert(USER_AGENT, HeaderValue::from_static("chainbot-cli"));
    headers.insert(
        ACCEPT,
        HeaderValue::from_static("application/vnd.github+json"),
    );
    let client = Client::builder()
        .default_headers(headers)
        .connect_timeout(SOURCE_HTTP_TIMEOUT)
        .timeout(SOURCE_HTTP_TIMEOUT)
        .build()
        .map_err(|source| ContractError::CliUsage {
            message: format!("failed to build github client: {source}"),
        })?;
    let response = client
        .get(archive_url)
        .send()
        .map_err(|source| ContractError::CliUsage {
            message: format!("failed to fetch github source archive: {source}"),
        })?
        .error_for_status()
        .map_err(|source| ContractError::CliUsage {
            message: format!("github source archive request failed: {source}"),
        })?;
    let bytes = response.bytes().map_err(|source| ContractError::CliUsage {
        message: format!("failed to read github source archive: {source}"),
    })?;
    let mut archive =
        ZipArchive::new(Cursor::new(bytes)).map_err(|source| ContractError::CliUsage {
            message: format!("failed to decode github source archive: {source}"),
        })?;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|source| ContractError::CliUsage {
                message: format!("failed to read github archive entry: {source}"),
            })?;
        let enclosed = entry
            .enclosed_name()
            .ok_or_else(|| ContractError::CliUsage {
                message: "github archive contained an unsafe path".to_owned(),
            })?;
        let output_path = temp_root.join(enclosed);
        if entry.name().ends_with('/') {
            fs::create_dir_all(&output_path).map_err(|source| ContractError::Io {
                path: output_path.clone(),
                operation: "create extracted directory",
                source,
            })?;
            continue;
        }
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).map_err(|source| ContractError::Io {
                path: parent.to_path_buf(),
                operation: "create extracted parent directory",
                source,
            })?;
        }
        let mut file = fs::File::create(&output_path).map_err(|source| ContractError::Io {
            path: output_path.clone(),
            operation: "create extracted file",
            source,
        })?;
        std::io::copy(&mut entry, &mut file).map_err(|source| ContractError::Io {
            path: output_path.clone(),
            operation: "write extracted file",
            source,
        })?;
        restore_archive_entry_permissions(entry.unix_mode(), &output_path)?;
    }
    let repo_root = detect_single_root(&temp_root)?;
    let resolved_ref = git_ref.map(str::to_owned);
    Ok(MaterializedSource {
        repo_root,
        resolved_ref,
    })
}

fn detect_single_root(temp_root: &Path) -> Result<PathBuf, ContractError> {
    let mut directories = fs::read_dir(temp_root)
        .map_err(|source| ContractError::Io {
            path: temp_root.to_path_buf(),
            operation: "read extracted root directory",
            source,
        })?
        .filter_map(|entry| entry.ok().map(|value| value.path()))
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    directories.sort();
    match directories.as_slice() {
        [repo_root] => Ok(repo_root.clone()),
        _ => Err(ContractError::CliUsage {
            message: format!(
                "github source archive must extract to exactly one root directory under {}",
                temp_root.display()
            ),
        }),
    }
}

fn run_git(cwd: Option<&Path>, args: &[&str]) -> Result<(), ContractError> {
    let mut command = Command::new("git");
    if let Some(path) = cwd {
        command.current_dir(path);
    }
    command.args(args);
    let output = run_command_with_timeout(&mut command, SOURCE_GIT_TIMEOUT).map_err(|source| {
        ContractError::CliUsage {
            message: format!("failed to start git {}: {source}", args.join(" ")),
        }
    })?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    Err(ContractError::CliUsage {
        message: format!("git {} failed: {stderr}", args.join(" ")),
    })
}

fn ensure_clean_local_git_source(remote_path: &Path) -> Result<(), ContractError> {
    if !remote_path.join(".git").exists() {
        return Ok(());
    }

    let mut command = Command::new("git");
    command
        .current_dir(remote_path)
        .args(["status", "--porcelain", "--untracked-files=all"]);
    let output = run_command_with_timeout(&mut command, SOURCE_GIT_TIMEOUT).map_err(|source| {
        ContractError::CliUsage {
            message: format!(
                "failed to inspect local git source cleanliness at {}: {source}",
                remote_path.display()
            ),
        }
    })?;
    if !output.status.success() {
        return Err(ContractError::CliUsage {
            message: format!(
                "git status failed for local plugin source {}: {}",
                remote_path.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        });
    }

    if String::from_utf8_lossy(&output.stdout).trim().is_empty() {
        return Ok(());
    }

    Err(ContractError::CliUsage {
        message: format!(
            "local git plugin source {} has uncommitted or untracked changes; commit or stash them first, or re-run with --ref to install a committed revision",
            remote_path.display()
        ),
    })
}

fn restore_archive_entry_permissions(
    unix_mode: Option<u32>,
    output_path: &Path,
) -> Result<(), ContractError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        if let Some(mode) = unix_mode {
            fs::set_permissions(output_path, fs::Permissions::from_mode(mode)).map_err(
                |source| ContractError::Io {
                    path: output_path.to_path_buf(),
                    operation: "restore extracted file permissions",
                    source,
                },
            )?;
        }
    }

    Ok(())
}

fn run_command_with_timeout(
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
                        format!("command timed out after {timeout:?}"),
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

fn git_head(cwd: Option<&Path>) -> Result<String, ContractError> {
    let mut command = Command::new("git");
    if let Some(path) = cwd {
        command.current_dir(path);
    }
    command.args(["rev-parse", "HEAD"]);
    let output = run_command_with_timeout(&mut command, SOURCE_GIT_TIMEOUT).map_err(|source| {
        ContractError::CliUsage {
            message: format!("failed to read git HEAD: {source}"),
        }
    })?;
    if !output.status.success() {
        return Err(ContractError::CliUsage {
            message: format!(
                "git rev-parse HEAD failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        });
    }
    let head = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if head.is_empty() {
        return Err(ContractError::CliUsage {
            message: "git rev-parse HEAD returned an empty value".to_owned(),
        });
    }
    Ok(head)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(unix)]
    fn github_archive_permissions_are_restored() {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;

        let temp_root = create_temp_dir("transport-permissions-test")
            .expect("temp root should be creatable for permission test");
        let archive_path = temp_root.join("fixture.zip");
        let extracted_path = temp_root.join("plugin.sh");

        let archive_file =
            fs::File::create(&archive_path).expect("archive file should be creatable");
        let mut writer = zip::ZipWriter::new(archive_file);
        let options: zip::write::FileOptions<'_, ()> =
            zip::write::FileOptions::default().unix_permissions(0o755);
        writer
            .start_file("plugin.sh", options)
            .expect("zip file entry should start");
        writer
            .write_all(b"#!/bin/sh\necho restored\n")
            .expect("zip file entry should be writable");
        writer.finish().expect("zip archive should finish cleanly");

        let archive_file = fs::File::open(&archive_path).expect("archive file should open");
        let mut archive =
            ZipArchive::new(archive_file).expect("zip archive should be readable for test");
        let mut entry = archive.by_index(0).expect("first entry should exist");
        let mut extracted = fs::File::create(&extracted_path)
            .expect("extracted path should be creatable for permission test");
        std::io::copy(&mut entry, &mut extracted).expect("archive entry should copy");

        restore_archive_entry_permissions(entry.unix_mode(), &extracted_path)
            .expect("archive entry permissions should restore cleanly");

        let mode = fs::metadata(&extracted_path)
            .expect("restored file metadata should exist")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o755);
    }
}
