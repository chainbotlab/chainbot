//! [INPUT]
//! CLI-facing capability/runtime snapshots from catalog descriptors, root definitions, and runtime state reads.
//!
//! [OUTPUT]
//! CLI read models and human-readable renderers for catalog, plugin source, status, and observe commands.
//!
//! [ROLE]
//! Owns shell-facing read-model construction and rendering inside the CLI boundary.

pub(crate) mod catalog;
pub(crate) mod observe;
pub(crate) mod plugin_source;
pub(crate) mod status;
