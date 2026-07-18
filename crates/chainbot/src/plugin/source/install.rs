//! [INPUT]
//! Prepared plugin package trees, plugin target directories, and transactional install requests.
//!
//! [OUTPUT]
//! Swaps prepared plugin packages into the root with rollback support and reports the install result.
//!
//! [ROLE]
//! Owns transactional plugin-package replacement for source-based installs.

use std::fs::{self, File, OpenOptions};
use std::os::fd::AsRawFd;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::errors::ContractError;

use super::fs::{copy_tree_strict, create_temp_dir_in, remove_path_if_exists};
use super::prepare::PreparedPlugin;

#[derive(Debug)]
pub(crate) struct InstallTransaction {
    target_dir: PathBuf,
    backup_dir: Option<PathBuf>,
    staging_root: PathBuf,
    journal_path: PathBuf,
    state: InstallTransactionState,
    _lock: InstallTransactionLock,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum InstallTransactionState {
    Staged,
    BackupPending,
    BackupMoved,
    Promoted,
    Finalized,
    RolledBack,
}

impl InstallTransactionState {
    fn is_terminal(self) -> bool {
        matches!(self, Self::Finalized | Self::RolledBack)
    }
}

#[derive(Debug)]
struct InstallTransactionLock(File);

impl Drop for InstallTransactionLock {
    fn drop(&mut self) {
        // SAFETY: the file descriptor belongs to this lock and remains valid until drop.
        unsafe {
            libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct InstallTransactionJournal {
    target_dir: PathBuf,
    backup_dir: Option<PathBuf>,
    staging_root: PathBuf,
    state: InstallTransactionState,
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
        let transaction_lock = acquire_install_transaction_lock(&transaction_dir)?;
        recover_pending_transactions(plugins_dir, &transaction_dir)?;

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
        let staging_root = create_temp_dir_in(&transaction_dir, "plugin-stage")?;
        let staged_dir = staging_root.join(&prepared.plugin_id);
        copy_tree_strict(&prepared.package_root, &staged_dir)?;
        let mut transaction = Self {
            target_dir: target_dir.clone(),
            backup_dir: None,
            journal_path: transaction_dir.join(format!("install-{}.json", prepared.plugin_id)),
            staging_root,
            state: InstallTransactionState::Staged,
            _lock: transaction_lock,
        };
        transaction.persist_journal()?;

        if target_dir.exists() {
            let backup_dir = transaction_dir.join(format!("backup-{}", prepared.plugin_id));
            remove_path_if_exists(&backup_dir)?;
            transaction.backup_dir = Some(backup_dir.clone());
            transaction.state = InstallTransactionState::BackupPending;
            transaction.persist_journal()?;
            fs::rename(&target_dir, &backup_dir).map_err(|source| ContractError::Io {
                path: target_dir.clone(),
                operation: "move existing plugin to backup",
                source,
            })?;
            transaction.state = InstallTransactionState::BackupMoved;
            transaction.persist_journal()?;
        }

        if let Err(source) = fs::rename(&staged_dir, &target_dir) {
            let promotion_error = ContractError::Io {
                path: staged_dir,
                operation: "promote staged plugin into target directory",
                source,
            };
            return match transaction.rollback() {
                Ok(()) => Err(promotion_error),
                Err(rollback_error) => Err(ContractError::CliUsage {
                    message: format!(
                        "plugin promotion failed ({promotion_error}) and rollback failed ({rollback_error})"
                    ),
                }),
            };
        }
        transaction.state = InstallTransactionState::Promoted;
        transaction.persist_journal()?;
        let result = InstallTransactionResult {
            plugin_id: prepared.plugin_id.clone(),
            target_dir,
            replaced_existing: transaction.backup_dir.is_some(),
        };
        Ok((transaction, result))
    }

    pub(crate) fn finalize(mut self) -> Result<(), ContractError> {
        if let Some(backup_dir) = self.backup_dir.take() {
            remove_path_if_exists(&backup_dir)?;
        }
        remove_path_if_exists(&self.staging_root)?;
        remove_path_if_exists(&self.journal_path)?;
        self.state = InstallTransactionState::Finalized;
        Ok(())
    }

    pub(crate) fn rollback(mut self) -> Result<(), ContractError> {
        if matches!(
            self.state,
            InstallTransactionState::BackupMoved | InstallTransactionState::Promoted
        ) {
            remove_path_if_exists(&self.target_dir)?;
        }
        if let Some(backup_dir) = self.backup_dir.as_ref() {
            fs::rename(backup_dir, &self.target_dir).map_err(|source| ContractError::Io {
                path: backup_dir.clone(),
                operation: "restore plugin backup",
                source,
            })?;
        }
        remove_path_if_exists(&self.staging_root)?;
        remove_path_if_exists(&self.journal_path)?;
        self.backup_dir = None;
        self.state = InstallTransactionState::RolledBack;
        Ok(())
    }
}

impl InstallTransaction {
    fn persist_journal(&self) -> Result<(), ContractError> {
        let journal = InstallTransactionJournal {
            target_dir: self.target_dir.clone(),
            backup_dir: self.backup_dir.clone(),
            staging_root: self.staging_root.clone(),
            state: self.state,
        };
        let bytes = serde_json::to_vec(&journal).map_err(|source| ContractError::CliUsage {
            message: format!("serialize plugin install transaction journal: {source}"),
        })?;
        let temporary_path = self.journal_path.with_extension("tmp");
        fs::write(&temporary_path, bytes).map_err(|source| ContractError::Io {
            path: temporary_path.clone(),
            operation: "write plugin install transaction journal",
            source,
        })?;
        fs::rename(&temporary_path, &self.journal_path).map_err(|source| ContractError::Io {
            path: self.journal_path.clone(),
            operation: "commit plugin install transaction journal",
            source,
        })
    }
}

fn acquire_install_transaction_lock(
    transaction_dir: &Path,
) -> Result<InstallTransactionLock, ContractError> {
    let lock_path = transaction_dir.join("install.lock");
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .open(&lock_path)
        .map_err(|source| ContractError::Io {
            path: lock_path.clone(),
            operation: "open plugin install transaction lock",
            source,
        })?;
    // SAFETY: the descriptor remains owned by the returned RAII lock.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } == -1 {
        return Err(ContractError::Io {
            path: lock_path,
            operation: "acquire plugin install transaction lock",
            source: std::io::Error::last_os_error(),
        });
    }
    Ok(InstallTransactionLock(file))
}

fn validate_journal_path(
    path: &Path,
    root: &Path,
    field: &str,
    direct_child: bool,
) -> Result<(), ContractError> {
    let relative = path.strip_prefix(root).map_err(|_| ContractError::CliUsage {
        message: format!("plugin install transaction journal {field} escapes {}: {}", root.display(), path.display()),
    })?;
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(ContractError::CliUsage {
            message: format!("plugin install transaction journal {field} contains unsafe path components: {}", path.display()),
        });
    }
    let component_count = relative.components().count();
    if component_count == 0 || (direct_child && component_count != 1) {
        return Err(ContractError::CliUsage {
            message: format!("plugin install transaction journal {field} has invalid path: {}", path.display()),
        });
    }
    Ok(())
}

fn recover_pending_transactions(plugins_dir: &Path, transaction_dir: &Path) -> Result<(), ContractError> {
    let entries = fs::read_dir(transaction_dir).map_err(|source| ContractError::Io {
        path: transaction_dir.to_path_buf(),
        operation: "read plugin transaction directory",
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| ContractError::Io {
            path: transaction_dir.to_path_buf(),
            operation: "read plugin transaction entry",
            source,
        })?;
        let journal_path = entry.path();
        if !journal_path.is_file()
            || !journal_path.file_name().is_some_and(|name| name.to_string_lossy().starts_with("install-"))
            || journal_path.extension().is_none_or(|extension| extension != "json")
        {
            continue;
        }
        let bytes = fs::read(&journal_path).map_err(|source| ContractError::Io {
            path: journal_path.clone(),
            operation: "read plugin install transaction journal",
            source,
        })?;
        let journal: InstallTransactionJournal = serde_json::from_slice(&bytes).map_err(|source| ContractError::CliUsage {
            message: format!("decode plugin install transaction journal {}: {source}", journal_path.display()),
        })?;
        validate_journal_path(
            &journal.target_dir,
            plugins_dir,
            "target_dir",
            true,
        )?;
        if let Some(backup_dir) = journal.backup_dir.as_ref() {
            validate_journal_path(backup_dir, transaction_dir, "backup_dir", false)?;
        }
        validate_journal_path(
            &journal.staging_root,
            transaction_dir,
            "staging_root",
            false,
        )?;
        let target_is_valid = journal.target_dir.join("config.toml").is_file();
        match journal.state {
            InstallTransactionState::Staged => {}
            InstallTransactionState::BackupPending => {
                if !journal.target_dir.exists()
                    && let Some(backup_dir) = journal.backup_dir.as_ref().filter(|path| path.exists())
                {
                    fs::rename(backup_dir, &journal.target_dir).map_err(|source| ContractError::Io {
                        path: backup_dir.clone(),
                        operation: "restore interrupted plugin backup intent",
                        source,
                    })?;
                } else if journal.target_dir.exists()
                    && let Some(backup_dir) = journal.backup_dir.as_ref()
                    && backup_dir.exists()
                {
                    remove_path_if_exists(backup_dir)?;
                }
            }
            InstallTransactionState::BackupMoved => {
                if !journal.target_dir.exists()
                    && let Some(backup_dir) = journal.backup_dir.as_ref().filter(|path| path.exists())
                {
                    fs::rename(backup_dir, &journal.target_dir).map_err(|source| ContractError::Io {
                        path: backup_dir.clone(),
                        operation: "restore interrupted plugin install backup",
                        source,
                    })?;
                } else if target_is_valid
                    && let Some(backup_dir) = journal.backup_dir.as_ref()
                {
                    remove_path_if_exists(backup_dir)?;
                }
            }
            InstallTransactionState::Promoted => {
                if !target_is_valid {
                    remove_path_if_exists(&journal.target_dir)?;
                    if let Some(backup_dir) = journal.backup_dir.as_ref().filter(|path| path.exists()) {
                        fs::rename(backup_dir, &journal.target_dir).map_err(|source| ContractError::Io {
                            path: backup_dir.clone(),
                            operation: "restore invalid promoted plugin backup",
                            source,
                        })?;
                    }
                } else if let Some(backup_dir) = journal.backup_dir.as_ref() {
                    remove_path_if_exists(backup_dir)?;
                }
            }
            InstallTransactionState::Finalized | InstallTransactionState::RolledBack => {}
        }
        remove_path_if_exists(&journal.staging_root)?;
        remove_path_if_exists(&journal_path)?;
    }
    Ok(())
}

impl Drop for InstallTransaction {
    fn drop(&mut self) {
        if self.state.is_terminal() {
            return;
        }
        let target_removed = if matches!(
            self.state,
            InstallTransactionState::BackupMoved | InstallTransactionState::Promoted
        ) && self.backup_dir.is_some() {
            remove_path_if_exists(&self.target_dir).is_ok()
        } else {
            true
        };
        let backup_restored = if target_removed {
            if let Some(backup_dir) = self.backup_dir.as_ref() {
                fs::rename(backup_dir, &self.target_dir).is_ok()
            } else {
                true
            }
        } else {
            false
        };
        if !target_removed || !backup_restored {
            return;
        }
        if remove_path_if_exists(&self.staging_root).is_err()
            || remove_path_if_exists(&self.journal_path).is_err()
        {
            return;
        }
        self.backup_dir = None;
        self.state = InstallTransactionState::RolledBack;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn recovery_restores_backup_after_interrupted_promotion() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("chainbot-install-recovery-{unique}"));
        let plugins_dir = root.join("plugins");
        let transaction_dir = root.join(".chainbot/plugin-transactions");
        let target_dir = plugins_dir.join("example");
        let backup_dir = transaction_dir.join("backup-example");
        let staging_root = transaction_dir.join("plugin-stage-interrupted");
        let journal_path = transaction_dir.join("install-example.json");
        fs::create_dir_all(&plugins_dir).expect("plugins directory should be creatable");
        fs::create_dir_all(&backup_dir).expect("backup directory should be creatable");
        fs::write(backup_dir.join("config.toml"), "plugin_id = 'example'")
            .expect("backup manifest should be writable");
        fs::create_dir_all(&staging_root).expect("staging directory should be creatable");
        let journal = InstallTransactionJournal {
            target_dir: target_dir.clone(),
            backup_dir: Some(backup_dir),
            staging_root: staging_root.clone(),
            state: InstallTransactionState::BackupMoved,
        };
        fs::write(
            &journal_path,
            serde_json::to_vec(&journal).expect("journal should serialize"),
        )
        .expect("journal should be writable");

