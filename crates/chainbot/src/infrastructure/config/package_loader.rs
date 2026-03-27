//! [INPUT]
//! Root directories, package config TOML files, plugin package manifests, and root-relative path override values.
//!
//! [OUTPUT]
//! Provides TOML decoding, directory/file existence validation, package config discovery, and package-root hydration helpers.
//!
//! [ROLE]
//! Owns infrastructure-level package decoding and path validation primitives.

use std::fs;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;

use crate::domain::trigger::TriggerDefinition;
use crate::domain::workflow::WorkflowDefinition;
use crate::errors::ContractError;
use crate::plugin::PluginManifest;

pub const PACKAGE_CONFIG_FILE_NAME: &str = "config.toml";

pub(crate) fn validate_directory_exists(
    path: &Path,
    kind: &'static str,
) -> Result<(), ContractError> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_dir() => Ok(()),
        Ok(_) => Err(ContractError::MissingDirectory {
            path: path.to_path_buf(),
            kind,
        }),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            Err(ContractError::MissingDirectory {
                path: path.to_path_buf(),
                kind,
            })
        }
        Err(err) => Err(ContractError::Io {
            path: path.to_path_buf(),
            operation: "inspect metadata",
            source: err,
        }),
    }
}

pub(crate) fn validate_file_exists(path: &Path, kind: &'static str) -> Result<(), ContractError> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => Ok(()),
        Ok(_) => Err(ContractError::MissingFile {
            path: path.to_path_buf(),
            kind,
        }),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Err(ContractError::MissingFile {
            path: path.to_path_buf(),
            kind,
        }),
        Err(err) => Err(ContractError::Io {
            path: path.to_path_buf(),
            operation: "inspect metadata",
            source: err,
        }),
    }
}

pub(crate) fn decode_required_toml<T>(path: &Path, kind: &'static str) -> Result<T, ContractError>
where
    T: DeserializeOwned,
{
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Err(ContractError::MissingFile {
                path: path.to_path_buf(),
                kind,
            });
        }
        Err(err) => {
            return Err(ContractError::Io {
                path: path.to_path_buf(),
                operation: "read file",
                source: err,
            });
        }
    };

    toml::from_str(&contents)
        .map_err(|source| ContractError::toml_decode(path.to_path_buf(), &contents, source))
}

pub(crate) fn decode_package_collection<T>(directory: &Path) -> Result<Vec<T>, ContractError>
where
    T: DeserializeOwned + PackageManifestExt,
{
    let mut definitions = Vec::new();

    for path in collect_package_config_files(directory)? {
        let mut definition = decode_required_toml::<T>(&path, "definition")?;
        definition.set_package_root(
            path.parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(PathBuf::new),
        );
        definitions.push(definition);
    }

    Ok(definitions)
}

pub(crate) fn decode_plugin_manifests(
    plugins_dir: &Path,
) -> Result<Vec<PluginManifest>, ContractError> {
    let mut definitions = Vec::new();

    for path in collect_plugin_package_config_files(plugins_dir)? {
        let mut definition = decode_required_toml::<PluginManifest>(&path, "definition")?;
        definition.manifest_path = path.clone();
        definitions.push(definition);
    }

    Ok(definitions)
}

pub(crate) fn collect_plugin_package_config_files(
    plugins_dir: &Path,
) -> Result<Vec<PathBuf>, ContractError> {
    collect_package_config_files(plugins_dir)
}

pub(crate) fn collect_package_config_files(
    directory: &Path,
) -> Result<Vec<PathBuf>, ContractError> {
    let entries = fs::read_dir(directory).map_err(|source| ContractError::Io {
        path: directory.to_path_buf(),
        operation: "read directory",
        source,
    })?;

    let mut files = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| ContractError::Io {
            path: directory.to_path_buf(),
            operation: "read directory entry",
            source,
        })?;
        let path = entry.path();
        if path.is_dir() {
            let config_path = path.join(PACKAGE_CONFIG_FILE_NAME);
            if config_path.is_file() {
                files.push(config_path);
            }
        }
    }

    files.sort();
    Ok(files)
}

pub(crate) fn resolve_root_relative_dir(
    root: &Path,
    field: &'static str,
    value: &str,
) -> Result<PathBuf, ContractError> {
    let candidate = Path::new(value);
    if candidate.is_absolute() {
        return Err(ContractError::InvalidRootConfigField {
            field,
            detail: format!("absolute paths are not allowed: {value}"),
        });
    }

    if candidate
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(ContractError::InvalidRootConfigField {
            field,
            detail: format!("paths must stay within <root> and may not contain `..`: {value}"),
        });
    }

    Ok(root.join(candidate))
}

pub(crate) trait PackageManifestExt {
    fn set_package_root(&mut self, package_root: PathBuf);
}

impl PackageManifestExt for WorkflowDefinition {
    fn set_package_root(&mut self, package_root: PathBuf) {
        self.package_root = package_root;
    }
}

impl PackageManifestExt for TriggerDefinition {
    fn set_package_root(&mut self, package_root: PathBuf) {
        self.package_root = package_root;
    }
}
