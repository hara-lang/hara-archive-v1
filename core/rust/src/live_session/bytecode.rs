use serde_json::{json, Value as JsonValue};

use crate::core::Value;
use crate::task::{PromiseRejection, PromiseState};
use crate::vm::{BytecodeObservationSession, BytecodeSessionStatus};

use super::{
    required_text, LiveBackend, LiveReplacementPolicy, LiveSession, LiveSessionCapabilities,
    LiveSessionCommand, LiveSessionError, LiveSessionOperation, LiveSessionState,
    LiveSessionStatus, LiveSettlement, LiveSource,
};

pub struct BytecodeLiveSession {
    session: BytecodeObservationSession,
    revision: String,
    generation: u64,
    pending_source: Option<LiveSource>,
    terminal_status: Option<LiveSessionStatus>,
}

impl BytecodeLiveSession {
    pub fn compile(
        session_id: impl Into<String>,
        source: LiveSource,
    ) -> Result<Self, LiveSessionError> {
        let session_id = required_text(session_id.into(), "session id")?;
        let session = BytecodeObservationSession::compile_named(
            session_id,
            source.source_id(),
            source.source(),
        )
        .map_err(backend_error)?;
        Ok(Self {
            session,
            revision: source.revision().to_owned(),
            generation: 0,
            pending_source: None,
            terminal_status: None,
        })
    }

    pub fn from_artifact(
        session_id: impl Into<String>,
        source_id: impl Into<String>,
        revision: impl Into<String>,
        artifact: &[u8],
    ) -> Result<Self, LiveSessionError> {
        let session_id = required_text(session_id.into(), "session id")?;
        let source_id = required_text(source_id.into(), "source id")?;
        let revision = required_text(revision.into(), "revision")?;
        let session =
            BytecodeObservationSession::from_artifact_named(session_id, source_id, artifact)
                .map_err(backend_error)?;
        Ok(Self {
            session,
            revision,
            generation: 0,
            pending_source: None,
            terminal_status: None,
        })
    }

    pub fn pending_revision(&self) -> Option<&str> {
        self.pending_source.as_ref().map(LiveSource::revision)
    }

    fn current_status(&self) -> LiveSessionStatus {
        self.terminal_status
            .unwrap_or_else(|| map_status(self.session.status()))
    }

    fn restart(&mut self, source: LiveSource) -> Result<JsonValue, LiveSessionError> {
        let replacement = BytecodeObservationSession::compile_named(
            self.session.session_id(),
            source.source_id(),
            source.source(),
        )
        .map_err(backend_error)?;
        self.session.dispose();
        self.session = replacement;
        self.revision = source.revision().to_owned();
        self.generation = self.generation.saturating_add(1);
        self.pending_source = None;
        self.terminal_status = None;
        self.snapshot_json()
    }

    fn reset(&mut self) -> Result<JsonValue, LiveSessionError> {
        if let Some(source) = self.pending_source.take() {
            return match self.restart(source.clone()) {
                Ok(payload) => Ok(payload),
                Err(error) => {
                    self.pending_source = Some(source);
                    Err(error)
                }
            };
        }
        let payload = self.session.reset().map_err(backend_error)?;
        self.generation = self.generation.saturating_add(1);
        self.terminal_status = None;
        value_to_json(&payload)
    }

    fn snapshot_json(&self) -> Result<JsonValue, LiveSessionError> {
        let snapshot = self.session.snapshot_value().map_err(backend_error)?;
        value_to_json(&snapshot)
    }
}

impl LiveSession for BytecodeLiveSession {
    fn state(&self) -> LiveSessionState {
        LiveSessionState {
            session_id: self.session.session_id().to_owned(),
            source_id: self.session.source_id().to_owned(),
            generation: self.generation,
            revision: self.revision.clone(),
            sequence: self.session.sequence(),
            backend: LiveBackend::Hbc,
            status: self.current_status(),
        }
    }

