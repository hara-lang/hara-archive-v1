//! Backend-neutral live execution sessions.
//!
//! The interpreter and observed HBC machine keep their existing evaluator and
//! evidence APIs. This module owns the common host-facing identity, lifecycle,
//! capability, replacement, and stale-request contract and adapts each backend
//! without treating a [`crate::Sandbox`] as an evaluator.

#[path = "live_session/model.rs"]
mod model;
pub(crate) use model::required_text;
pub use model::{
    LiveBackend, LiveReplacementPolicy, LiveSession, LiveSessionCapabilities, LiveSessionCommand,
    LiveSessionError, LiveSessionOperation, LiveSessionReply, LiveSessionRequest, LiveSessionState,
    LiveSessionStatus, LiveSettlement, LiveSource, LIVE_SESSION_CAPABILITIES_SCHEMA,
    LIVE_SESSION_PROTOCOL, LIVE_SESSION_REPLY_SCHEMA, LIVE_SESSION_STATE_SCHEMA,
};

#[path = "live_session/interpreter.rs"]
mod interpreter;
pub use interpreter::InterpreterLiveSession;

#[path = "live_session/instrumented_interpreter.rs"]
mod instrumented_interpreter;
pub(crate) use instrumented_interpreter::InstrumentedInterpreterLiveSession;

#[cfg(feature = "bytecode-observation")]
#[path = "live_session/bytecode.rs"]
mod bytecode;
#[cfg(feature = "bytecode-observation")]
pub use bytecode::BytecodeLiveSession;

#[cfg(all(feature = "bytecode-observation", feature = "bytecode-instrumentation"))]
#[path = "live_session/instrumented_hbc.rs"]
mod instrumented_hbc;
#[cfg(all(feature = "bytecode-observation", feature = "bytecode-instrumentation"))]
pub(crate) use instrumented_hbc::InstrumentedHbcLiveSession;

#[cfg(all(feature = "whole-wasm", not(target_arch = "wasm32")))]
#[path = "live_session/whole_wasm.rs"]
mod whole_wasm;
#[cfg(all(feature = "whole-wasm", not(target_arch = "wasm32")))]
pub(crate) use whole_wasm::WholeWasmLiveSession;

#[cfg(test)]
#[path = "live_session/tests.rs"]
mod tests;
