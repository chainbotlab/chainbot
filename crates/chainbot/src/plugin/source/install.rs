use std::fs;
use std::path::{Path, PathBuf};

use crate::errors::ContractError;

use super::fs::{copy_tree_strict, create_temp_dir, remove_path_if_exists};
use super::prepare::PreparedPlugin;

#[derive(Debug)]
pub(crate) struct InstallTransaction {
    target_dir: PathBuf,
    backup_dir: Option<PathBuf>,
    staged_dir: PathBuf,
    finalized: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InstallTransactionResult {
    pub(crate) plugin_id: String,
    pub(crate) target_dir: PathBuf,
    pub(crate) replaced_existing: bool,
}

impl InstallTransaction {
    pub(crate) fn begin(
        plugins_dir: &Path,
        prepared: &PreparedPlugin,
        force: bool,
    ) -> Result<(Self, InstallTransactionResult), ContractError> {
        fs::create_dir_all(plugins_dir).map_err(|source| ContractError::Io {
            path: plugins_dir.to_path_buf(),
            operation: "create plugins directory",
            source,
        })?;
        let transaction_dir = plugins_dir
            .parent()
            .unwrap_or(plugins_dir)
            .join(".chainbot")
            .join("plugin-transactions");
        fs::create_dir_all(&transaction_dir).map_err(|source| ContractError::Io {
            path: transaction_dir.clone(),
            operation: "create plugin transaction directory",
            source,
        })?;
        let target_dir = plugins_dir.join(&prepared.plugin_id);
        if target_dir.exists() && !force {
            return Err(ContractError::CliUsage {
                message: format!(
                    "plugin {} is already installed at {}; re-run with --force to replace it",
                    prepared.plugin_id,
                    target_dir.display()
                ),
            });
        }
        let staging_root = create_temp_dir("plugin-stage")?;
        let staged_dir = staging_root.join(&prepared.plugin_id);
        copy_tree_strict(&prepared.package_root, &staged_dir)?;
        let backup_dir = if target_dir.exists() {
            let backup_dir = transaction_dir.join(format!("backup-{}", prepared.plugin_id));
            remove_path_if_exists(&backup_dir)?;
            fs::rename(&target_dir, &backup_dir).map_err(|source| ContractError::Io {
                path: target_dir.clone(),
                operation: "move existing plugin to backup",
                source,
            })?;
            Some(backup_dir)
        } else {
            None
        };
        fs::rename(&staged_dir, &target_dir).map_err(|source| ContractError::Io {
            path: staged_dir.clone(),
            operation: "promote staged plugin into target directory",
            source,
        })?;
        let result = InstallTransactionResult {
            plugin_id: prepared.plugin_id.clone(),
            target_dir: target_dir.clone(),
            replaced_existing: backup_dir.is_some(),
        };
        Ok((
            Self {
                target_dir,
                backup_dir,
                staged_dir,
                finalized: false,
            },
            result,
        ))
    }

    pub(crate) fn finalize(mut self) -> Result<(), ContractError> {
        if let Some(backup_dir) = self.backup_dir.take() {
            remove_path_if_exists(&backup_dir)?;
        }
        self.finalized = true;
        Ok(())
    }

    pub(crate) fn rollback(mut self) -> Result<(), ContractError> {
        remove_path_if_exists(&self.target_dir)?;
        if let Some(backup_dir) = self.backup_dir.take() {
            fs::rename(&backup_dir, &self.target_dir).map_err(|source| ContractError::Io {
                path: backup_dir.clone(),
                operation: "restore plugin backup",
                source,
            })?;
        }
        self.finalized = true;
        Ok(())
    }

}

impl Drop for InstallTransaction {
    fn drop(&mut self) {
        if self.finalized {
            let _ = remove_path_if_exists(&self.staged_dir);
        }
    }
}
