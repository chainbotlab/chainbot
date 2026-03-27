//! [INPUT]
//! Trigger wasm transport callbacks, host push outcomes, and persistent-session runtime seams.
//!
//! [OUTPUT]
//! Defines guest-driven push callback contracts and typed host outcomes for durable ACK, backpressure, lease loss, and shutdown.
//!
//! [ROLE]
//! Owns the wasm trigger transport ABI surface without duplicating manifest business schema.

#[cfg(test)]
const ROOT_TRIGGER_PLUGIN_WIT: &str = include_str!("../../../../../../wit/trigger-plugin.wit");
#[cfg(test)]
const CRATE_TRIGGER_PLUGIN_WIT: &str = include_str!("../../../../wit/trigger-plugin.wit");

#[cfg(test)]
use std::sync::{Arc, Weak};

use std::path::Path;

use wasmtime::{
    Caller, Engine, Extern, ExternType, Func, Instance, Module, Store, StoreLimits,
    StoreLimitsBuilder, TypedFunc, ValType,
};

const MINIMAL_WASM_MODULE_BYTES: &[u8] = b"\0asm\x01\0\0\0";
const GUEST_RUN_SESSION_EXPORT_NAMES: [&str; 2] = ["run_session", "run-session"];
const HOST_PUSH_IMPORT_MODULE_NAMES: [&str; 1] = ["trigger-host"];
const HOST_PUSH_IMPORT_FUNC_NAMES: [&str; 2] = ["push-trigger-event", "push_trigger_event"];
const GUEST_CALLBACK_OUTCOME_DURABLE_ACK: i32 = 0;
const GUEST_CALLBACK_OUTCOME_RETRYABLE_BACKPRESSURE: i32 = 1;
const GUEST_CALLBACK_OUTCOME_TERMINAL_LEASE_LOST: i32 = 2;
const GUEST_CALLBACK_OUTCOME_TERMINAL_SHUTTING_DOWN: i32 = 3;
const MAX_GUEST_TURN_ENVELOPE_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WasmGuestTransportEnvelope {
    pub event_key: String,
    pub occurred_at_ms: i64,
    #[serde(default)]
    pub checkpoint: Option<String>,
    #[serde(default)]
    pub payload: serde_json::Value,
    #[serde(default)]
    pub dedup_key: Option<String>,
    #[serde(default)]
    pub dedup_window_ms: Option<i64>,
    #[serde(default)]
    pub cooldown_key: Option<String>,
    #[serde(default)]
    pub cooldown_ms: Option<i64>,
}

#[derive(Debug)]
pub enum WasmGuestExecutionError {
    GuestCallFailed(String),
}

