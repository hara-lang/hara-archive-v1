//! Trusted host binding for opaque instrumentation target identities.
//!
//! Session products such as `LiveSession` expose only a target id and
//! generation in their bounded reply payloads. This module lets the embedding
//! host turn that identity back into a fenced native handle without receiving
//! the evaluator, bytecode machine, or the Runtime-owned hub.

use super::model::TargetHandle;
use super::native::{NativeInstrumentation, NativeInstrumentationError, NativeTargetHandle};

impl NativeInstrumentation {
    /// Binds one opaque target identity obtained from a trusted Session surface.
    ///
    /// The generation is mandatory: a stale identity must not bind a
    /// replacement target that later reuses the same id. Resolution still
    /// applies the service's Runtime and Session fences before returning an
    /// opaque native target handle.
    pub fn bind_target_identity(
        &self,
        target_id: impl Into<String>,
        generation: u64,
    ) -> Result<NativeTargetHandle, NativeInstrumentationError> {
        self.bind_target(&TargetHandle::new(target_id.into(), generation))
    }
}

#[cfg(test)]
#[path = "native_identity/tests.rs"]
mod tests;

#[cfg(all(test, feature = "whole-wasm", not(target_arch = "wasm32")))]
#[path = "native_identity/whole_wasm_tests.rs"]
mod whole_wasm_tests;
