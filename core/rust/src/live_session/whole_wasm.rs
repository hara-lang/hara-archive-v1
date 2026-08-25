//! Whole-Wasm LiveSession adapter.
//!
//! Whole-Wasm execution is synchronous and prepared. It therefore exposes
//! run/call and lifecycle operations, but never claims VM-style stepping,
//! suspension, or snapshots.

use serde_json::{json, Value as JsonValue};
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;

use crate::core::Value;
use crate::instrumentation::{
    Capability, InstrumentationHub, RuntimeBackend, TargetDescriptor, TargetHandle, TargetKind,
};
use crate::vm::{compile_source, encode_program, FunctionId};
use crate::whole_wasm::{compile_artifact_from_hbc, NativeModule};
use crate::Runtime;

use super::{
    required_text, LiveBackend, LiveReplacementPolicy, LiveSession, LiveSessionCapabilities,
    LiveSessionCommand, LiveSessionError, LiveSessionOperation, LiveSessionState,
    LiveSessionStatus, LiveSource,
};

pub(crate) struct WholeWasmLiveSession {
    session_id: String,
    source: LiveSource,
    artifact: Vec<u8>,
    generation: u64,
    sequence: u64,
    status: LiveSessionStatus,
    pending_source: Option<LiveSource>,
    module: Option<NativeModule>,
    hub: Rc<RefCell<InstrumentationHub>>,
    target_handle: Option<TargetHandle>,
}

impl WholeWasmLiveSession {
    pub(crate) fn start(
        runtime: &Runtime,
        owner_session_id: impl Into<String>,
        session_id: impl Into<String>,
        source: LiveSource,
    ) -> Result<Self, LiveSessionError> {
        let program = compile_source(source.source()).map_err(backend_error)?;
        let hbc = encode_program(&program).map_err(backend_error)?;
        let artifact = compile_artifact_from_hbc(&hbc).map_err(backend_error)?;
        Self::from_artifact(runtime, owner_session_id, session_id, source, artifact)
    }

    pub(crate) fn from_artifact(
        runtime: &Runtime,
        owner_session_id: impl Into<String>,
        session_id: impl Into<String>,
        source: LiveSource,
        artifact: Vec<u8>,
    ) -> Result<Self, LiveSessionError> {
        let owner_session_id = required_text(owner_session_id.into(), "owner session id")?;
        let session_id = required_text(session_id.into(), "session id")?;
        let hub = runtime.instrumentation_handle();
        let target_id = target_id(&owner_session_id, &session_id);
        let target_handle = hub
            .borrow_mut()
            .register_target(TargetDescriptor {
                target_id,
                session_id: owner_session_id.clone(),
                kind: TargetKind::WholeWasm,
                backend: RuntimeBackend::new("rust").expect("Rust is a valid backend id"),
                capabilities: whole_wasm_capabilities(),
            })
            .map_err(instrumentation_error)?;
        let module = match NativeModule::load_with_instrumentation(
            &artifact,
            hub.clone(),
            target_handle.clone(),
        ) {
            Ok(module) => module,
            Err(error) => {
                let _ = hub.borrow_mut().remove_target(&target_handle);
                return Err(backend_error(error));
            }
        };
        Ok(Self {
            session_id,
            source,
            artifact,
            generation: 0,
            sequence: 0,
            status: LiveSessionStatus::Ready,
            pending_source: None,
            module: Some(module),
            hub,
            target_handle: Some(target_handle),
        })
    }

    fn module(&mut self) -> Result<&mut NativeModule, LiveSessionError> {
        self.module.as_mut().ok_or_else(|| {
            LiveSessionError::new(
                "live-session/disposed",
                "whole-Wasm module has been disposed",
            )
        })
    }

