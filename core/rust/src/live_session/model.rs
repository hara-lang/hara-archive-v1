use serde_json::{json, Value as JsonValue};
use std::fmt;

pub const LIVE_SESSION_PROTOCOL: &str = "hara.live-session/0-alpha";
pub const LIVE_SESSION_STATE_SCHEMA: &str = "hara.live-session.state/0-alpha";
pub const LIVE_SESSION_REPLY_SCHEMA: &str = "hara.live-session.reply/0-alpha";
pub const LIVE_SESSION_CAPABILITIES_SCHEMA: &str = "hara.live-session.capabilities/0-alpha";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiveBackend {
    Interpreter,
    Hbc,
    WholeWasm,
}

impl LiveBackend {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Interpreter => "interpreter",
            Self::Hbc => "hbc",
            Self::WholeWasm => "whole-wasm",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiveSessionStatus {
    Ready,
    Running,
    Paused,
    Suspended,
    Returned,
    Failed,
    Cancelled,
    Disposed,
}

impl LiveSessionStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Running => "running",
            Self::Paused => "paused",
            Self::Suspended => "suspended",
            Self::Returned => "returned",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Disposed => "disposed",
        }
    }

    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Returned | Self::Failed | Self::Cancelled | Self::Disposed
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiveSessionOperation {
    Snapshot,
    Step,
    Run,
    Call,
    Pause,
    Resume,
    Resolve,
    Reject,
    Update,
    Reset,
    Cancel,
    Dispose,
}

impl LiveSessionOperation {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Snapshot => "snapshot",
            Self::Step => "step",
            Self::Run => "run",
            Self::Call => "call",
            Self::Pause => "pause",
            Self::Resume => "resume",
            Self::Resolve => "resolve",
            Self::Reject => "reject",
            Self::Update => "update",
            Self::Reset => "reset",
            Self::Cancel => "cancel",
            Self::Dispose => "dispose",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiveReplacementPolicy {
    Restart,
    ReplaceOnNextStart,
    PreserveRuntime,
}

impl LiveReplacementPolicy {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Restart => "restart",
            Self::ReplaceOnNextStart => "replace-on-next-start",
            Self::PreserveRuntime => "preserve-runtime",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveSource {
    source_id: String,
    revision: String,
    source: String,
}

impl LiveSource {
    pub fn new(
        source_id: impl Into<String>,
        revision: impl Into<String>,
        source: impl Into<String>,
    ) -> Result<Self, LiveSessionError> {
        Ok(Self {
            source_id: required_text(source_id.into(), "source id")?,
            revision: required_text(revision.into(), "revision")?,
            source: required_text(source.into(), "source")?,
        })
    }

    pub fn source_id(&self) -> &str {
        &self.source_id
    }

    pub fn revision(&self) -> &str {
        &self.revision
    }

