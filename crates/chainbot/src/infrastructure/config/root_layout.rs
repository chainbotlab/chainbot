//! [INPUT]
//! Root path overrides, home-directory fallbacks, root config path defaults, and validated root config contracts.
//!
//! [OUTPUT]
//! Resolves canonical root layout paths and applies root config path overrides with safety checks.
//!
//! [ROLE]
//! Owns root layout resolution and root-relative path composition inside infrastructure config loading.

use std::path::{Path, PathBuf};

use crate::errors::ContractError;

use super::package_loader::{
    resolve_root_relative_dir, validate_directory_exists, validate_file_exists,
};
use super::RootConfigDefinition;

pub const DEFAULT_ROOT_DIR_NAME: &str = ".chainbot";
pub const CHAINBOT_CONFIG_DIR_ENV: &str = "CHAINBOT_CONFIG_DIR";
pub const ROOT_CONFIG_FILE_NAME: &str = "chainbot.toml";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootLayout {
    pub root: PathBuf,
    pub config_dir: PathBuf,
    pub workflows_dir: PathBuf,
    pub triggers_dir: PathBuf,
    pub plugins_dir: PathBuf,
    pub secrets_dir: PathBuf,
    pub state_dir: PathBuf,
}

impl RootLayout {
    pub fn resolve() -> Result<Self, ContractError> {
        Self::resolve_with_env_home(None, None)
    }

    pub fn resolve_with_env_home(
        config_dir_override: Option<&Path>,
        home_override: Option<&Path>,
    ) -> Result<Self, ContractError> {
        let root = match config_dir_override {
            Some(path) => path.to_path_buf(),
            None => {
                if let Some(path) =
                    std::env::var_os(CHAINBOT_CONFIG_DIR_ENV).filter(|value| !value.is_empty())
                {
                    return Ok(Self::from_root(PathBuf::from(path)));
                }
                let home = match home_override {
                    Some(path) => path.to_path_buf(),
                    None => {
                        let home_os =
                            std::env::var_os("HOME").ok_or(ContractError::MissingHomeDirectory)?;
                        PathBuf::from(home_os)
                    }
                };
                home.join(DEFAULT_ROOT_DIR_NAME)
            }
        };

        Ok(Self::from_root(root))
    }

    pub fn from_root(root: PathBuf) -> Self {
        Self {
            config_dir: root.join("config"),
            workflows_dir: root.join("workflows"),
            triggers_dir: root.join("triggers"),
            plugins_dir: root.join("plugins"),
            secrets_dir: root.join("secrets"),
            state_dir: root.join("state"),
            root,
        }
    }

    pub fn root_config_path(&self) -> PathBuf {
        self.root.join(ROOT_CONFIG_FILE_NAME)
    }

    pub fn validate_bootstrap_paths_exist(&self) -> Result<(), ContractError> {
        validate_directory_exists(&self.root, "root")?;
        let root_config_path = self.root_config_path();
        validate_file_exists(&root_config_path, "root config")?;
        Ok(())
    }

    pub fn validate_paths_exist(&self) -> Result<(), ContractError> {
        validate_directory_exists(&self.root, "root")?;
        let root_config_path = self.root_config_path();
        validate_file_exists(&root_config_path, "root config")?;
        validate_directory_exists(&self.workflows_dir, "workflows")?;
        validate_directory_exists(&self.triggers_dir, "triggers")?;
        validate_directory_exists(&self.plugins_dir, "plugins")?;
        validate_directory_exists(&self.secrets_dir, "secrets")?;
        validate_directory_exists(&self.state_dir, "state")?;
        Ok(())
    }

    pub fn apply_root_config(
        &self,
        root_config: &RootConfigDefinition,
    ) -> Result<Self, ContractError> {
        Ok(Self {
            root: self.root.clone(),
            config_dir: self.config_dir.clone(),
            workflows_dir: resolve_root_relative_dir(
                &self.root,
                "root_config.paths.workflows_dir",
                root_config
                    .paths
                    .workflows_dir
                    .as_deref()
                    .unwrap_or("workflows"),
            )?,
            triggers_dir: resolve_root_relative_dir(
                &self.root,
                "root_config.paths.triggers_dir",
                root_config
                    .paths
                    .triggers_dir
                    .as_deref()
                    .unwrap_or("triggers"),
            )?,
            plugins_dir: resolve_root_relative_dir(
                &self.root,
                "root_config.paths.plugins_dir",
                root_config
                    .paths
                    .plugins_dir
                    .as_deref()
                    .unwrap_or("plugins"),
            )?,
            secrets_dir: resolve_root_relative_dir(
                &self.root,
                "root_config.paths.secrets_dir",
                root_config
                    .paths
                    .secrets_dir
                    .as_deref()
                    .unwrap_or("secrets"),
            )?,
            state_dir: resolve_root_relative_dir(
                &self.root,
                "root_config.paths.state_dir",
                root_config.paths.state_dir.as_deref().unwrap_or("state"),
            )?,
        })
    }
}
