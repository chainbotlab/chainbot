//! [INPUT]
//! Effective infrastructure root layouts, decoded package manifests, and cross-package contract references.
//!
//! [OUTPUT]
//! Exposes app-level definition entrypoints for root bundle assembly and bundle-wide validation.
//!
//! [ROLE]
//! Defines the application definitions boundary for configuration bundle assembly.
//!
//! ## Ownership Boundary
//!
//! ```text
//! infrastructure::config
//!   root_layout      — resolves root filesystem paths
//!   package_loader   — decodes TOML manifests from disk
//!   loader           — orchestrates full bundle load with version migration
//!
//! app::definitions
//!   load_root_definition_bundle  — entrypoint combining layout + loader output
//!   validate                    — bundle-wide contract validation (ingress desired-state,
//!                                 workflow/trigger/plugin identity checks)
//! ```
//!
//! The seam between infrastructure and app is at `load_root_definition_bundle`:
//! infrastructure produces decoded manifests; app assembles them into a `RootDefinitionBundle`
//! and runs bundle-wide validation before returning to callers (CLI, daemon, tests).

mod root_bundle;
mod validate;

pub use root_bundle::load_root_definition_bundle;
pub use validate::{collect_compatibility_warnings, CompatibilityWarning};
