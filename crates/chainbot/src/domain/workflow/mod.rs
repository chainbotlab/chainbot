//! [INPUT]
//! Workflow package manifests, runtime variable declarations, conditional predicates, and subflow boundary mappings.
//!
//! [OUTPUT]
//! Exposes pure workflow semantics split into contract, variable, when, and subflow submodules.
//!
//! [ROLE]
//! Owns workflow domain semantics independent from app orchestration and infrastructure loading.

pub mod contract;
pub mod subflow;
pub mod variables;
pub mod when;

pub use contract::{DependsMode, WorkflowDefinition, CURRENT_API_MAJOR};
pub use subflow::{SubflowContract, SubflowExport, SubflowImport};
pub use variables::{
    ResolvedRuntimeVariable, RuntimeVariableLayers, RuntimeVariableNamespace,
    RuntimeVariableNamespaces, RuntimeVariableSource, VariableBinding, VariableReference,
};
pub use when::{WhenCondition, WhenOperator};
