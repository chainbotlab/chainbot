//! [INPUT]
//! Source locators, repository-local plugin source manifests, transport-backed repo trees, and install requests.
//!
//! [OUTPUT]
//! Defines the remote plugin source subsystem used for source discovery, install preparation, and safe replacement.
//!
//! [ROLE]
//! Owns plugin-source discoverability and installation as an internal vertical slice of the plugin subsystem.

mod discover;
mod fs;
mod install;
mod locator;
mod manifest;
mod prepare;
mod transport;

pub(crate) use discover::{
    build_list_output, build_show_output, discover_source_repository, resolve_plugin_selection,
};
pub(crate) use install::{InstallTransaction, InstallTransactionResult};
pub(crate) use locator::PluginSourceLocator;
pub(crate) use manifest::{
    PluginSourceDescriptor, PluginSourceDetail, PluginSourceListOutput, PluginSourceShowOutput,
};
pub(crate) use prepare::prepare_installable_plugin;
pub(crate) use transport::materialize_source;