    fn capabilities(&self) -> LiveSessionCapabilities {
        LiveSessionCapabilities {
            backend: LiveBackend::Hbc,
            operations: vec![
                LiveSessionOperation::Snapshot,
                LiveSessionOperation::Step,
                LiveSessionOperation::Run,
                LiveSessionOperation::Pause,
                LiveSessionOperation::Resume,
                LiveSessionOperation::Resolve,
                LiveSessionOperation::Reject,
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
            LiveSessionCommand::Snapshot => self.snapshot_json(),
            LiveSessionCommand::Step => {
                let payload = self.session.step().map_err(backend_error)?;
                value_to_json(&payload)
            }
            LiveSessionCommand::Run { boundary_limit } => {
                let payload = self.session.run(boundary_limit).map_err(backend_error)?;
                value_to_json(&payload)
            }
            LiveSessionCommand::Call { .. } => Err(LiveSessionError::new(
                "live-session/unsupported-operation",
                "HBC observation backend does not support direct function calls",
            )),
            LiveSessionCommand::Pause => Ok(JsonValue::Bool(self.session.pause())),
            LiveSessionCommand::Resume { settlement } => {
                let settlement = settlement.map(settlement_state).transpose()?;
                let payload = self.session.resume(settlement).map_err(backend_error)?;
                value_to_json(&payload)
            }
            LiveSessionCommand::Resolve { value } => {
                let value = json_to_value(value)?;
                Ok(JsonValue::Bool(
                    self.session
                        .resolve_suspension(value)
                        .map_err(backend_error)?,
                ))
            }
            LiveSessionCommand::Reject { error } => {
                let error = json_to_value(error)?;
                Ok(JsonValue::Bool(
                    self.session
                        .reject_suspension(error)
                        .map_err(backend_error)?,
                ))
            }
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
                    "HBC backend does not support preserve-runtime replacement",
                )),
            },
            LiveSessionCommand::Reset => self.reset(),
            LiveSessionCommand::Cancel => {
                let cancelled = self.session.dispose();
                self.terminal_status = Some(LiveSessionStatus::Cancelled);
                self.pending_source = None;
                Ok(json!({"cancelled": cancelled}))
            }
            LiveSessionCommand::Dispose => {
                let disposed = self.session.dispose();
                self.terminal_status = Some(LiveSessionStatus::Disposed);
                self.pending_source = None;
                Ok(JsonValue::Bool(disposed))
            }
        }
    }
}

fn settlement_state(settlement: LiveSettlement) -> Result<PromiseState, LiveSessionError> {
    match settlement {
        LiveSettlement::Fulfilled(value) => Ok(PromiseState::Fulfilled(json_to_value(value)?)),
        LiveSettlement::Rejected(error) => Ok(PromiseState::Rejected(PromiseRejection::Value(
            json_to_value(error)?,
        ))),
    }
}

fn json_to_value(value: JsonValue) -> Result<Value, LiveSessionError> {
    crate::json::read(&value.to_string()).map_err(|error| {
        LiveSessionError::backend(format!("unable to decode live-session value: {error}"))
    })
}

fn value_to_json(value: &Value) -> Result<JsonValue, LiveSessionError> {
    let encoded = crate::json::write(value).map_err(|error| {
        LiveSessionError::backend(format!(
            "unable to encode HBC live-session payload: {error}"
        ))
    })?;
    serde_json::from_str(&encoded).map_err(|error| {
        LiveSessionError::backend(format!(
            "HBC live-session payload is not valid JSON: {error}"
        ))
    })
}

fn backend_error(error: impl std::fmt::Display) -> LiveSessionError {
    LiveSessionError::backend(error.to_string())
}

fn map_status(status: BytecodeSessionStatus) -> LiveSessionStatus {
    match status {
        BytecodeSessionStatus::Ready => LiveSessionStatus::Ready,
        BytecodeSessionStatus::Running => LiveSessionStatus::Running,
        BytecodeSessionStatus::Paused => LiveSessionStatus::Paused,
        BytecodeSessionStatus::Suspended => LiveSessionStatus::Suspended,
        BytecodeSessionStatus::Returned => LiveSessionStatus::Returned,
        BytecodeSessionStatus::Failed => LiveSessionStatus::Failed,
        BytecodeSessionStatus::Disposed => LiveSessionStatus::Disposed,
    }
}
