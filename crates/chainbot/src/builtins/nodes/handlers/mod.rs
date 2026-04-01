//! [INPUT]
//! Individual builtin node handler modules for flow, data, script, and subflow-output behaviors.
//!
//! [OUTPUT]
//! Exposes the builtin node handler module tree used by registry assembly.
//!
//! [ROLE]
//! Defines the module boundary for builtin node handler implementations.

pub mod assert;
pub mod data_coalesce;
pub mod data_compare;
pub mod data_get;
pub mod data_math;
pub mod data_merge;
pub mod data_parse_json;
pub mod data_pick;
pub mod data_stringify_json;
pub mod data_template;
pub mod emit_subflow_output;
pub mod fail;
pub mod identity;
pub mod script;