    pub fn source(&self) -> &str {
        &self.source
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveSessionState {
    pub session_id: String,
    pub source_id: String,
    pub generation: u64,
    pub revision: String,
    pub sequence: u64,
    pub backend: LiveBackend,
    pub status: LiveSessionStatus,
}

impl LiveSessionState {
    pub fn to_json(&self) -> JsonValue {
        json!({
            "schema": LIVE_SESSION_STATE_SCHEMA,
            "protocol": LIVE_SESSION_PROTOCOL,
            "session-id": self.session_id,
            "source-id": self.source_id,
            "generation": self.generation,
            "revision": self.revision,
            "sequence": self.sequence,
            "backend": self.backend.as_str(),
            "status": self.status.as_str(),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveSessionCapabilities {
    pub backend: LiveBackend,
    pub operations: Vec<LiveSessionOperation>,
    pub replacement_policies: Vec<LiveReplacementPolicy>,
}

impl LiveSessionCapabilities {
    pub fn supports(&self, operation: LiveSessionOperation) -> bool {
        self.operations.contains(&operation)
    }

    pub fn supports_replacement(&self, policy: LiveReplacementPolicy) -> bool {
        self.replacement_policies.contains(&policy)
    }

    pub fn to_json(&self) -> JsonValue {
        json!({
            "schema": LIVE_SESSION_CAPABILITIES_SCHEMA,
            "protocol": LIVE_SESSION_PROTOCOL,
            "backend": self.backend.as_str(),
            "operations": self.operations.iter().map(|operation| operation.as_str()).collect::<Vec<_>>(),
            "replacement-policies": self.replacement_policies.iter().map(|policy| policy.as_str()).collect::<Vec<_>>(),
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum LiveSettlement {
    Fulfilled(JsonValue),
    Rejected(JsonValue),
}

#[derive(Clone, Debug, PartialEq)]
pub enum LiveSessionCommand {
    Snapshot,
    Step,
    Run {
        boundary_limit: usize,
    },
    Call {
        function: u16,
        arguments: Vec<JsonValue>,
    },
    Pause,
    Resume {
        settlement: Option<LiveSettlement>,
    },
    Resolve {
        value: JsonValue,
    },
    Reject {
        error: JsonValue,
    },
    Update {
        source: LiveSource,
        policy: LiveReplacementPolicy,
    },
    Reset,
    Cancel,
    Dispose,
}

impl LiveSessionCommand {
    pub const fn operation(&self) -> LiveSessionOperation {
        match self {
            Self::Snapshot => LiveSessionOperation::Snapshot,
            Self::Step => LiveSessionOperation::Step,
            Self::Run { .. } => LiveSessionOperation::Run,
            Self::Call { .. } => LiveSessionOperation::Call,
            Self::Pause => LiveSessionOperation::Pause,
            Self::Resume { .. } => LiveSessionOperation::Resume,
            Self::Resolve { .. } => LiveSessionOperation::Resolve,
            Self::Reject { .. } => LiveSessionOperation::Reject,
            Self::Update { .. } => LiveSessionOperation::Update,
            Self::Reset => LiveSessionOperation::Reset,
            Self::Cancel => LiveSessionOperation::Cancel,
            Self::Dispose => LiveSessionOperation::Dispose,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct LiveSessionRequest {
    pub protocol: String,
    pub request_id: String,
    pub session_id: String,
    pub generation: Option<u64>,
    pub revision: Option<String>,
    pub command: LiveSessionCommand,
}

impl LiveSessionRequest {
    pub fn for_state(
        request_id: impl Into<String>,
        state: &LiveSessionState,
        command: LiveSessionCommand,
    ) -> Self {
        Self {
            protocol: LIVE_SESSION_PROTOCOL.into(),
            request_id: request_id.into(),
            session_id: state.session_id.clone(),
            generation: Some(state.generation),
            revision: Some(state.revision.clone()),
            command,
        }
    }

    fn validate(&self, state: &LiveSessionState) -> Result<(), LiveSessionError> {
        if self.protocol != LIVE_SESSION_PROTOCOL {
            return Err(LiveSessionError::new(
                "live-session/protocol",
                format!("unsupported live-session protocol: {}", self.protocol),
            ));
        }
        required_text(self.request_id.clone(), "request id")?;
        required_text(self.session_id.clone(), "session id")?;
        if self.session_id != state.session_id {
            return Err(LiveSessionError::new(
                "live-session/session-mismatch",
                format!(
                    "request targets session {} but adapter owns {}",
                    self.session_id, state.session_id
                ),
            ));
        }
        if let Some(generation) = self.generation {
            if generation != state.generation {
                return Err(LiveSessionError::new(
                    "live-session/stale-generation",
                    format!(
                        "request generation {generation} does not match current generation {}",
                        state.generation
                    ),
                ));
            }
        }
        if let Some(revision) = self.revision.as_deref() {
            if revision != state.revision {
                return Err(LiveSessionError::new(
                    "live-session/stale-revision",
                    format!(
                        "request revision {revision} does not match current revision {}",
                        state.revision
                    ),
                ));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct LiveSessionReply {
    pub request_id: String,
    pub state: LiveSessionState,
    pub payload: JsonValue,
}

impl LiveSessionReply {
    pub fn to_json(&self) -> JsonValue {
        json!({
            "schema": LIVE_SESSION_REPLY_SCHEMA,
            "protocol": LIVE_SESSION_PROTOCOL,
            "request-id": self.request_id,
            "state": self.state.to_json(),
            "payload": self.payload,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveSessionError {
    code: String,
    message: String,
}

impl LiveSessionError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }

    pub fn backend(message: impl Into<String>) -> Self {
        Self::new("live-session/backend", message)
    }

    pub fn code(&self) -> &str {
        &self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for LiveSessionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for LiveSessionError {}

pub trait LiveSession {
    fn state(&self) -> LiveSessionState;

    fn capabilities(&self) -> LiveSessionCapabilities;

    fn dispatch_command(
        &mut self,
        command: LiveSessionCommand,
    ) -> Result<JsonValue, LiveSessionError>;

    fn dispatch(
        &mut self,
        request: LiveSessionRequest,
    ) -> Result<LiveSessionReply, LiveSessionError> {
        let before = self.state();
        request.validate(&before)?;
        let operation = request.command.operation();
        match before.status {
            LiveSessionStatus::Disposed if operation != LiveSessionOperation::Dispose => {
                return Err(LiveSessionError::new(
                    "live-session/disposed",
                    "disposed live session accepts only dispose",
                ));
            }
            LiveSessionStatus::Cancelled if operation != LiveSessionOperation::Dispose => {
                return Err(LiveSessionError::new(
                    "live-session/cancelled",
                    "cancelled live session accepts only dispose",
                ));
            }
            _ => {}
        }
        let capabilities = self.capabilities();
        if !capabilities.supports(operation) {
            return Err(LiveSessionError::new(
                "live-session/unsupported-operation",
                format!(
                    "{} backend does not support {}",
                    before.backend.as_str(),
                    operation.as_str()
                ),
            ));
        }
        if let LiveSessionCommand::Update { policy, .. } = &request.command {
            if !capabilities.supports_replacement(*policy) {
                return Err(LiveSessionError::new(
                    "live-session/unsupported-replacement",
                    format!(
                        "{} backend does not support {} replacement",
                        before.backend.as_str(),
                        policy.as_str()
                    ),
                ));
            }
        }
        let request_id = request.request_id;
        let payload = self.dispatch_command(request.command)?;
        Ok(LiveSessionReply {
            request_id,
            state: self.state(),
            payload,
        })
    }
}

pub(crate) fn required_text(value: String, label: &str) -> Result<String, LiveSessionError> {
    if value.trim().is_empty() {
        Err(LiveSessionError::new(
            "live-session/invalid-identity",
            format!("live-session {label} must not be empty"),
        ))
    } else {
        Ok(value)
    }
}
