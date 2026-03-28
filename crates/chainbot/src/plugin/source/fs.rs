use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::errors::ContractError;

pub(crate) fn create_temp_dir(prefix: &str) -> Result<PathBuf, ContractError> {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| ContractError::CliUsage {
            message: format!("failed to compute temp dir timestamp: {error}"),
        })?
        .as_nanos();
    let path = std::env::temp_dir().join(format!("chainbot-{prefix}-{suffix}"));
    fs::create_dir_all(&path).map_err(|source| ContractError::Io {
        path: path.clone(),
        operation: "create temp directory",
        source,
    })?;
    Ok(path)
}

pub(crate) fn ensure_safe_relative_path(
    value: &str,
    field: &'static str,
) -> Result<PathBuf, ContractError> {
    let path = Path::new(value);
    if path.is_absolute() {
        return Err(ContractError::CliUsage {
            message: format!("{field} must be relative, got absolute path `{value}`"),
        });
    }
    if path.as_os_str().is_empty() {
        return Err(ContractError::CliUsage {
            message: format!("{field} must not be empty"),
        });
    }
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(ContractError::CliUsage {
            message: format!("{field} must stay within its root and may not contain `..`: {value}"),
        });
    }
    Ok(path.to_path_buf())
}

pub(crate) fn resolve_within_root(
    root: &Path,
    candidate: &Path,
    field: &'static str,
) -> Result<PathBuf, ContractError> {
    let joined = root.join(candidate);
    ensure_path_prefix(root, &joined, field)?;
    Ok(joined)
}

pub(crate) fn resolve_within_root_allow_parents(
    root: &Path,
    candidate: &Path,
    field: &'static str,
) -> Result<PathBuf, ContractError> {
    let root = fs::canonicalize(root).map_err(|source| ContractError::Io {
        path: root.to_path_buf(),
        operation: "canonicalize path root",
        source,
    })?;
    let normalized = normalize_join(&root, candidate);
    ensure_path_prefix(&root, &normalized, field)?;
    Ok(normalized)
}

pub(crate) fn ensure_path_prefix(
    root: &Path,
    path: &Path,
    field: &'static str,
) -> Result<(), ContractError> {
    if !path.starts_with(root) {
        return Err(ContractError::CliUsage {
            message: format!("{field} escapes its allowed root: {}", path.display()),
        });
    }
    Ok(())
}

pub(crate) fn normalize_join(root: &Path, rel: &Path) -> PathBuf {
    let mut normalized = root.to_path_buf();
    for component in rel.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(value) => normalized.push(value),
            Component::ParentDir => {
                normalized.pop();
            }
            Component::RootDir | Component::Prefix(_) => {}
        }
    }
    normalized
}

pub(crate) fn copy_tree_strict(from: &Path, to: &Path) -> Result<(), ContractError> {
    let metadata = fs::symlink_metadata(from).map_err(|source| ContractError::Io {
        path: from.to_path_buf(),
        operation: "inspect source metadata",
        source,
    })?;
    if metadata.file_type().is_symlink() {
        return Err(ContractError::CliUsage {
            message: format!("symlink inputs are not allowed: {}", from.display()),
        });
    }
    if metadata.is_dir() {
        fs::create_dir_all(to).map_err(|source| ContractError::Io {
            path: to.to_path_buf(),
            operation: "create directory",
            source,
        })?;
        for entry in fs::read_dir(from).map_err(|source| ContractError::Io {
            path: from.to_path_buf(),
            operation: "read directory",
            source,
        })? {
            let entry = entry.map_err(|source| ContractError::Io {
                path: from.to_path_buf(),
                operation: "read directory entry",
                source,
            })?;
            let name = entry.file_name();
            if name == ".git" {
                continue;
            }
            copy_tree_strict(&entry.path(), &to.join(name))?;
        }
        return Ok(());
    }
    if !metadata.is_file() {
        return Err(ContractError::CliUsage {
            message: format!("unsupported source entry type: {}", from.display()),
        });
    }
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent).map_err(|source| ContractError::Io {
            path: parent.to_path_buf(),
            operation: "create parent directory",
            source,
        })?;
    }
    fs::copy(from, to).map_err(|source| ContractError::Io {
        path: to.to_path_buf(),
        operation: "copy file",
        source,
    })?;
    let permissions = metadata.permissions();
    fs::set_permissions(to, permissions).map_err(|source| ContractError::Io {
        path: to.to_path_buf(),
        operation: "set file permissions",
        source,
    })?;
    Ok(())
}

pub(crate) fn read_shebang(path: &Path) -> Result<Option<String>, ContractError> {
    let contents = fs::read(path).map_err(|source| ContractError::Io {
        path: path.to_path_buf(),
        operation: "read file",
        source,
    })?;
    let line = contents
        .split(|byte| *byte == b'\n')
        .next()
        .map(|bytes| String::from_utf8_lossy(bytes).trim().to_owned())
        .unwrap_or_default();
    if line.starts_with("#!") {
        Ok(Some(line))
    } else {
        Ok(None)
    }
}

pub(crate) fn remove_path_if_exists(path: &Path) -> Result<(), ContractError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.is_dir() && !metadata.file_type().is_symlink() {
                fs::remove_dir_all(path).map_err(|source| ContractError::Io {
                    path: path.to_path_buf(),
                    operation: "remove directory",
                    source,
                })?;
            } else {
                fs::remove_file(path).map_err(|source| ContractError::Io {
                    path: path.to_path_buf(),
                    operation: "remove file",
                    source,
                })?;
            }
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(ContractError::Io {
            path: path.to_path_buf(),
            operation: "inspect metadata",
            source,
        }),
    }
}