    fn result_payload(
        &mut self,
        operation: &str,
        result: Result<Value, String>,
    ) -> Result<JsonValue, LiveSessionError> {
        self.sequence = self.sequence.saturating_add(1);
        let terminal_status = if result.is_ok() { "returned" } else { "failed" };
        if let Some(module) = self.module.as_mut() {
            module
                .emit_terminal(terminal_status)
                .map_err(backend_error)?;
        }
        match result {
            Ok(value) => {
                self.status = LiveSessionStatus::Returned;
                Ok(json!({
                    "operation": operation,
                    "status": self.status.as_str(),
                    "result": value_to_json(&value)?,
                    "sequence": self.sequence,
                    "target": self.target_payload(),
                }))
            }
            Err(error) => {
                self.status = LiveSessionStatus::Failed;
                Err(backend_error(error))
            }
        }
    }

    fn run(&mut self) -> Result<JsonValue, LiveSessionError> {
        let result = self.module()?.call_entry_i64().map(Value::Number);
        self.result_payload("run", result)
    }

    fn call(
        &mut self,
        function: u16,
        arguments: Vec<JsonValue>,
    ) -> Result<JsonValue, LiveSessionError> {
        let arguments = arguments
            .into_iter()
            .map(json_to_i64)
            .collect::<Result<Vec<_>, _>>()?;
        let result = self
            .module()?
            .call_i64(FunctionId::from(function), &arguments)
            .map(Value::Number);
        self.result_payload("call", result)
    }

    fn restart(&mut self, source: LiveSource) -> Result<JsonValue, LiveSessionError> {
        let program = compile_source(source.source()).map_err(backend_error)?;
        let hbc = encode_program(&program).map_err(backend_error)?;
        let artifact = compile_artifact_from_hbc(&hbc).map_err(backend_error)?;
        let target = self.target_handle.clone().ok_or_else(|| {
            LiveSessionError::new(
                "live-session/disposed",
                "whole-Wasm instrumentation target has been disposed",
            )
        })?;
        let module = NativeModule::load_with_instrumentation(&artifact, self.hub.clone(), target)
            .map_err(backend_error)?;
        self.module = Some(module);
        self.artifact = artifact;
        self.source = source;
        self.pending_source = None;
        self.generation = self.generation.saturating_add(1);
        self.sequence = 0;
        self.status = LiveSessionStatus::Ready;
        Ok(json!({
            "operation": "restart",
            "status": self.status.as_str(),
            "generation": self.generation,
            "target": self.target_payload(),
        }))
    }

    fn reset(&mut self) -> Result<JsonValue, LiveSessionError> {
        if let Some(source) = self.pending_source.take() {
            return self.restart(source);
        }
        let target = self.target_handle.clone().ok_or_else(|| {
            LiveSessionError::new(
                "live-session/disposed",
                "whole-Wasm instrumentation target has been disposed",
            )
        })?;
        let module =
            NativeModule::load_with_instrumentation(&self.artifact, self.hub.clone(), target)
                .map_err(backend_error)?;
        self.module = Some(module);
        self.generation = self.generation.saturating_add(1);
        self.sequence = 0;
        self.status = LiveSessionStatus::Ready;
        Ok(json!({
            "operation": "reset",
            "status": self.status.as_str(),
            "generation": self.generation,
            "target": self.target_payload(),
        }))
    }

    fn dispose(&mut self) -> JsonValue {
        if self.status == LiveSessionStatus::Disposed {
            return JsonValue::Bool(false);
        }
        self.module = None;
        self.remove_target();
        self.pending_source = None;
        self.status = LiveSessionStatus::Disposed;
        JsonValue::Bool(true)
    }

    fn target_payload(&self) -> JsonValue {
        self.target_handle
            .as_ref()
            .map(|target| {
                json!({
                    "id": target.target_id(),
                    "generation": target.generation(),
                })
            })
            .unwrap_or(JsonValue::Null)
    }

    fn remove_target(&mut self) {
        if let Some(target) = self.target_handle.take() {
            let _ = self.hub.borrow_mut().remove_target(&target);
        }
    }
}

