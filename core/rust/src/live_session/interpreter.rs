use serde_json::{json, Value as JsonValue};

use super::{
    required_text, LiveBackend, LiveReplacementPolicy, LiveSession, LiveSessionCapabilities,
    LiveSessionCommand, LiveSessionError, LiveSessionOperation, LiveSessionState,
    LiveSessionStatus, LiveSettlement, LiveSource,
};

pub struct InterpreterLiveSession {
    session_id: String,
    source: LiveSource,
    handle: Option<u64>,
    generation_base: u64,
    backend_generation: u64,
    sequence: u64,
    status: LiveSessionStatus,
    pending_source: Option<LiveSource>,
}

impl InterpreterLiveSession {
    pub fn start(
        session_id: impl Into<String>,
        source: LiveSource,
    ) -> Result<Self, LiveSessionError> {
        let session_id = required_text(session_id.into(), "session id")?;
        let (handle, info) = start_backend(&session_id, &source)?;
        let mut session = Self {
            session_id,
            source,
            handle: Some(handle),
            generation_base: 0,
            backend_generation: 0,
            sequence: 0,
            status: LiveSessionStatus::Ready,
            pending_source: None,
        };
        session.sync_info(&info)?;
        Ok(session)
    }

    pub fn pending_revision(&self) -> Option<&str> {
        self.pending_source.as_ref().map(LiveSource::revision)
    }

    fn generation(&self) -> u64 {
        self.generation_base.saturating_add(self.backend_generation)
    }

    fn handle(&self) -> Result<u64, LiveSessionError> {
        self.handle.ok_or_else(|| {
            LiveSessionError::new(
                "live-session/disposed",
                "interpreter live session has been disposed",
            )
        })
    }

    fn refresh(&mut self) -> Result<(), LiveSessionError> {
        let handle = self.handle()?;
        let info = invoke_legacy(json!({"op": "info", "handle": handle}))?;
        self.sync_info(&info)
    }

    fn sync_info(&mut self, info: &JsonValue) -> Result<(), LiveSessionError> {
        self.backend_generation = required_u64(info, "generation")?;
        self.sequence = required_u64(info, "sequence")?;
        self.status = parse_status(required_string(info, "status")?)?;
        Ok(())
    }

    fn invoke_handle(&self, mut request: JsonValue) -> Result<JsonValue, LiveSessionError> {
        let handle = self.handle()?;
        let object = request.as_object_mut().ok_or_else(|| {
            LiveSessionError::new(
                "live-session/internal",
                "interpreter backend request must be a JSON object",
            )
        })?;
        object.insert("handle".into(), JsonValue::from(handle));
        invoke_legacy(request)
    }

    fn invoke_and_refresh(&mut self, request: JsonValue) -> Result<JsonValue, LiveSessionError> {
        let payload = self.invoke_handle(request)?;
        self.refresh()?;
        Ok(payload)
    }

    fn restart(&mut self, source: LiveSource) -> Result<JsonValue, LiveSessionError> {
        let next_generation = self.generation().saturating_add(1);
        let (new_handle, new_info) = start_backend(&self.session_id, &source)?;
        if let Some(old_handle) = self.handle {
            if let Err(error) = invoke_legacy(json!({"op": "dispose", "handle": old_handle})) {
                let _ = invoke_legacy(json!({"op": "dispose", "handle": new_handle}));
                return Err(error);
            }
        }
        self.source = source;
        self.handle = Some(new_handle);
        self.generation_base = next_generation;
        self.backend_generation = 0;
        self.sequence = 0;
        self.status = LiveSessionStatus::Ready;
        self.pending_source = None;
        self.sync_info(&new_info)?;
        Ok(new_info)
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
        self.invoke_and_refresh(json!({"op": "reset"}))
    }

    fn dispose(&mut self) -> Result<JsonValue, LiveSessionError> {
        let Some(handle) = self.handle else {
            self.status = LiveSessionStatus::Disposed;
            return Ok(JsonValue::Bool(false));
        };
        let payload = invoke_legacy(json!({"op": "dispose", "handle": handle}))?;
        self.handle = None;
        self.status = LiveSessionStatus::Disposed;
        self.pending_source = None;
        Ok(payload)
    }
}

impl LiveSession for InterpreterLiveSession {
    fn state(&self) -> LiveSessionState {
        LiveSessionState {
            session_id: self.session_id.clone(),
            source_id: self.source.source_id().to_owned(),
            generation: self.generation(),
            revision: self.source.revision().to_owned(),
            sequence: self.sequence,
            backend: LiveBackend::Interpreter,
            status: self.status,
        }
    }

