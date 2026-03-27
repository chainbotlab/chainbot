//! [INPUT]
//! Current wall-clock time used while evaluating builtin trigger emitters.
//!
//! [OUTPUT]
//! Defines the shared evaluation context passed into builtin trigger handlers.
//!
//! [ROLE]
//! Carries shared runtime inputs for builtin trigger emission.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuiltinTriggerContext {
    pub now_ms: i64,
}
