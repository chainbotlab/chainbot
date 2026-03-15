/*
[INPUT]:  Contract module declarations and shared runtime boundaries.
[OUTPUT]: Exposes stable V2 module boundaries for config/protocol contracts, scheduler execution-plane APIs, and worker host contracts.
[POS]:    Library surface for chainbot module boundaries and validation/runtime types.
[UPDATE]: 2026-03-16 - Add frozen V2 module map for MVP foundations.
[UPDATE]: 2026-03-16 - Extend worker module usage to include subprocess host boundary types.
[UPDATE]: 2026-03-16 - Keep trigger-plane runtime APIs exported for integration coverage and serve wiring.
[UPDATE]: 2026-03-16 - Keep executor scheduling and builtin-node registry contracts exported for task-7 integration tests.
*/

pub mod cli;
pub mod config;
pub mod errors;
pub mod executor;
pub mod plugin;
pub mod secrets;
pub mod state;
pub mod trigger;
pub mod worker;
pub mod workflow;
