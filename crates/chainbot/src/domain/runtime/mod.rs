//! [INPUT]
//! Workflow graph node definitions, run identifiers, runtime namespace snapshots, and contract validation helpers.
//!
//! [OUTPUT]
//! Backend-agnostic execution contract models for run requests, scheduler state, and run reports.
//!
//! [ROLE]
//! Owns domain runtime execution contracts independent from app-level orchestration.

pub mod contract;
pub mod report;

pub use contract::{
    DependsMode, NodeDefinition, NormalizedRunRequest, RuntimeVariableLayers,
    RuntimeVariableNamespace, RuntimeVariableNamespaces, RuntimeVariableSource,
    ScheduledNodeState, SubflowContract, SubflowExport, SubflowImport, VariableBinding,
    VariableReference, WhenCondition, WhenOperator, WorkflowDefinition, CURRENT_API_MAJOR,
};
pub use report::{WorkflowRunReport, WorkflowRunStatus};