        recover_pending_transactions(&plugins_dir, &transaction_dir)
            .expect("interrupted transaction should recover");

        assert!(target_dir.join("config.toml").is_file());
        assert!(!journal_path.exists());
        assert!(!staging_root.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn recovery_restores_backup_after_pending_backup_intent() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("chainbot-install-pending-{unique}"));
        let plugins_dir = root.join("plugins");
        let transaction_dir = root.join(".chainbot/plugin-transactions");
        let target_dir = plugins_dir.join("example");
        let backup_dir = transaction_dir.join("backup-example");
        let staging_root = transaction_dir.join("plugin-stage-pending");
        let journal_path = transaction_dir.join("install-example.json");
        fs::create_dir_all(&plugins_dir).expect("plugins directory should be creatable");
        fs::create_dir_all(&backup_dir).expect("backup directory should be creatable");
        fs::write(backup_dir.join("config.toml"), "plugin_id = 'example'")
            .expect("backup manifest should be writable");
        fs::create_dir_all(&staging_root).expect("staging directory should be creatable");
        let journal = InstallTransactionJournal {
            target_dir: target_dir.clone(),
            backup_dir: Some(backup_dir),
            staging_root: staging_root.clone(),
            state: InstallTransactionState::BackupPending,
        };
        fs::create_dir_all(&transaction_dir).expect("transaction directory should be creatable");
        fs::write(
            &journal_path,
            serde_json::to_vec(&journal).expect("journal should serialize"),
        )
        .expect("journal should be writable");

        recover_pending_transactions(&plugins_dir, &transaction_dir)
            .expect("pending backup intent should recover");

        assert!(target_dir.join("config.toml").is_file());
        assert!(!journal_path.exists());
        assert!(!staging_root.exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn recovery_rejects_journal_parent_path_before_mutation() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("chainbot-install-unsafe-{unique}"));
        let plugins_dir = root.join("plugins");
        let transaction_dir = root.join(".chainbot/plugin-transactions");
        let journal_path = transaction_dir.join("install-example.json");
        fs::create_dir_all(&transaction_dir).expect("transaction directory should be creatable");
        let journal = InstallTransactionJournal {
            target_dir: plugins_dir.join("..").join("outside"),
            backup_dir: None,
            staging_root: transaction_dir.join("stage"),
            state: InstallTransactionState::Staged,
        };
        fs::write(
            &journal_path,
            serde_json::to_vec(&journal).expect("journal should serialize"),
        )
        .expect("journal should be writable");

        let error = recover_pending_transactions(&plugins_dir, &transaction_dir)
            .expect_err("parent traversal in a journal should be rejected");
        assert!(error.to_string().contains("unsafe path components"));
        assert!(journal_path.exists());
        let _ = fs::remove_dir_all(root);
    }
}
