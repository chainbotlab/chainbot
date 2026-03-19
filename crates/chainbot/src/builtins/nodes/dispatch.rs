use crate::executor::NodeDefinition;

pub fn builtin_dispatch_kind(node: &NodeDefinition) -> Option<&str> {
    if node.kind == "builtin" {
        return Some(node.plugin_id.as_str());
    }

    if node.kind.starts_with("builtin.") {
        return Some(node.kind.as_str());
    }

    None
}