impl Drop for WholeWasmLiveSession {
    fn drop(&mut self) {
        self.remove_target();
    }
}

impl LiveSession for WholeWasmLiveSession {
    fn state(&self) -> LiveSessionState {
        LiveSessionState {
            session_id: self.session_id.clone(),
            source_id: self.source.source_id().to_owned(),
            generation: self.generation,
            revision: self.source.revision().to_owned(),
            sequence: self.sequence,
            backend: LiveBackend::WholeWasm,
            status: self.status,
        }
    }

    fn capabilities(&self) -> LiveSessionCapabilities {
        LiveSessionCapabilities {
            backend: LiveBackend::WholeWasm,
            operations: vec![
                LiveSessionOperation::Run,
                LiveSessionOperation::Call,
                LiveSessionOperation::Update,
                LiveSessionOperation::Reset,
                LiveSessionOperation::Cancel,
                LiveSessionOperation::Dispose,
            ],
            replacement_policies: vec![
                LiveReplacementPolicy::Restart,
                LiveReplacementPolicy::ReplaceOnNextStart,
            ],
        }
    }

    fn dispatch_command(
        &mut self,
        command: LiveSessionCommand,
    ) -> Result<JsonValue, LiveSessionError> {
        match command {
            LiveSessionCommand::Run { .. } => self.run(),
            LiveSessionCommand::Call {
                function,
                arguments,
            } => self.call(function, arguments),
            LiveSessionCommand::Update { source, policy } => match policy {
                LiveReplacementPolicy::Restart => self.restart(source),
                LiveReplacementPolicy::ReplaceOnNextStart => {
                    let revision = source.revision().to_owned();
                    self.pending_source = Some(source);
                    Ok(json!({
                        "accepted": true,
                        "activation": "next-start",
                        "revision": revision,
                    }))
                }
                LiveReplacementPolicy::PreserveRuntime => Err(LiveSessionError::new(
                    "live-session/unsupported-replacement",
                    "whole-Wasm backend does not support preserve-runtime replacement",
                )),
            },
            LiveSessionCommand::Reset => self.reset(),
            LiveSessionCommand::Cancel => {
                self.status = LiveSessionStatus::Cancelled;
                self.pending_source = None;
                Ok(json!({"cancelled": true}))
            }
            LiveSessionCommand::Dispose => Ok(self.dispose()),
            _ => Err(LiveSessionError::new(
                "live-session/unsupported-operation",
                "whole-Wasm backend does not support this operation",
            )),
        }
    }
}

fn json_to_i64(value: JsonValue) -> Result<i64, LiveSessionError> {
    value.as_i64().ok_or_else(|| {
        LiveSessionError::backend(
            "whole-Wasm LiveSession call currently requires integer arguments",
        )
    })
}

fn value_to_json(value: &Value) -> Result<JsonValue, LiveSessionError> {
    let encoded = crate::json::write(value).map_err(|error| {
        LiveSessionError::backend(format!("unable to encode whole-Wasm result: {error}"))
    })?;
    serde_json::from_str(&encoded).map_err(|error| {
        LiveSessionError::backend(format!("whole-Wasm result is not valid JSON: {error}"))
    })
}

fn backend_error(error: impl std::fmt::Display) -> LiveSessionError {
    LiveSessionError::backend(error.to_string())
}

fn target_id(owner_session_id: &str, session_id: &str) -> String {
    format!("{owner_session_id}/whole-wasm/{session_id}")
}

fn whole_wasm_capabilities() -> BTreeSet<Capability> {
    [
        Capability::EventSemanticBoundary,
        Capability::EventLifecycle,
    ]
    .into_iter()
    .collect()
}

fn instrumentation_error(error: impl std::fmt::Display) -> LiveSessionError {
    LiveSessionError::backend(format!("whole-Wasm instrumentation target: {error}"))
}