impl std::fmt::Display for WasmGuestExecutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::GuestCallFailed(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for WasmGuestExecutionError {}

#[derive(Debug)]
struct WasmStoreState {
    turn_count: u64,
    store_limits: StoreLimits,
    active_push_context: Option<*mut ()>,
    active_push_dispatch: Option<unsafe fn(*mut (), &[u8]) -> HostPushResult>,
    callback_invoked: bool,
    last_callback_outcome: Option<HostPushOutcome>,
    #[cfg(test)]
    runtime_guard: Arc<()>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WasmStoreLimiterConfig {
    pub memory_size_bytes: usize,
    pub table_elements: usize,
    pub instances: usize,
    pub tables: usize,
    pub memories: usize,
    pub trap_on_grow_failure: bool,
}

impl Default for WasmStoreLimiterConfig {
    fn default() -> Self {
        Self {
            memory_size_bytes: 16 * 1024 * 1024,
            table_elements: 16_384,
            instances: 64,
            tables: 64,
            memories: 64,
            trap_on_grow_failure: true,
        }
    }
}

impl WasmStoreLimiterConfig {
    fn build_store_limits(self) -> StoreLimits {
        StoreLimitsBuilder::new()
            .memory_size(self.memory_size_bytes)
            .table_elements(self.table_elements)
            .instances(self.instances)
            .tables(self.tables)
            .memories(self.memories)
            .trap_on_grow_failure(self.trap_on_grow_failure)
            .build()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WasmTriggerSessionConfig {
    pub store_limiter: WasmStoreLimiterConfig,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WasmRuntimeHandles {
    pub engine_ptr: usize,
    pub module_ptr: usize,
    pub store_ptr: usize,
    pub instance_ptr: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WasmGuestState {
    pub turn_count: u64,
    pub last_turn_at_ms: Option<i64>,
    pub last_reconciled_at_ms: i64,
}

#[derive(Debug)]
pub struct WasmTriggerSession {
    #[cfg(test)]
    trigger_id: String,
    #[cfg(test)]
    plugin_id: String,
    component: String,
    #[cfg(test)]
    engine: Engine,
    #[cfg(test)]
    module: Module,
    store: Store<WasmStoreState>,
    instance: Instance,
    guest_state: WasmGuestState,
    #[cfg(test)]
    started_at_ms: i64,
    #[cfg(test)]
    config: WasmTriggerSessionConfig,
}

impl WasmTriggerSession {
    #[cfg(test)]
    pub fn new(
        trigger_id: impl Into<String>,
        plugin_id: impl Into<String>,
        component: impl Into<String>,
        started_at_ms: i64,
    ) -> Self {
        Self::new_with_config(
            trigger_id,
            plugin_id,
            component,
            started_at_ms,
            WasmTriggerSessionConfig::default(),
        )
    }

    pub fn new_with_config(
        trigger_id: impl Into<String>,
        plugin_id: impl Into<String>,
        component: impl Into<String>,
        started_at_ms: i64,
        config: WasmTriggerSessionConfig,
    ) -> Self {
        #[cfg(not(test))]
        let _ = (trigger_id, plugin_id);

        let component = component.into();
        let engine = Engine::default();
        let module = load_session_module(&engine, &component);
        let store_limits = config.store_limiter.build_store_limits();
        let mut store = Store::new(
            &engine,
            WasmStoreState {
                turn_count: 0,
                store_limits,
                active_push_context: None,
                active_push_dispatch: None,
                callback_invoked: false,
                last_callback_outcome: None,
                #[cfg(test)]
                runtime_guard: Arc::new(()),
            },
        );
        store.limiter(|state| &mut state.store_limits);
        let instance = instantiate_session_instance(&module, &mut store);

        Self {
            #[cfg(test)]
            trigger_id: trigger_id.into(),
            #[cfg(test)]
            plugin_id: plugin_id.into(),
            component,
            #[cfg(test)]
            engine,
            #[cfg(test)]
            module,
            store,
            instance,
            guest_state: WasmGuestState {
                turn_count: 0,
                last_turn_at_ms: None,
                last_reconciled_at_ms: started_at_ms,
            },
            #[cfg(test)]
            started_at_ms,
            #[cfg(test)]
            config,
        }
    }

    #[cfg(test)]
    pub fn trigger_id(&self) -> &str {
        &self.trigger_id
    }

    #[cfg(test)]
    pub fn plugin_id(&self) -> &str {
        &self.plugin_id
    }

    pub fn component(&self) -> &str {
        &self.component
    }

    #[cfg(test)]
    pub fn started_at_ms(&self) -> i64 {
        self.started_at_ms
    }

    #[cfg(test)]
    pub fn config(&self) -> WasmTriggerSessionConfig {
        self.config
    }

    #[cfg(test)]
    pub fn runtime_handles(&self) -> WasmRuntimeHandles {
        WasmRuntimeHandles {
            engine_ptr: &self.engine as *const Engine as usize,
            module_ptr: &self.module as *const Module as usize,
            store_ptr: &self.store as *const Store<WasmStoreState> as usize,
            instance_ptr: &self.instance as *const Instance as usize,
        }
    }

    #[cfg(test)]
    pub fn store_turn_count(&self) -> u64 {
        self.store.data().turn_count
    }

    #[cfg(test)]
    pub fn runtime_drop_probe(&self) -> Weak<()> {
        Arc::downgrade(&self.store.data().runtime_guard)
    }

    #[cfg(test)]
    pub fn guest_state(&self) -> &WasmGuestState {
        &self.guest_state
    }

    pub fn mark_reconciled(&mut self, at_ms: i64) {
        self.guest_state.last_reconciled_at_ms = at_ms;
    }

    pub fn begin_turn(&mut self, at_ms: i64) {
        self.store.data_mut().turn_count = self.store.data().turn_count.saturating_add(1);
        let _ = self.instance.exports(&mut self.store).count();
        self.guest_state.turn_count = self.guest_state.turn_count.saturating_add(1);
        self.guest_state.last_turn_at_ms = Some(at_ms);
    }

    pub fn execute_guest_turn(
        &mut self,
        at_ms: i64,
        mut host: &mut dyn TriggerPushHost,
    ) -> Result<HostPushOutcome, WasmGuestExecutionError> {
        self.begin_turn(at_ms);

        {
            let state = self.store.data_mut();
            state.active_push_context =
                Some((&mut host as *mut &mut dyn TriggerPushHost).cast::<()>());
            state.active_push_dispatch = Some(dispatch_push_to_trigger_host);
            state.callback_invoked = false;
            state.last_callback_outcome = None;
        }

        let run_session_result = self.execute_guest_run_session();

        let (callback_invoked, callback_outcome) = {
            let state = self.store.data_mut();
            state.active_push_context = None;
            state.active_push_dispatch = None;
            let callback_invoked = state.callback_invoked;
            let callback_outcome = state.last_callback_outcome;
            state.callback_invoked = false;
            state.last_callback_outcome = None;
            (callback_invoked, callback_outcome)
        };

        run_session_result.map_err(|source| {
            WasmGuestExecutionError::GuestCallFailed(format!(
                "failed to execute guest run-session export: {source}"
            ))
        })?;

        if !callback_invoked {
            return Err(WasmGuestExecutionError::GuestCallFailed(
                "guest run-session did not invoke trigger-host.push-trigger-event".to_owned(),
            ));
        }

        Ok(callback_outcome.unwrap_or(HostPushOutcome::TerminalShuttingDown))
    }

    fn execute_guest_run_session(&mut self) -> Result<(), String> {
        let Some(run_session_export) = self
            .lookup_guest_unit_export(&GUEST_RUN_SESSION_EXPORT_NAMES)
            .map_err(|source| format!("failed to type guest run-session export: {source}"))?
        else {
            return Ok(());
        };

        run_session_export
            .call(&mut self.store, ())
            .map_err(|source| format!("failed to call guest run-session export: {source}"))
    }

    fn lookup_guest_unit_export(
        &mut self,
        export_names: &[&str],
    ) -> Result<Option<TypedFunc<(), ()>>, wasmtime::Error> {
        for export_name in export_names {
            if let Some(func) = self.instance.get_func(&mut self.store, export_name) {
                let typed_func = func.typed::<(), ()>(&self.store)?;
                return Ok(Some(typed_func));
            }
        }
        Ok(None)
    }
}

fn load_session_module(engine: &Engine, component: &str) -> Module {
    let module_path = component.trim();
    if module_path.is_empty() {
        return Module::new(engine, MINIMAL_WASM_MODULE_BYTES)
            .expect("minimal wasm module should compile with default engine");
    }

    let candidate = Path::new(module_path);
    if !candidate.is_file() {
        return Module::new(engine, MINIMAL_WASM_MODULE_BYTES)
            .expect("minimal wasm module should compile with default engine");
    }

    Module::from_file(engine, candidate).unwrap_or_else(|source| {
        panic!("configured wasm module should compile and instantiate: {module_path}: {source}")
    })
}

fn instantiate_session_instance(module: &Module, store: &mut Store<WasmStoreState>) -> Instance {
    let imports = build_session_imports(module, store);
    Instance::new(store, module, &imports)
        .expect("configured wasm module should instantiate with supported imports")
}

fn build_session_imports(module: &Module, store: &mut Store<WasmStoreState>) -> Vec<Extern> {
    let mut imports = Vec::new();
    for import in module.imports() {
        let import_module = import.module();
        let import_name = import.name();
        match import.ty() {
            ExternType::Func(function_type)
                if is_push_trigger_event_import(import_module, import_name) =>
            {
                let mut params = function_type.params();
                let params_match = matches!(params.next(), Some(ValType::I32))
                    && matches!(params.next(), Some(ValType::I32))
                    && params.next().is_none();
                let mut results = function_type.results();
                let results_match =
                    matches!(results.next(), Some(ValType::I32)) && results.next().is_none();
                if !params_match || !results_match {
                    panic!(
                        "push-trigger-event import must be (i32, i32) -> i32, got module={import_module} name={import_name}"
                    );
                }
                imports.push(Func::wrap(&mut *store, invoke_push_trigger_event_import).into());
            }
            ExternType::Func(_) => {
                panic!(
                    "unsupported wasm import function module={import_module} name={import_name}; only trigger-host.push-trigger-event is supported"
                );
            }
            other => {
                panic!(
                    "unsupported wasm import kind for module={import_module} name={import_name}: {other:?}"
                );
            }
        }
    }
    imports
}

fn is_push_trigger_event_import(module: &str, name: &str) -> bool {
    HOST_PUSH_IMPORT_MODULE_NAMES.contains(&module) && HOST_PUSH_IMPORT_FUNC_NAMES.contains(&name)
}

fn invoke_push_trigger_event_import(
    mut caller: Caller<'_, WasmStoreState>,
    ptr: i32,
    len: i32,
) -> i32 {
    let outcome = read_guest_callback_event_bytes(&mut caller, ptr, len)
        .map(|event_bytes| dispatch_guest_callback_event(&mut caller, &event_bytes))
        .unwrap_or(HostPushOutcome::TerminalShuttingDown);

    {
        let state = caller.data_mut();
        state.callback_invoked = true;
        state.last_callback_outcome = Some(outcome);
    }

    map_host_push_outcome_to_guest_status_code(outcome)
}

fn read_guest_callback_event_bytes(
    caller: &mut Caller<'_, WasmStoreState>,
    ptr: i32,
    len: i32,
) -> Result<Vec<u8>, String> {
    if ptr < 0 {
        return Err(format!("guest callback pointer cannot be negative: {ptr}"));
    }
    if len <= 0 {
        return Err(format!(
            "guest callback payload length must be greater than zero: {len}"
        ));
    }

    let ptr = usize::try_from(ptr)
        .map_err(|source| format!("guest callback pointer overflowed usize: {source}"))?;
    let len = usize::try_from(len)
        .map_err(|source| format!("guest callback payload length overflowed usize: {source}"))?;
    if len > MAX_GUEST_TURN_ENVELOPE_BYTES {
        return Err(format!(
            "guest callback payload length {len} exceeds maximum {}",
            MAX_GUEST_TURN_ENVELOPE_BYTES
        ));
    }

    let memory = caller
        .get_export("memory")
        .and_then(|export| export.into_memory())
        .ok_or_else(|| {
            "guest callback import requires exported memory named `memory`".to_owned()
        })?;

    let mut bytes = vec![0_u8; len];
    memory
        .read(&*caller, ptr, &mut bytes)
        .map_err(|source| format!("failed to read guest callback payload bytes: {source}"))?;
    Ok(bytes)
}

fn dispatch_guest_callback_event(
    caller: &mut Caller<'_, WasmStoreState>,
    event_bytes: &[u8],
) -> HostPushOutcome {
    let (active_push_context, active_push_dispatch) = {
        let state = caller.data();
        (state.active_push_context, state.active_push_dispatch)
    };
    let Some(active_push_context) = active_push_context else {
        return HostPushOutcome::TerminalShuttingDown;
    };
    let Some(active_push_dispatch) = active_push_dispatch else {
        return HostPushOutcome::TerminalShuttingDown;
    };

    let push_result = unsafe { active_push_dispatch(active_push_context, event_bytes) };
    classify_host_push_result(push_result)
}

unsafe fn dispatch_push_to_trigger_host(context: *mut (), event_bytes: &[u8]) -> HostPushResult {
    let host = unsafe { &mut *context.cast::<&mut dyn TriggerPushHost>() };
    host.push_trigger_event(event_bytes)
}

const fn map_host_push_outcome_to_guest_status_code(outcome: HostPushOutcome) -> i32 {
    match outcome {
        HostPushOutcome::DurableAck => GUEST_CALLBACK_OUTCOME_DURABLE_ACK,
        HostPushOutcome::RetryableBackpressure => GUEST_CALLBACK_OUTCOME_RETRYABLE_BACKPRESSURE,
        HostPushOutcome::TerminalLeaseLost => GUEST_CALLBACK_OUTCOME_TERMINAL_LEASE_LOST,
        HostPushOutcome::TerminalShuttingDown => GUEST_CALLBACK_OUTCOME_TERMINAL_SHUTTING_DOWN,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostPushSuccess {
    DurableAck,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostPushError {
    Backpressure,
    #[cfg(test)]
    LeaseLost,
    ShuttingDown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostPushErrorSource {
    Backpressure,
    #[cfg(test)]
    LeaseLost,
    ShuttingDown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostPushOutcome {
    DurableAck,
    RetryableBackpressure,
    TerminalLeaseLost,
    TerminalShuttingDown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostPushControlFlowSource {
    #[cfg(test)]
    QueueSaturated,
    BudgetExhausted,
    DaemonShuttingDown,
    LeaseLost,
}

pub type HostPushResult = Result<HostPushSuccess, HostPushError>;

pub trait TriggerPushHost {
    fn push_trigger_event(&mut self, event_bytes: &[u8]) -> HostPushResult;
}

impl HostPushError {
    #[cfg(test)]
    pub const fn is_retryable(self) -> bool {
        matches!(self, Self::Backpressure)
    }

    #[cfg(test)]
    pub const fn is_terminal(self) -> bool {
        !self.is_retryable()
    }
}

impl From<HostPushErrorSource> for HostPushError {
    fn from(source: HostPushErrorSource) -> Self {
        match source {
            HostPushErrorSource::Backpressure => Self::Backpressure,
            #[cfg(test)]
            HostPushErrorSource::LeaseLost => Self::LeaseLost,
            HostPushErrorSource::ShuttingDown => Self::ShuttingDown,
        }
    }
}

pub fn classify_host_push_result(result: HostPushResult) -> HostPushOutcome {
    match result {
        Ok(HostPushSuccess::DurableAck) => HostPushOutcome::DurableAck,
        Err(HostPushError::Backpressure) => HostPushOutcome::RetryableBackpressure,
        #[cfg(test)]
        Err(HostPushError::LeaseLost) => HostPushOutcome::TerminalLeaseLost,
        Err(HostPushError::ShuttingDown) => HostPushOutcome::TerminalShuttingDown,
    }
}

pub const fn map_control_flow_source_to_push_outcome(
    source: HostPushControlFlowSource,
) -> HostPushOutcome {
    match source {
        #[cfg(test)]
        HostPushControlFlowSource::QueueSaturated | HostPushControlFlowSource::BudgetExhausted => {
            HostPushOutcome::RetryableBackpressure
        }
        #[cfg(not(test))]
        HostPushControlFlowSource::BudgetExhausted => HostPushOutcome::RetryableBackpressure,
        HostPushControlFlowSource::DaemonShuttingDown => HostPushOutcome::TerminalShuttingDown,
        HostPushControlFlowSource::LeaseLost => HostPushOutcome::TerminalLeaseLost,
    }
}

pub fn host_push_result_from_staged_append<E>(
    append_result: Result<bool, E>,
    map_error: impl FnOnce(&E) -> HostPushErrorSource,
) -> HostPushResult {
    match append_result {
        Ok(true) => Ok(HostPushSuccess::DurableAck),
        Ok(false) => Err(HostPushError::Backpressure),
        Err(error) => Err(HostPushError::from(map_error(&error))),
    }
}

#[cfg(test)]
pub fn push_event_with_host_callback(
    host: &mut dyn TriggerPushHost,
    event_bytes: &[u8],
) -> HostPushOutcome {
    classify_host_push_result(host.push_trigger_event(event_bytes))
}

#[cfg(test)]
pub fn trigger_plugin_wit_sources() -> (&'static str, &'static str) {
    (ROOT_TRIGGER_PLUGIN_WIT, CRATE_TRIGGER_PLUGIN_WIT)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST_OWNED_BUSINESS_SCHEMA_TOKENS: &[&str] = &[
        "trigger_id",
        "workflow_id",
        "params",
        "input_mapping",
        "event_schema",
    ];

    #[test]
    fn trigger_wasm_wit_sources_stay_in_sync() {
        let (root_copy, crate_copy) = trigger_plugin_wit_sources();
        assert_eq!(root_copy, crate_copy);
    }

    #[test]
    fn trigger_wasm_wit_stays_transport_only() {
        let (_root_copy, crate_copy) = trigger_plugin_wit_sources();

        assert!(crate_copy.contains("push-trigger-event"));
        assert!(!crate_copy.contains("start:"));
        assert!(!crate_copy.contains("drain:"));
        assert!(!crate_copy.contains("stop:"));
        for token in MANIFEST_OWNED_BUSINESS_SCHEMA_TOKENS {
            assert!(
                !crate_copy.contains(token),
                "WIT transport must not duplicate business schema token: {token}"
            );
        }
    }

    #[test]
    fn trigger_wasm_classifies_durable_ack_as_success() {
        let outcome = classify_host_push_result(Ok(HostPushSuccess::DurableAck));
        assert_eq!(outcome, HostPushOutcome::DurableAck);
    }

    #[test]
    fn trigger_wasm_classifies_backpressure_as_retryable() {
        let outcome = classify_host_push_result(Err(HostPushError::Backpressure));
        assert_eq!(outcome, HostPushOutcome::RetryableBackpressure);
        assert!(HostPushError::Backpressure.is_retryable());
        assert!(!HostPushError::Backpressure.is_terminal());
    }

    #[test]
    fn trigger_wasm_classifies_lease_lost_as_terminal() {
        let outcome = classify_host_push_result(Err(HostPushError::LeaseLost));
        assert_eq!(outcome, HostPushOutcome::TerminalLeaseLost);
        assert!(HostPushError::LeaseLost.is_terminal());
        assert!(!HostPushError::LeaseLost.is_retryable());
    }

    #[test]
    fn trigger_wasm_classifies_shutting_down_as_terminal() {
        let outcome = classify_host_push_result(Err(HostPushError::ShuttingDown));
        assert_eq!(outcome, HostPushOutcome::TerminalShuttingDown);
        assert!(HostPushError::ShuttingDown.is_terminal());
        assert!(!HostPushError::ShuttingDown.is_retryable());
    }

    #[test]
    fn trigger_wasm_guest_push_uses_host_callback() {
        #[derive(Debug)]
        struct StubHost {
            next_result: HostPushResult,
            received: Vec<Vec<u8>>,
        }

        impl TriggerPushHost for StubHost {
            fn push_trigger_event(&mut self, event_bytes: &[u8]) -> HostPushResult {
                self.received.push(event_bytes.to_vec());
                self.next_result
            }
        }

        let mut host = StubHost {
            next_result: Err(HostPushError::Backpressure),
            received: Vec::new(),
        };

        let outcome = push_event_with_host_callback(&mut host, b"transport-envelope");
        assert_eq!(outcome, HostPushOutcome::RetryableBackpressure);
        assert_eq!(host.received, vec![b"transport-envelope".to_vec()]);
    }

    #[test]
    fn staged_append_success_maps_to_durable_ack() {
        let result = host_push_result_from_staged_append::<()>(Ok(true), |_| {
            HostPushErrorSource::Backpressure
        });
        assert_eq!(result, Ok(HostPushSuccess::DurableAck));
    }

    #[test]
    fn staged_append_noop_maps_to_retryable_backpressure() {
        let result = host_push_result_from_staged_append::<()>(Ok(false), |_| {
            HostPushErrorSource::LeaseLost
        });
        assert_eq!(result, Err(HostPushError::Backpressure));
    }

    #[test]
    fn staged_append_error_maps_to_retryable_backpressure() {
        let result = host_push_result_from_staged_append::<&str>(Err("insert failed"), |_| {
            HostPushErrorSource::Backpressure
        });
        assert_eq!(result, Err(HostPushError::Backpressure));
    }

    #[test]
    fn staged_append_error_maps_to_terminal_lease_lost_when_source_requires_it() {
        let result = host_push_result_from_staged_append::<&str>(Err("lease expired"), |_| {
            HostPushErrorSource::LeaseLost
        });
        assert_eq!(result, Err(HostPushError::LeaseLost));
    }

    #[test]
    fn staged_append_error_maps_to_terminal_shutting_down_when_source_requires_it() {
        let result = host_push_result_from_staged_append::<&str>(Err("daemon stopping"), |_| {
            HostPushErrorSource::ShuttingDown
        });
        assert_eq!(result, Err(HostPushError::ShuttingDown));
    }

    #[test]
    fn control_flow_queue_saturated_maps_to_retryable_backpressure() {
        let outcome =
            map_control_flow_source_to_push_outcome(HostPushControlFlowSource::QueueSaturated);
        assert_eq!(outcome, HostPushOutcome::RetryableBackpressure);
    }

    #[test]
    fn control_flow_budget_exhausted_maps_to_retryable_backpressure() {
        let outcome =
            map_control_flow_source_to_push_outcome(HostPushControlFlowSource::BudgetExhausted);
        assert_eq!(outcome, HostPushOutcome::RetryableBackpressure);
    }

    #[test]
    fn control_flow_daemon_shutting_down_maps_to_terminal_shutting_down() {
        let outcome =
            map_control_flow_source_to_push_outcome(HostPushControlFlowSource::DaemonShuttingDown);
        assert_eq!(outcome, HostPushOutcome::TerminalShuttingDown);
    }

    #[test]
    fn control_flow_lease_lost_maps_to_terminal_lease_lost() {
        let outcome = map_control_flow_source_to_push_outcome(HostPushControlFlowSource::LeaseLost);
        assert_eq!(outcome, HostPushOutcome::TerminalLeaseLost);
    }

    #[test]
    fn wasm_trigger_session_reuses_real_runtime_across_turns() {
        let mut session =
            WasmTriggerSession::new("tr-wasm", "plugin-wasm", "trigger_component", 100);

        let first_handles = session.runtime_handles();

        session.begin_turn(110);
        session.mark_reconciled(120);
        session.begin_turn(130);

        assert_eq!(session.trigger_id(), "tr-wasm");
        assert_eq!(session.plugin_id(), "plugin-wasm");
        assert_eq!(session.component(), "trigger_component");
        assert_eq!(session.started_at_ms(), 100);
        assert_eq!(session.runtime_handles(), first_handles);
        assert_eq!(session.store_turn_count(), 2);
        assert_eq!(session.guest_state().turn_count, 2);
        assert_eq!(session.guest_state().last_turn_at_ms, Some(130));
        assert_eq!(session.guest_state().last_reconciled_at_ms, 120);
    }

    #[test]
    fn wasm_trigger_session_drop_releases_real_runtime_store() {
        let runtime_probe = {
            let session =
                WasmTriggerSession::new("tr-wasm", "plugin-wasm", "trigger_component", 100);
            session.runtime_drop_probe()
        };

        assert!(runtime_probe.upgrade().is_none());
    }

    #[test]
    fn wasm_trigger_session_uses_explicit_store_limiter_config() {
        let config = WasmTriggerSessionConfig {
            store_limiter: WasmStoreLimiterConfig {
                memory_size_bytes: 64 * 1024,
                table_elements: 1,
                instances: 1,
                tables: 1,
                memories: 1,
                trap_on_grow_failure: true,
            },
        };
        let mut session = WasmTriggerSession::new_with_config(
            "tr-wasm",
            "plugin-wasm",
            "trigger_component",
            100,
            config,
        );

        assert_eq!(session.config(), config);
        assert!(
            Instance::new(&mut session.store, &session.module, &[]).is_err(),
            "configured instance limiter should bound long-lived store growth"
        );
    }

    #[test]
    fn wasm_trigger_session_executes_guest_callback_import_from_module_artifact() {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos();
        let module_path = std::env::temp_dir().join(format!(
            "chainbot-wasm-session-guest-callback-{timestamp}.wat"
        ));

        let guest_envelope = serde_json::json!({
            "event_key": "guest-callback-event",
            "occurred_at_ms": 1_710_600_123_456_i64,
            "checkpoint": "cp-guest-callback",
            "payload": {
                "symbol": "SOLUSDT",
                "price": 204.75,
                "producer": "guest"
            },
            "dedup_key": "dedup-guest-callback",
            "dedup_window_ms": 30_000,
            "cooldown_key": "cooldown-guest-callback",
            "cooldown_ms": 5_000
        });
        let guest_envelope_json = serde_json::to_string(&guest_envelope)
            .expect("guest callback envelope should serialize");
        std::fs::write(
            &module_path,
            render_guest_callback_session_module_wat(&guest_envelope_json),
        )
        .expect("guest callback wasm module should be writable");

        let mut session = WasmTriggerSession::new(
            "tr-wasm",
            "plugin-wasm",
            module_path.to_string_lossy().into_owned(),
            100,
        );

        #[derive(Debug)]
        struct RecordingHost {
            received: Vec<Vec<u8>>,
        }

        impl TriggerPushHost for RecordingHost {
            fn push_trigger_event(&mut self, event_bytes: &[u8]) -> HostPushResult {
                self.received.push(event_bytes.to_vec());
                Ok(HostPushSuccess::DurableAck)
            }
        }

        let mut host = RecordingHost {
            received: Vec::new(),
        };
        let outcome = session
            .execute_guest_turn(110, &mut host)
            .expect("guest turn should execute with callback import");
        assert_eq!(outcome, HostPushOutcome::DurableAck);

        let event_bytes = host
            .received
            .first()
            .expect("host callback should receive one envelope");
        let envelope: WasmGuestTransportEnvelope =
            serde_json::from_slice(event_bytes).expect("guest callback envelope should decode");

        assert_eq!(envelope.event_key, "guest-callback-event");
        assert_eq!(envelope.occurred_at_ms, 1_710_600_123_456_i64);
        assert_eq!(envelope.checkpoint.as_deref(), Some("cp-guest-callback"));
        assert_eq!(envelope.payload["producer"], serde_json::json!("guest"));
        assert_eq!(session.guest_state().turn_count, 1);

        let _ = std::fs::remove_file(module_path);
    }

    #[test]
    fn wasm_trigger_session_requires_guest_callback_invocation() {
        let mut session = WasmTriggerSession::new("tr-wasm", "plugin-wasm", "", 100);

        #[derive(Debug)]
        struct RecordingHost {
            received: Vec<Vec<u8>>,
        }

        impl TriggerPushHost for RecordingHost {
            fn push_trigger_event(&mut self, event_bytes: &[u8]) -> HostPushResult {
                self.received.push(event_bytes.to_vec());
                Ok(HostPushSuccess::DurableAck)
            }
        }

        let mut host = RecordingHost {
            received: Vec::new(),
        };
        let error = session
            .execute_guest_turn(110, &mut host)
            .expect_err("guest turn should fail when no callback import is invoked");

        assert!(matches!(
            error,
            WasmGuestExecutionError::GuestCallFailed(message)
                if message.contains("did not invoke trigger-host.push-trigger-event")
        ));
        assert!(host.received.is_empty());
    }

    fn render_guest_callback_session_module_wat(guest_envelope_json: &str) -> String {
        let escaped_bytes = guest_envelope_json
            .as_bytes()
            .iter()
            .map(|byte| format!("\\{:02x}", byte))
            .collect::<String>();
        format!(
            r#"(module
  (import "trigger-host" "push-trigger-event" (func $push-trigger-event (param i32 i32) (result i32)))
  (memory (export "memory") 1)
  (data (i32.const 0) "{escaped_bytes}")
  (func (export "run-session")
    i32.const 0
    i32.const {}
    call $push-trigger-event
    drop
  )
)
"#,
            guest_envelope_json.as_bytes().len()
        )
    }
}
