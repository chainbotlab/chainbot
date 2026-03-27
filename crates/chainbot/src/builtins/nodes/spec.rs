//! [INPUT]
//! Stable builtin node kind identifiers and CLI-facing builtin node descriptor fields.
//!
//! [OUTPUT]
//! Defines builtin node kind constants and specification records used for registry wiring and catalog discovery.
//!
//! [ROLE]
//! Serves as the canonical builtin node descriptor catalog.

pub(crate) const BUILTIN_ASSERT_KIND: &str = "builtin.flow.assert";
pub(crate) const BUILTIN_FAIL_KIND: &str = "builtin.flow.fail";
pub(crate) const BUILTIN_DATA_PICK_KIND: &str = "builtin.data.pick";
pub(crate) const BUILTIN_DATA_MERGE_KIND: &str = "builtin.data.merge";
pub(crate) const BUILTIN_DATA_TEMPLATE_KIND: &str = "builtin.data.template";
pub(crate) const BUILTIN_DATA_GET_KIND: &str = "builtin.data.get";
pub(crate) const BUILTIN_DATA_COALESCE_KIND: &str = "builtin.data.coalesce";
pub(crate) const BUILTIN_DATA_COMPARE_KIND: &str = "builtin.data.compare";
pub(crate) const BUILTIN_DATA_PARSE_JSON_KIND: &str = "builtin.data.parse_json";
pub(crate) const BUILTIN_DATA_STRINGIFY_JSON_KIND: &str = "builtin.data.stringify_json";
pub(crate) const BUILTIN_DATA_MATH_KIND: &str = "builtin.data.math";
pub(crate) const BUILTIN_IDENTITY_KIND: &str = "builtin.identity";
pub(crate) const BUILTIN_EMIT_SUBFLOW_OUTPUT_KIND: &str = "builtin.emit_subflow_output";
pub(crate) const BUILTIN_SCRIPT_KIND: &str = "builtin.script";
pub(crate) const BUILTIN_HTTP_KIND: &str = "builtin.http";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuiltinNodeSpec {
    pub kind: &'static str,
    pub summary: &'static str,
    pub operations: &'static [&'static str],
    pub inputs: &'static [&'static str],
    pub outputs: &'static [&'static str],
}

const BUILTIN_NODE_SPECS: &[BuiltinNodeSpec] = &[
    BuiltinNodeSpec {
        kind: BUILTIN_ASSERT_KIND,
        summary: "Assert workflow invariants and fail when the condition does not pass.",
        operations: &["truthy", "falsy", "exists", "equals", "not_equals"],
        inputs: &["value", "expected", "message", "code"],
        outputs: &["ok", "checked_value"],
    },
    BuiltinNodeSpec {
        kind: BUILTIN_FAIL_KIND,
        summary: "Raise an explicit workflow failure with optional message and code.",
        operations: &["raise"],
        inputs: &["message", "code"],
        outputs: &[],
    },
    BuiltinNodeSpec {
        kind: BUILTIN_DATA_PICK_KIND,
        summary: "Pick selected top-level fields from an input object.",
        operations: &["fields"],
        inputs: &["input", "fields"],
        outputs: &["result"],
    },
    BuiltinNodeSpec {
        kind: BUILTIN_DATA_MERGE_KIND,
        summary: "Shallow-merge objects into a single result object.",
        operations: &["objects"],
        inputs: &["objects", "left", "right", "extra"],
        outputs: &["result"],
    },
    BuiltinNodeSpec {
        kind: BUILTIN_DATA_TEMPLATE_KIND,
        summary: "Render {{placeholder}} templates from a values object.",
        operations: &["render"],
        inputs: &["template", "values"],
        outputs: &["result"],
    },
    BuiltinNodeSpec {
        kind: BUILTIN_DATA_GET_KIND,
        summary: "Resolve a dotted path from an input JSON value.",
        operations: &["path"],
        inputs: &["input", "path"],
        outputs: &["result"],
    },
    BuiltinNodeSpec {
        kind: BUILTIN_DATA_COALESCE_KIND,
        summary: "Return the first non-null entry from a values array.",
        operations: &["run"],
        inputs: &["values"],
        outputs: &["result"],
    },
    BuiltinNodeSpec {
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
    BuiltinNodeSpec {
        kind: BUILTIN_DATA_PARSE_JSON_KIND,
        summary: "Parse a string input into a JSON value.",
        operations: &["parse"],
        inputs: &["text"],
        outputs: &["result"],
    },
    BuiltinNodeSpec {
        kind: BUILTIN_DATA_STRINGIFY_JSON_KIND,
        summary: "Serialize a JSON-compatible value into a JSON string.",
        operations: &["stringify"],
        inputs: &["value"],
        outputs: &["result"],
    },
    BuiltinNodeSpec {
        kind: BUILTIN_DATA_MATH_KIND,
        summary: "Apply numeric math operations and emit the numeric result.",
        operations: &[
            "add", "subtract", "multiply", "divide", "min", "max", "round",
        ],
        inputs: &["left", "right", "value", "precision"],
        outputs: &["result"],
    },
    BuiltinNodeSpec {
        kind: BUILTIN_IDENTITY_KIND,
        summary: "Pass inputs through unchanged.",
        operations: &["run"],
        inputs: &["*"],
        outputs: &["*"],
    },
    BuiltinNodeSpec {
        kind: BUILTIN_EMIT_SUBFLOW_OUTPUT_KIND,
        summary: "Publish values into the subflow output namespace.",
        operations: &["run"],
        inputs: &["*"],
        outputs: &["subflow_output:*"],
    },
    BuiltinNodeSpec {
        kind: BUILTIN_SCRIPT_KIND,
        summary: "Execute a Python or JavaScript worker script from the workflow package.",
        operations: &["python:<relative_path>", "javascript:<relative_path>"],
        inputs: &["payload:*"],
        outputs: &["result", "object_fields:*"],
    },
    BuiltinNodeSpec {
        kind: BUILTIN_HTTP_KIND,
        summary: "Send an HTTP request and capture status, body, URL, and headers.",
        operations: &["<url>"],
        inputs: &["method", "headers", "body"],
        outputs: &["status", "ok", "url", "body", "headers"],
    },
];

pub fn builtin_node_specs() -> &'static [BuiltinNodeSpec] {
    BUILTIN_NODE_SPECS
}