    fn capabilities(&self) -> LiveSessionCapabilities {
        LiveSessionCapabilities {
            backend: LiveBackend::Interpreter,
            operations: vec![
                LiveSessionOperation::Snapshot,
                LiveSessionOperation::Step,
                LiveSessionOperation::Run,
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
            LiveSessionCommand::Snapshot => self.invoke_and_refresh(json!({"op": "snapshot"})),
            LiveSessionCommand::Step => self.invoke_and_refresh(json!({"op": "step"})),
            LiveSessionCommand::Run { boundary_limit } => self.invoke_and_refresh(json!({
                "op": "run",
                "boundaryLimit": boundary_limit,
            })),
            LiveSessionCommand::Call { .. } => Err(LiveSessionError::new(
                "live-session/unsupported-operation",
                "interpreter backend does not support direct function calls",
            )),
            LiveSessionCommand::Pause => Err(LiveSessionError::new(
                "live-session/unsupported-operation",
                "interpreter backend does not support pause",
            )),
            LiveSessionCommand::Resume { settlement } => {
                let mut request = json!({"op": "resume"});
                if let Some(settlement) = settlement {
                    request["settlement"] = settlement_json(settlement);
                }
                self.invoke_and_refresh(request)
            }
            LiveSessionCommand::Resolve { value } => self.invoke_and_refresh(json!({
                "op": "resolve-suspension",
                "value": value,
            })),
            LiveSessionCommand::Reject { error } => self.invoke_and_refresh(json!({
                "op": "reject-suspension",
                "error": error,
            })),
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
                    "interpreter backend does not support preserve-runtime replacement",
                )),
            },
            LiveSessionCommand::Reset => self.reset(),
            LiveSessionCommand::Cancel => self.invoke_and_refresh(json!({"op": "cancel"})),
            LiveSessionCommand::Dispose => self.dispose(),
        }
    }
}

impl Drop for InterpreterLiveSession {
    fn drop(&mut self) {
        if let Some(handle) = self.handle {
            let _ = invoke_legacy(json!({"op": "dispose", "handle": handle}));
            self.handle = None;
        }
    }
}

fn start_backend(
    session_id: &str,
    source: &LiveSource,
) -> Result<(u64, JsonValue), LiveSessionError> {
    let info = invoke_legacy(json!({
        "op": "start",
        "sessionId": session_id,
        "sourceId": source.source_id(),
        "source": source.source(),
    }))?;
    let handle = required_u64(&info, "handle")?;
    Ok((handle, info))
}

fn settlement_json(settlement: LiveSettlement) -> JsonValue {
    match settlement {
        LiveSettlement::Fulfilled(value) => json!({
            "status": "fulfilled",
            "value": value,
        }),
        LiveSettlement::Rejected(error) => json!({
            "status": "rejected",
            "error": error,
        }),
    }
}

fn invoke_legacy(request: JsonValue) -> Result<JsonValue, LiveSessionError> {
    let encoded = request.to_string();
    let bytes = crate::interpreter_observation::invoke_json(&encoded);
    let response: JsonValue = serde_json::from_slice(&bytes).map_err(|error| {
        LiveSessionError::backend(format!(
            "interpreter observation returned invalid JSON: {error}"
        ))
    })?;
    if response.get("ok").and_then(JsonValue::as_bool) == Some(true) {
        return Ok(response.get("value").cloned().unwrap_or(JsonValue::Null));
    }
    let error = response.get("error").and_then(JsonValue::as_object);
    let code = error
        .and_then(|value| value.get("code"))
        .and_then(JsonValue::as_str)
        .unwrap_or("interpreter-observation/error");
    let message = error
        .and_then(|value| value.get("message"))
        .and_then(JsonValue::as_str)
        .unwrap_or("interpreter observation request failed");
    Err(LiveSessionError::new(
        format!("live-session/backend/{code}"),
        message,
    ))
}

fn required_u64(value: &JsonValue, field: &str) -> Result<u64, LiveSessionError> {
    value.get(field).and_then(JsonValue::as_u64).ok_or_else(|| {
        LiveSessionError::backend(format!(
            "interpreter observation response requires unsigned {field}"
        ))
    })
}

fn required_string<'a>(value: &'a JsonValue, field: &str) -> Result<&'a str, LiveSessionError> {
    value.get(field).and_then(JsonValue::as_str).ok_or_else(|| {
        LiveSessionError::backend(format!(
            "interpreter observation response requires string {field}"
        ))
    })
}

fn parse_status(status: &str) -> Result<LiveSessionStatus, LiveSessionError> {
    match status {
        "ready" => Ok(LiveSessionStatus::Ready),
        "running" => Ok(LiveSessionStatus::Running),
        "suspended" => Ok(LiveSessionStatus::Suspended),
        "returned" => Ok(LiveSessionStatus::Returned),
        "failed" => Ok(LiveSessionStatus::Failed),
        "cancelled" => Ok(LiveSessionStatus::Cancelled),
        "disposed" => Ok(LiveSessionStatus::Disposed),
        other => Err(LiveSessionError::backend(format!(
            "unknown interpreter live-session status: {other}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::{invoke_legacy, InterpreterLiveSession};
    use crate::live_session::LiveSource;
    use serde_json::json;

    #[test]
    fn dropping_an_interpreter_adapter_releases_its_backend_handle() {
        let handle = {
            let session = InterpreterLiveSession::start(
                "fixture/live-interpreter-drop",
                LiveSource::new("drop.hal", "sha256:drop", "(+ 1 2)").unwrap(),
            )
            .unwrap();
            session.handle.expect("started session must own a handle")
        };

        let error = invoke_legacy(json!({"op": "info", "handle": handle})).unwrap_err();
        assert_eq!(
            error.code(),
            "live-session/backend/interpreter-observation/no-session"
        );
    }
}
