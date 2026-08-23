//! Whole-Wasm LiveSession adapter.
//!
//! Whole-Wasm execution is synchronous and prepared. It therefore exposes
//! run/call and lifecycle operations, but never claims VM-style stepping,
//! suspension, or snapshots.

use serde_json::{json, Value as JsonValue};

use crate::core::Value;
use crate::vm::{compile_source, encode_program, FunctionId};
use crate::whole_wasm::{compile_artifact_from_hbc, NativeModule};

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
}

impl WholeWasmLiveSession {
    pub(crate) fn start(
        session_id: impl Into<String>,
        source: LiveSource,
    ) -> Result<Self, LiveSessionError> {
        let program = compile_source(source.source()).map_err(backend_error)?;
        let hbc = encode_program(&program).map_err(backend_error)?;
        let artifact = compile_artifact_from_hbc(&hbc).map_err(backend_error)?;
        Self::from_artifact(session_id, source, artifact)
    }

    pub(crate) fn from_artifact(
        session_id: impl Into<String>,
        source: LiveSource,
        artifact: Vec<u8>,
    ) -> Result<Self, LiveSessionError> {
        let session_id = required_text(session_id.into(), "session id")?;
        let module = NativeModule::load(&artifact).map_err(backend_error)?;
        Ok(Self {
            session_id,
            source,
            artifact,
            generation: 0,
            sequence: 0,
            status: LiveSessionStatus::Ready,
            pending_source: None,
            module: Some(module),
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
        match result {
            Ok(value) => {
                self.status = LiveSessionStatus::Returned;
                Ok(json!({
                    "operation": operation,
                    "status": self.status.as_str(),
                    "result": value_to_json(&value)?,
                    "sequence": self.sequence,
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
        let replacement = Self::start(self.session_id.clone(), source.clone())?;
        self.module = replacement.module;
        self.artifact = replacement.artifact;
        self.source = source;
        self.pending_source = None;
        self.generation = self.generation.saturating_add(1);
        self.sequence = 0;
        self.status = LiveSessionStatus::Ready;
        Ok(json!({
            "operation": "restart",
            "status": self.status.as_str(),
            "generation": self.generation,
        }))
    }

    fn reset(&mut self) -> Result<JsonValue, LiveSessionError> {
        if let Some(source) = self.pending_source.take() {
            return self.restart(source);
        }
        let module = NativeModule::load(&self.artifact).map_err(backend_error)?;
        self.module = Some(module);
        self.generation = self.generation.saturating_add(1);
        self.sequence = 0;
        self.status = LiveSessionStatus::Ready;
        Ok(json!({
            "operation": "reset",
            "status": self.status.as_str(),
            "generation": self.generation,
        }))
    }

    fn dispose(&mut self) -> JsonValue {
        if self.status == LiveSessionStatus::Disposed {
            return JsonValue::Bool(false);
        }
        self.module = None;
        self.pending_source = None;
        self.status = LiveSessionStatus::Disposed;
        JsonValue::Bool(true)
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
