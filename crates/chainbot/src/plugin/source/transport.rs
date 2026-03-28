use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::Command;

use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, USER_AGENT};
use zip::ZipArchive;

use crate::errors::ContractError;

use super::fs::copy_tree_strict;
use super::fs::create_temp_dir;
use super::locator::PluginSourceLocator;

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

fn materialize_git(remote: &str, git_ref: Option<&str>) -> Result<MaterializedSource, ContractError> {
    let temp_root = create_temp_dir("plugin-source-git")?;
    let repo_root = temp_root.join("repo");
    let remote_path = Path::new(remote);
    if remote_path.exists() && git_ref.is_none() {
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
    run_git(None, &["clone", remote, repo_root.to_string_lossy().as_ref()])?;
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
    headers.insert(ACCEPT, HeaderValue::from_static("application/vnd.github+json"));
    let client = Client::builder()
        .default_headers(headers)
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
    let mut archive = ZipArchive::new(Cursor::new(bytes)).map_err(|source| ContractError::CliUsage {
        message: format!("failed to decode github source archive: {source}"),
    })?;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|source| ContractError::CliUsage {
            message: format!("failed to read github archive entry: {source}"),
        })?;
        let enclosed = entry.enclosed_name().ok_or_else(|| ContractError::CliUsage {
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
    let output = command.output().map_err(|source| ContractError::CliUsage {
        message: format!("failed to start git {}: {source}", args.join(" ")),
    })?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    Err(ContractError::CliUsage {
        message: format!("git {} failed: {stderr}", args.join(" ")),
    })
}

fn git_head(cwd: Option<&Path>) -> Result<String, ContractError> {
    let mut command = Command::new("git");
    if let Some(path) = cwd {
        command.current_dir(path);
    }
    let output = command
        .args(["rev-parse", "HEAD"])
        .output()
        .map_err(|source| ContractError::CliUsage {
            message: format!("failed to read git HEAD: {source}"),
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
