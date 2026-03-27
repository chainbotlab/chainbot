//! [INPUT]
//! Individual builtin trigger emitter modules for cron, manual, market-tick, webhook, and websocket sources.
//!
//! [OUTPUT]
//! Exposes the builtin trigger emitter module tree used by registry assembly.
//!
//! [ROLE]
//! Defines the module boundary for builtin trigger emitter implementations.

pub mod cron;
pub mod manual;
pub mod market_tick;
pub mod webhook;
pub mod websocket;
