//! [INPUT]
//! Builtin node handler contracts, registry kind constants, and stable authoring semantics from the workflow execution surface.
//!
//! [OUTPUT]
//! Defines static builtin-node descriptors for CLI catalog discovery and completeness tests.
//!
//! [ROLE]
//! Keeps builtin node discoverability metadata co-located with builtin node ownership rather than the CLI layer.

use super::registry::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuiltinNodeCatalogDescriptor {
    pub kind: &'static str,
    pub summary: &'static str,
    pub operations: &'static [&'static str],
    pub inputs: &'static [&'static str],
    pub outputs: &'static [&'static str],
}

const BUILTIN_NODE_DESCRIPTORS: &[BuiltinNodeCatalogDescriptor] = &[
    BuiltinNodeCatalogDescriptor {
        kind: BUILTIN_ASSERT_KIND,
        summary: "Assert workflow invariants and fail when the condition does not pass.",
        operations: &["truthy", "falsy", "exists", "equals", "not_equals"],
        inputs: &["value", "expected", "message", "code"],
        outputs: &["ok", "checked_value"],
    },
    BuiltinNodeCatalogDescriptor {
        kind: BUILTIN_FAIL_KIND,
        summary: "Raise an explicit workflow failure with optional message and code.",
        operations: &["raise"],
        inputs: &["message", "code"],
        outputs: &[],
    },
    BuiltinNodeCatalogDescriptor {
        kind: BUILTIN_DATA_PICK_KIND,
        summary: "Pick selected top-level fields from an input object.",
        operations: &["fields"],
        inputs: &["input", "fields"],
        outputs: &["result"],
    },
    BuiltinNodeCatalogDescriptor {
        kind: BUILTIN_DATA_MERGE_KIND,
        summary: "Shallow-merge objects into a single result object.",
        operations: &["objects"],
        inputs: &["objects", "left", "right", "extra"],
        outputs: &["result"],
    },
    BuiltinNodeCatalogDescriptor {
        kind: BUILTIN_DATA_TEMPLATE_KIND,
        summary: "Render {{placeholder}} templates from a values object.",
        operations: &["render"],
        inputs: &["template", "values"],
        outputs: &["result"],
    },
    BuiltinNodeCatalogDescriptor {
        kind: BUILTIN_DATA_GET_KIND,
        summary: "Resolve a dotted path from an input JSON value.",
        operations: &["path"],
        inputs: &["input", "path"],
        outputs: &["result"],
    },
    BuiltinNodeCatalogDescriptor {
        kind: BUILTIN_DATA_COALESCE_KIND,
        summary: "Return the first non-null entry from a values array.",
        operations: &["run"],
        inputs: &["values"],
        outputs: &["result"],
    },
    BuiltinNodeCatalogDescriptor {
        kind: BUILTIN_DATA_COMPARE_KIND,
        summary: "Compare scalar values and emit a boolean result without failing on false.",
        operations: &[
            "equals",
            "not_equals",
            "less_than",
            "less_than_or_equals",
            "greater_than",
            "greater_than_or_equals",
        ],
        inputs: &["left", "right"],
        outputs: &["result"],
    },
    BuiltinNodeCatalogDescriptor {
        kind: BUILTIN_DATA_PARSE_JSON_KIND,
        summary: "Parse a string input into a JSON value.",
        operations: &["parse"],
        inputs: &["text"],
        outputs: &["result"],
    },
    BuiltinNodeCatalogDescriptor {
        kind: BUILTIN_DATA_STRINGIFY_JSON_KIND,
        summary: "Serialize a JSON-compatible value into a JSON string.",
        operations: &["stringify"],
        inputs: &["value"],
        outputs: &["result"],
    },
    BuiltinNodeCatalogDescriptor {
        kind: BUILTIN_DATA_MATH_KIND,
        summary: "Apply numeric math operations and emit the numeric result.",
        operations: &[
            "add", "subtract", "multiply", "divide", "min", "max", "round",
        ],
        inputs: &["left", "right", "value", "precision"],
        outputs: &["result"],
    },
    BuiltinNodeCatalogDescriptor {
        kind: BUILTIN_IDENTITY_KIND,
        summary: "Pass inputs through unchanged.",
        operations: &["run"],
        inputs: &["*"],
        outputs: &["*"],
    },
    BuiltinNodeCatalogDescriptor {
        kind: BUILTIN_EMIT_SUBFLOW_OUTPUT_KIND,
        summary: "Publish values into the subflow output namespace.",
        operations: &["run"],
        inputs: &["*"],
        outputs: &["subflow_output:*"],
    },
    BuiltinNodeCatalogDescriptor {
        kind: BUILTIN_SCRIPT_KIND,
        summary: "Execute a Python or JavaScript worker script from the workflow package.",
        operations: &["python:<relative_path>", "javascript:<relative_path>"],
        inputs: &["payload:*"],
        outputs: &["result", "object_fields:*"],
    },
    BuiltinNodeCatalogDescriptor {
        kind: BUILTIN_HTTP_KIND,
        summary: "Send an HTTP request and capture status, body, URL, and headers.",
        operations: &["<url>"],
        inputs: &["method", "headers", "body"],
        outputs: &["status", "ok", "url", "body", "headers"],
    },
];

pub fn builtin_node_descriptors() -> &'static [BuiltinNodeCatalogDescriptor] {
    BUILTIN_NODE_DESCRIPTORS
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::builtins::nodes::registry::{
        BUILTIN_ASSERT_KIND, BUILTIN_DATA_COALESCE_KIND, BUILTIN_DATA_COMPARE_KIND,
        BUILTIN_DATA_GET_KIND, BUILTIN_DATA_MATH_KIND, BUILTIN_DATA_MERGE_KIND,
        BUILTIN_DATA_PARSE_JSON_KIND, BUILTIN_DATA_PICK_KIND, BUILTIN_DATA_STRINGIFY_JSON_KIND,
        BUILTIN_DATA_TEMPLATE_KIND, BUILTIN_EMIT_SUBFLOW_OUTPUT_KIND, BUILTIN_FAIL_KIND,
        BUILTIN_HTTP_KIND, BUILTIN_IDENTITY_KIND, BUILTIN_SCRIPT_KIND,
    };
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
