//! [INPUT]
//! Builtin node specs and stable authoring semantics from the workflow execution surface.
//!
//! [OUTPUT]
//! Exposes static builtin-node descriptors derived from the canonical node spec source.
//!
//! [ROLE]
//! Maps canonical builtin node specs into discoverability metadata for the CLI layer.

use super::spec::{builtin_node_specs, BuiltinNodeSpec};

pub type BuiltinNodeCatalogDescriptor = BuiltinNodeSpec;

pub fn builtin_node_descriptors() -> &'static [BuiltinNodeCatalogDescriptor] {
    builtin_node_specs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builtins::nodes::spec::*;
    use std::collections::BTreeSet;

    #[test]
    fn builtin_node_descriptors_cover_registered_kinds() {
        let expected = BTreeSet::from([
            BUILTIN_ASSERT_KIND,
            BUILTIN_FAIL_KIND,
            BUILTIN_DATA_PICK_KIND,
            BUILTIN_DATA_MERGE_KIND,
            BUILTIN_DATA_TEMPLATE_KIND,
            BUILTIN_DATA_GET_KIND,
            BUILTIN_DATA_COALESCE_KIND,
            BUILTIN_DATA_COMPARE_KIND,
            BUILTIN_DATA_PARSE_JSON_KIND,
            BUILTIN_DATA_STRINGIFY_JSON_KIND,
            BUILTIN_DATA_MATH_KIND,
            BUILTIN_IDENTITY_KIND,
            BUILTIN_EMIT_SUBFLOW_OUTPUT_KIND,
            BUILTIN_SCRIPT_KIND,
            BUILTIN_HTTP_KIND,
        ]);
        let actual = builtin_node_descriptors()
            .iter()
            .map(|descriptor| descriptor.kind)
            .collect::<BTreeSet<_>>();
        assert_eq!(actual, expected);
    }
}
