//! [INPUT]
//! Runtime `NodeDefinition` values from the workflow execution plane.
//!
//! [OUTPUT]
//! Resolves workflow node definitions into builtin node dispatch kinds when a node targets the builtin namespace.
//!
//! [ROLE]
//! Bridges workflow node contracts into builtin node registry keys.

use crate::domain::runtime::NodeDefinition;

pub fn builtin_dispatch_kind(node: &NodeDefinition) -> Option<&str> {
    if node.kind == "builtin" {
        return Some(node.plugin_id.as_str());
    }

    if node.kind.starts_with("builtin.") {
        return Some(node.kind.as_str());
    }

    None
}
