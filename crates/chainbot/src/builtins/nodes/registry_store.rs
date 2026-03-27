//! [INPUT]
//! Builtin node handler trait objects, request or result contracts, and contract error semantics.
//!
//! [OUTPUT]
//! Provides a registry type that stores builtin node handlers and dispatches requests by builtin kind.
//!
//! [ROLE]
//! Encapsulates handler registration and lookup for builtin node execution.

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::errors::ContractError;

use super::contract::{BuiltinNodeHandler, BuiltinNodeRequest, BuiltinNodeResult};

#[derive(Clone, Default)]
pub struct BuiltinNodeRegistry {
    handlers: BTreeMap<String, Arc<dyn BuiltinNodeHandler>>,
}

impl std::fmt::Debug for BuiltinNodeRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BuiltinNodeRegistry")
            .field(
                "registered_kinds",
                &self.handlers.keys().collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl BuiltinNodeRegistry {
    pub fn new() -> Self {
        Self {
            handlers: BTreeMap::new(),
        }
    }

    pub fn with_test_handlers() -> Self {
        let mut registry = Self::new();
        registry.register_handler(crate::builtins::nodes::handlers::identity::IdentityHandler);
        registry.register_handler(
            crate::builtins::nodes::handlers::emit_subflow_output::EmitSubflowOutputHandler,
        );
        registry
    }

    pub fn register_handler<H>(&mut self, handler: H)
    where
        H: BuiltinNodeHandler + 'static,
    {
        self.handlers
            .insert(handler.kind().to_owned(), Arc::new(handler));
    }

    pub fn register<F>(&mut self, kind: impl Into<String>, handler: F)
    where
        F: Fn(&BuiltinNodeRequest) -> Result<BuiltinNodeResult, ContractError>
            + Send
            + Sync
            + 'static,
    {
        self.register_handler(ClosureBuiltinNodeHandler::new(kind.into(), handler));
    }

    pub fn dispatch(
        &self,
        kind: &str,
        request: &BuiltinNodeRequest,
    ) -> Result<BuiltinNodeResult, ContractError> {
        let Some(handler) = self.handlers.get(kind) else {
            return Err(ContractError::UnknownBuiltinNodeKind {
                workflow_id: request.workflow_id.clone(),
                node_id: request.node_id.clone(),
                kind: kind.to_owned(),
            });
        };

        handler.handle(request)
    }
}

struct ClosureBuiltinNodeHandler<F> {
    kind: String,
    handler: F,
}

impl<F> ClosureBuiltinNodeHandler<F> {
    fn new(kind: String, handler: F) -> Self {
        Self { kind, handler }
    }
}

impl<F> BuiltinNodeHandler for ClosureBuiltinNodeHandler<F>
where
    F: Fn(&BuiltinNodeRequest) -> Result<BuiltinNodeResult, ContractError> + Send + Sync,
{
    fn kind(&self) -> &str {
        &self.kind
    }

    fn handle(&self, request: &BuiltinNodeRequest) -> Result<BuiltinNodeResult, ContractError> {
        (self.handler)(request)
    }
}
