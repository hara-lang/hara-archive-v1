use crate::core::{EvalFiber, EvalFiberState, Value};
use crate::task::{PromiseRejection, PromiseState};
use crate::{core, json, Runtime};
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};

pub const ABI_VERSION: i32 = 1;
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
const MAX_RUN_BOUNDARIES: usize = 100_000;
const MAX_BINDINGS: usize = 4_096;
const MAX_DISPLAY_CHARS: usize = 16_384;
const MAX_RETAINED_BOUNDARIES: usize = 100_000;
const DEFAULT_BINDINGS: usize = 128;
const DEFAULT_DISPLAY_CHARS: usize = 1_024;
const DEFAULT_HISTORY: usize = 256;

const SESSION_SCHEMA: &str = "hal.interpreter-observation-session/0-alpha";
const ENTRY_SCHEMA: &str = "hal.interpreter-observation-entry/0-alpha";
const HISTORY_SCHEMA: &str = "hal.interpreter-observation-history/0-alpha";
const RUN_SCHEMA: &str = "hal.interpreter-observation-run/0-alpha";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InterpreterSessionStatus {
    Ready,
    Running,
    Suspended,
    Returned,
    Failed,
    Cancelled,
    Disposed,
}

impl InterpreterSessionStatus {
    const fn as_keyword(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Running => "running",
            Self::Suspended => "suspended",
            Self::Returned => "returned",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Disposed => "disposed",
        }
    }

    const fn can_step(self) -> bool {
        matches!(self, Self::Ready | Self::Running)
    }
}

struct InterpreterContext {
    runtime: Runtime,
}

impl InterpreterContext {
    fn fresh() -> (Self, HashMap<String, Value>) {
        let runtime = Runtime::new();
        let environment = runtime.execution.snapshot();
        (Self { runtime }, environment)
    }

    fn run<T>(&self, operation: impl FnOnce() -> T) -> T {
        let namespaces = self.runtime.namespace_registry.clone();
        let protocols = self.runtime.protocols.clone();
        let macros = self.runtime.macros.clone();
        core::with_macros(macros, move || {
            core::with_namespace_registry(&namespaces, move || {
                core::with_protocols(&protocols, operation)
            })
        })
    }
}

struct InterpreterObservationSession {
    session_id: String,
    source_id: String,
    source: String,
    generation: u64,
    sequence: u64,
    status: InterpreterSessionStatus,
    context: Option<InterpreterContext>,
    fiber: Option<EvalFiber>,
    binding_limit: usize,
    display_chars: usize,
    history_limit: usize,
    history: VecDeque<Value>,
    dropped_history: u64,
}

impl InterpreterObservationSession {
    fn start_named(
        session_id: impl Into<String>,
        source_id: impl Into<String>,
        source: impl Into<String>,
    ) -> Result<Self, String> {
        let session_id = required_id(session_id.into(), "session id")?;
        let source_id = required_id(source_id.into(), "source id")?;
        let source = source.into();
        if source.trim().is_empty() {
            return Err("interpreter observation source must not be empty".into());
        }
        let (context, environment) = InterpreterContext::fresh();
        let fiber = context.run(|| EvalFiber::start_observed(&source, environment))?;
        Ok(Self {
            session_id,
            source_id,
            source,
            generation: 0,
            sequence: 0,
            status: InterpreterSessionStatus::Ready,
            context: Some(context),
            fiber: Some(fiber),
            binding_limit: DEFAULT_BINDINGS,
            display_chars: DEFAULT_DISPLAY_CHARS,
            history_limit: DEFAULT_HISTORY,
            history: VecDeque::new(),
            dropped_history: 0,
        })
    }

    fn session_id(&self) -> &str {
        &self.session_id
    }

    fn source_id(&self) -> &str {
        &self.source_id
    }

    fn status(&self) -> InterpreterSessionStatus {
        self.status
    }

    fn generation(&self) -> u64 {
        self.generation
    }

    fn sequence(&self) -> u64 {
        self.sequence
    }

    fn retained_history(&self) -> usize {
        self.history.len()
    }

    fn dropped_history(&self) -> u64 {
        self.dropped_history
    }

    fn ensure_live(&self) -> Result<(), String> {
        if self.status == InterpreterSessionStatus::Disposed {
            Err("INTERPRETER_OBSERVATION_SESSION_DISPOSED".into())
        } else if self.context.is_none() || self.fiber.is_none() {
            Err("INTERPRETER_OBSERVATION_SESSION_MISSING_RUNTIME".into())
        } else {
            Ok(())
        }
    }

    fn snapshot(&self) -> Result<Value, String> {
        self.ensure_live()?;
        let context = self.context.as_ref().expect("checked context");
        let fiber = self.fiber.as_ref().expect("checked fiber");
        let source_id = self.source_id.clone();
        let snapshot = context.run(|| {
            fiber.snapshot_observed_value_with_limits(
                source_id,
                self.binding_limit,
                self.display_chars,
            )
        });
        Ok(object([
            ("schema", Value::String(SESSION_SCHEMA.into())),
            ("sessionId", Value::String(self.session_id.clone())),
            ("sourceId", Value::String(self.source_id.clone())),
            ("generation", Value::Number(safe_i64(self.generation))),
            ("sequence", Value::Number(safe_i64(self.sequence))),
            ("status", Value::String(self.status.as_keyword().into())),
            (
                "retained",
                Value::Number(safe_i64(self.history.len() as u64)),
            ),
            ("dropped", Value::Number(safe_i64(self.dropped_history))),
            ("snapshot", snapshot),
        ]))
    }

    fn step(&mut self) -> Result<Value, String> {
        self.ensure_live()?;
        if !self.status.can_step() {
            return Err(format!(
                "INTERPRETER_OBSERVATION_CANNOT_STEP {}",
                self.status.as_keyword()
            ));
        }
        let source_id = self.source_id.clone();
        let binding_limit = self.binding_limit;
        let display_chars = self.display_chars;
        let boundary = {
            let context = self.context.as_ref().expect("checked context");
            let fiber = self.fiber.as_mut().expect("checked fiber");
            context.run(|| {
                fiber.step_observed_value_with_limits(source_id, binding_limit, display_chars)
            })
        };
        let entry = self.retain(boundary);
        self.sync_status();
        Ok(entry)
    }

    fn run(&mut self, boundary_limit: usize) -> Result<Value, String> {
        self.ensure_live()?;
        let mut executed = 0_usize;
        let mut last = Value::Nil;
        while executed < boundary_limit && self.status.can_step() {
            last = self.step()?;
            executed += 1;
        }
        Ok(object([
            ("schema", Value::String(RUN_SCHEMA.into())),
            ("sessionId", Value::String(self.session_id.clone())),
            ("sourceId", Value::String(self.source_id.clone())),
            ("generation", Value::Number(safe_i64(self.generation))),
            ("steps", Value::Number(safe_i64(executed as u64))),
            ("status", Value::String(self.status.as_keyword().into())),
            ("last", last),
            ("session", self.snapshot()?),
        ]))
    }

    fn resume(&mut self, settlement: Option<PromiseState>) -> Result<Value, String> {
        self.ensure_live()?;
        let pending = self
            .fiber
            .as_ref()
            .and_then(EvalFiber::pending)
            .ok_or_else(|| "INTERPRETER_OBSERVATION_NOT_SUSPENDED".to_string())?;
        if let Some(settlement) = settlement {
            if !matches!(pending.state(), PromiseState::Pending) {
                return Err("INTERPRETER_OBSERVATION_PROMISE_ALREADY_SETTLED".into());
            }
            match settlement {
                PromiseState::Pending => {
                    return Err("INTERPRETER_OBSERVATION_SETTLEMENT_PENDING".into());
                }
                PromiseState::Fulfilled(value) => {
                    pending.resolve(value);
                }
                PromiseState::Rejected(error) => {
                    pending.reject_rejection(error);
                }
            }
        }
        let state = pending.state();
        if matches!(state, PromiseState::Pending) {
            return Err("INTERPRETER_OBSERVATION_PROMISE_PENDING".into());
        }
        let source_id = self.source_id.clone();
        let binding_limit = self.binding_limit;
        let display_chars = self.display_chars;
        let boundary = {
            let context = self.context.as_ref().expect("checked context");
            let fiber = self.fiber.as_mut().expect("checked fiber");
            context.run(|| {
                fiber.resume_observed_value_with_limits(
                    state,
                    source_id,
                    binding_limit,
                    display_chars,
                )
            })
        };
        let entry = self.retain(boundary);
        self.sync_status();
        Ok(entry)
    }

    fn resolve_suspension(&self, value: Value) -> Result<bool, String> {
        self.ensure_live()?;
        self.fiber
            .as_ref()
            .and_then(EvalFiber::pending)
            .ok_or_else(|| "INTERPRETER_OBSERVATION_NOT_SUSPENDED".to_string())
            .map(|promise| promise.resolve(value))
    }

    fn reject_suspension(&self, error: Value) -> Result<bool, String> {
        self.ensure_live()?;
        self.fiber
            .as_ref()
            .and_then(EvalFiber::pending)
            .ok_or_else(|| "INTERPRETER_OBSERVATION_NOT_SUSPENDED".to_string())
            .map(|promise| promise.reject_value(error))
    }

    fn suspension_state(&self) -> Result<Value, String> {
        self.ensure_live()?;
        Ok(self
            .fiber
            .as_ref()
            .and_then(EvalFiber::pending)
            .map(|promise| match promise.state() {
                PromiseState::Pending => Value::String("pending".into()),
                PromiseState::Fulfilled(_) => Value::String("fulfilled".into()),
                PromiseState::Rejected(_) => Value::String("rejected".into()),
            })
            .unwrap_or(Value::Nil))
    }

    fn cancel(&mut self) -> Result<Value, String> {
        self.ensure_live()?;
        if let Some(fiber) = self.fiber.as_mut() {
            fiber.cancel();
        }
        self.status = InterpreterSessionStatus::Cancelled;
        self.snapshot()
    }

    fn reset(&mut self) -> Result<Value, String> {
        self.ensure_live()?;
        self.fiber.take();
        self.context.take();
        let (context, environment) = InterpreterContext::fresh();
        let source = self.source.clone();
        let fiber = context.run(|| EvalFiber::start_observed(&source, environment))?;
        self.context = Some(context);
        self.fiber = Some(fiber);
        self.generation = self.generation.saturating_add(1);
        self.sequence = 0;
        self.status = InterpreterSessionStatus::Ready;
        self.history.clear();
        self.dropped_history = 0;
        self.snapshot()
    }

    fn dispose(&mut self) -> bool {
        if self.status == InterpreterSessionStatus::Disposed {
            return false;
        }
        if let Some(fiber) = self.fiber.as_mut() {
            fiber.cancel();
        }
        self.fiber.take();
        self.context.take();
        self.history.clear();
        self.status = InterpreterSessionStatus::Disposed;
        true
    }

    fn set_observation_limits(&mut self, bindings: usize, display_chars: usize) -> Value {
        self.binding_limit = bindings;
        self.display_chars = display_chars;
        object([
            ("bindings", Value::Number(safe_i64(bindings as u64))),
            (
                "displayChars",
                Value::Number(safe_i64(display_chars as u64)),
            ),
        ])
    }

    fn set_history_limit(&mut self, limit: usize) -> Value {
        self.history_limit = limit;
        while self.history.len() > limit {
            self.history.pop_front();
            self.dropped_history = self.dropped_history.saturating_add(1);
        }
        object([
            ("history", Value::Number(safe_i64(limit as u64))),
            (
                "retained",
                Value::Number(safe_i64(self.history.len() as u64)),
            ),
            ("dropped", Value::Number(safe_i64(self.dropped_history))),
        ])
    }

    fn history(&self) -> Result<Value, String> {
        self.ensure_live()?;
        Ok(object([
            ("schema", Value::String(HISTORY_SCHEMA.into())),
            ("sessionId", Value::String(self.session_id.clone())),
            ("sourceId", Value::String(self.source_id.clone())),
            ("generation", Value::Number(safe_i64(self.generation))),
            ("sequence", Value::Number(safe_i64(self.sequence))),
            (
                "retained",
                Value::Number(safe_i64(self.history.len() as u64)),
            ),
            ("dropped", Value::Number(safe_i64(self.dropped_history))),
            (
                "entries",
                Value::Vector(self.history.iter().cloned().collect::<Vec<_>>().into()),
            ),
        ]))
    }

    fn result_display(&self) -> Result<Value, String> {
        self.ensure_live()?;
        Ok(match self.fiber.as_ref().expect("checked fiber").state() {
            EvalFiberState::Completed(value) => Value::String(value.display()),
            _ => Value::Nil,
        })
    }

    fn error_message(&self) -> Result<Value, String> {
        self.ensure_live()?;
        Ok(match self.fiber.as_ref().expect("checked fiber").state() {
            EvalFiberState::Failed(error) => Value::String(error),
            _ => Value::Nil,
        })
    }

    fn retain(&mut self, boundary: Value) -> Value {
        self.sequence = self.sequence.saturating_add(1);
        let entry = object([
            ("schema", Value::String(ENTRY_SCHEMA.into())),
            ("sessionId", Value::String(self.session_id.clone())),
            ("sourceId", Value::String(self.source_id.clone())),
            ("generation", Value::Number(safe_i64(self.generation))),
            ("sequence", Value::Number(safe_i64(self.sequence))),
            ("boundary", boundary),
        ]);
        if self.history_limit == 0 {
            self.dropped_history = self.dropped_history.saturating_add(1);
        } else {
            self.history.push_back(entry.clone());
            while self.history.len() > self.history_limit {
                self.history.pop_front();
                self.dropped_history = self.dropped_history.saturating_add(1);
            }
        }
        entry
    }

    fn sync_status(&mut self) {
        let Some(fiber) = self.fiber.as_ref() else {
            self.status = InterpreterSessionStatus::Disposed;
            return;
        };
        if fiber.observed_pending_boundaries() > 0 {
            self.status = InterpreterSessionStatus::Running;
            return;
        }
        self.status = match fiber.state() {
            EvalFiberState::Running => InterpreterSessionStatus::Running,
            EvalFiberState::Suspended => InterpreterSessionStatus::Suspended,
            EvalFiberState::Completed(_) => InterpreterSessionStatus::Returned,
            EvalFiberState::Failed(_) => InterpreterSessionStatus::Failed,
            EvalFiberState::Cancelled => InterpreterSessionStatus::Cancelled,
        };
    }
}

struct ObservationRuntime {
    next_handle: u64,
    sessions: HashMap<u64, InterpreterObservationSession>,
}

impl ObservationRuntime {
    fn new() -> Self {
        Self {
            next_handle: 1,
            sessions: HashMap::new(),
        }
    }

    fn insert(&mut self, session: InterpreterObservationSession) -> Result<Value, String> {
        let handle = self.next_handle;
        self.next_handle = self
            .next_handle
            .checked_add(1)
            .filter(|value| *value <= MAX_SAFE_INTEGER)
            .ok_or_else(|| "INTERPRETER_OBSERVATION_HANDLES_EXHAUSTED".to_string())?;
        let value = session_info(handle, &session);
        self.sessions.insert(handle, session);
        Ok(value)
    }

    fn session(&self, handle: u64) -> Result<&InterpreterObservationSession, String> {
        self.sessions
            .get(&handle)
            .ok_or_else(|| format!("NO_INTERPRETER_OBSERVATION_SESSION {handle}"))
    }

    fn session_mut(&mut self, handle: u64) -> Result<&mut InterpreterObservationSession, String> {
        self.sessions
            .get_mut(&handle)
            .ok_or_else(|| format!("NO_INTERPRETER_OBSERVATION_SESSION {handle}"))
    }

    fn dispatch(&mut self, request: &Value) -> Result<Value, String> {
        let operation = required_string(request, "op")?;
        match operation.as_str() {
            "start" => {
                let session_id = required_string(request, "sessionId")?;
                let source_id = required_string(request, "sourceId")?;
                let source = required_string(request, "source")?;
                self.insert(InterpreterObservationSession::start_named(
                    session_id, source_id, source,
                )?)
            }
            "info" => {
                let handle = required_handle(request)?;
                Ok(session_info(handle, self.session(handle)?))
            }
            "snapshot" => self.session(required_handle(request)?)?.snapshot(),
            "step" => self.session_mut(required_handle(request)?)?.step(),
            "run" => {
                let handle = required_handle(request)?;
                let limit = bounded_usize(request, "boundaryLimit", MAX_RUN_BOUNDARIES)?;
                self.session_mut(handle)?.run(limit)
            }
            "resume" => {
                let handle = required_handle(request)?;
                let settlement = settlement_state(field(request, "settlement"))?;
                self.session_mut(handle)?.resume(settlement)
            }
            "resolve-suspension" => {
                let handle = required_handle(request)?;
                let value = field(request, "value")
                    .ok_or_else(|| "interpreter observation resolve requires value".to_string())?;
                self.session(handle)?
                    .resolve_suspension(value)
                    .map(Value::Bool)
            }
            "reject-suspension" => {
                let handle = required_handle(request)?;
                let error = field(request, "error")
                    .ok_or_else(|| "interpreter observation reject requires error".to_string())?;
                self.session(handle)?
                    .reject_suspension(error)
                    .map(Value::Bool)
            }
            "suspension-state" => self.session(required_handle(request)?)?.suspension_state(),
            "history" => self.session(required_handle(request)?)?.history(),
            "reset" => self.session_mut(required_handle(request)?)?.reset(),
            "cancel" => self.session_mut(required_handle(request)?)?.cancel(),
            "result-display" => self.session(required_handle(request)?)?.result_display(),
            "error-message" => self.session(required_handle(request)?)?.error_message(),
            "set-observation-limits" => {
                let handle = required_handle(request)?;
                let bindings = bounded_usize(request, "bindings", MAX_BINDINGS)?;
                let display_chars = bounded_usize(request, "displayChars", MAX_DISPLAY_CHARS)?;
                Ok(self
                    .session_mut(handle)?
                    .set_observation_limits(bindings, display_chars))
            }
            "set-retention-limits" => {
                let handle = required_handle(request)?;
                let history = bounded_usize(request, "history", MAX_RETAINED_BOUNDARIES)?;
                Ok(self.session_mut(handle)?.set_history_limit(history))
            }
            "dispose" => {
                let handle = required_handle(request)?;
                let mut session = self
                    .sessions
                    .remove(&handle)
                    .ok_or_else(|| format!("NO_INTERPRETER_OBSERVATION_SESSION {handle}"))?;
                Ok(Value::Bool(session.dispose()))
            }
            "dispose-all" => {
                let count = self.sessions.len();
                for (_, mut session) in self.sessions.drain() {
                    session.dispose();
                }
                Ok(Value::Number(safe_i64(count as u64)))
            }
            other => Err(format!("UNKNOWN_INTERPRETER_OBSERVATION_OPERATION {other}")),
        }
    }
}

impl Drop for ObservationRuntime {
    fn drop(&mut self) {
        // The raw ABI is module-lifetime and exposes explicit disposal. TLS
        // teardown order relative to namespace and semantic-context TLS is not
        // stable, so undisposed sessions are intentionally not re-entered here.
        let sessions = std::mem::take(&mut self.sessions);
        std::mem::forget(sessions);
    }
}

thread_local! {
    static RUNTIME: RefCell<ObservationRuntime> = RefCell::new(ObservationRuntime::new());
}

pub fn invoke_json(source: &str) -> Vec<u8> {
    let result = json::read(source)
        .map_err(|error| format!("INTERPRETER_OBSERVATION_REQUEST_INVALID {error}"))
        .and_then(|request| RUNTIME.with(|runtime| runtime.borrow_mut().dispatch(&request)));
    encode_response(result)
}

fn encode_response(result: Result<Value, String>) -> Vec<u8> {
    let value = match result {
        Ok(value) => object([("ok", Value::Bool(true)), ("value", value)]),
        Err(message) => object([
            ("ok", Value::Bool(false)),
            (
                "error",
                object([
                    ("code", Value::String(error_code(&message).into())),
                    ("message", Value::String(message)),
                ]),
            ),
        ]),
    };
    json::write(&value)
        .unwrap_or_else(|_| {
            "{\"ok\":false,\"error\":{\"code\":\"interpreter-observation/encode\",\"message\":\"unable to encode observation response\"}}".into()
        })
        .into_bytes()
}

fn error_code(message: &str) -> &'static str {
    if message.starts_with("NO_INTERPRETER_OBSERVATION_SESSION") {
        "interpreter-observation/no-session"
    } else if message.starts_with("UNKNOWN_INTERPRETER_OBSERVATION_OPERATION") {
        "interpreter-observation/unknown-operation"
    } else if message.contains("parse") || message.contains("reader") {
        "interpreter-observation/reader"
    } else if message.contains("SUSPEND") || message.contains("PROMISE") {
        "interpreter-observation/suspension"
    } else if message.contains("DISPOSED") {
        "interpreter-observation/disposed"
    } else {
        "interpreter-observation/error"
    }
}

fn session_info(handle: u64, session: &InterpreterObservationSession) -> Value {
    object([
        ("handle", Value::Number(safe_i64(handle))),
        ("sessionId", Value::String(session.session_id().to_owned())),
        ("sourceId", Value::String(session.source_id().to_owned())),
        ("generation", Value::Number(safe_i64(session.generation()))),
        ("sequence", Value::Number(safe_i64(session.sequence()))),
        (
            "status",
            Value::String(session.status().as_keyword().into()),
        ),
        (
            "retained",
            Value::Number(safe_i64(session.retained_history() as u64)),
        ),
        (
            "dropped",
            Value::Number(safe_i64(session.dropped_history())),
        ),
    ])
}

fn settlement_state(value: Option<Value>) -> Result<Option<PromiseState>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    if matches!(value, Value::Nil) {
        return Ok(None);
    }
    let status = required_string(&value, "status")?;
    match status.as_str() {
        "pending" => Ok(Some(PromiseState::Pending)),
        "fulfilled" => Ok(Some(PromiseState::Fulfilled(
            field(&value, "value").unwrap_or(Value::Nil),
        ))),
        "rejected" => Ok(Some(PromiseState::Rejected(PromiseRejection::Value(
            field(&value, "error").unwrap_or(Value::Nil),
        )))),
        other => Err(format!(
            "unsupported interpreter observation settlement status: {other}"
        )),
    }
}

fn required_id(value: String, label: &str) -> Result<String, String> {
    if value.trim().is_empty() {
        Err(format!("interpreter observation {label} must not be empty"))
    } else {
        Ok(value)
    }
}

fn required_handle(request: &Value) -> Result<u64, String> {
    match field(request, "handle") {
        Some(Value::Number(value)) if value > 0 => Ok(value as u64),
        _ => Err("interpreter observation request requires a positive handle".into()),
    }
}

fn required_string(request: &Value, name: &str) -> Result<String, String> {
    match field(request, name) {
        Some(Value::String(value)) if !value.trim().is_empty() => Ok(value),
        _ => Err(format!(
            "interpreter observation request requires non-empty {name}"
        )),
    }
}

fn bounded_usize(request: &Value, name: &str, maximum: usize) -> Result<usize, String> {
    match field(request, name) {
        Some(Value::Number(value)) if value >= 0 && value as u64 <= maximum as u64 => {
            Ok(value as usize)
        }
        _ => Err(format!(
            "interpreter observation {name} must be between 0 and {maximum}"
        )),
    }
}

fn field(value: &Value, name: &str) -> Option<Value> {
    core::map_entries(value)?
        .iter()
        .find_map(|(key, value)| match key {
            Value::String(key) if key == name => Some(value.clone()),
            Value::Keyword(key) if key.as_str() == name => Some(value.clone()),
            _ => None,
        })
}

fn object<const N: usize>(fields: [(&str, Value); N]) -> Value {
    Value::Map(
        fields
            .into_iter()
            .map(|(key, value)| (Value::String(key.into()), value))
            .collect(),
    )
}

fn safe_i64(value: u64) -> i64 {
    value.min(MAX_SAFE_INTEGER) as i64
}

#[cfg(test)]
mod tests {
    use super::{field, invoke_json};
    use crate::core::Value;
    use crate::json;

    fn invoke(request: &str) -> Value {
        let bytes = invoke_json(request);
        let response = json::read(std::str::from_utf8(&bytes).unwrap()).unwrap();
        assert_eq!(field(&response, "ok"), Some(Value::Bool(true)), "{request}");
        field(&response, "value").unwrap()
    }

    fn handle(value: &Value) -> i64 {
        match field(value, "handle") {
            Some(Value::Number(handle)) => handle,
            other => panic!("expected interpreter observation handle, got {other:?}"),
        }
    }

    #[test]
    fn live_session_runs_real_evaluation_and_retains_bounded_history() {
        let info = invoke(
            r#"{"op":"start","sessionId":"fixture/session","sourceId":"example/core.hal","source":"(+ 1 (* 2 3))"}"#,
        );
        let handle = handle(&info);
        invoke(&format!(
            "{{\"op\":\"set-retention-limits\",\"handle\":{handle},\"history\":8}}"
        ));
        let run = invoke(&format!(
            "{{\"op\":\"run\",\"handle\":{handle},\"boundaryLimit\":1000}}"
        ));
        assert_eq!(
            field(&run, "schema"),
            Some(Value::String(
                "hal.interpreter-observation-run/0-alpha".into()
            ))
        );
        assert_eq!(
            invoke(&format!(
                "{{\"op\":\"result-display\",\"handle\":{handle}}}"
            )),
            Value::String("7".into())
        );
        let history = invoke(&format!("{{\"op\":\"history\",\"handle\":{handle}}}"));
        assert_eq!(
            field(&history, "schema"),
            Some(Value::String(
                "hal.interpreter-observation-history/0-alpha".into()
            ))
        );
        assert!(
            matches!(field(&history, "retained"), Some(Value::Number(value)) if value > 0 && value <= 8)
        );
        assert_eq!(field(&history, "dropped"), Some(Value::Number(0)));
        let encoded = json::write(&history).unwrap();
        assert!(encoded.contains("(* 2 3)"));
        assert!(encoded.contains("\"path\":[0,2]"));
    }

    #[test]
    fn nested_closure_history_contains_live_lexical_frame_values() {
        let info = invoke(
            r#"{"op":"start","sessionId":"fixture/closure","sourceId":"closure.hal","source":"(let [x 10 f (fn [y] (+ x y))] (f 32))"}"#,
        );
        let handle = handle(&info);
        invoke(&format!(
            "{{\"op\":\"run\",\"handle\":{handle},\"boundaryLimit\":5000}}"
        ));
        assert_eq!(
            invoke(&format!(
                "{{\"op\":\"result-display\",\"handle\":{handle}}}"
            )),
            Value::String("42".into())
        );
        let history = invoke(&format!("{{\"op\":\"history\",\"handle\":{handle}}}"));
        let encoded = json::write(&history).unwrap();
        assert!(encoded.contains("\"name\":\"x\""));
        assert!(encoded.contains("\"display\":\"10\""));
        assert!(encoded.contains("\"name\":\"y\""));
        assert!(encoded.contains("\"display\":\"32\""));
    }

    #[test]
    fn pending_promise_can_be_settled_and_resumed() {
        let info = invoke(
            r#"{"op":"start","sessionId":"fixture/await","sourceId":"await.hal","source":"(Coroutine/await (promise/new (fn [resolve reject] nil)))"}"#,
        );
        let handle = handle(&info);
        let run = invoke(&format!(
            "{{\"op\":\"run\",\"handle\":{handle},\"boundaryLimit\":1000}}"
        ));
        assert_eq!(
            field(&run, "status"),
            Some(Value::String("suspended".into()))
        );
        assert_eq!(
            invoke(&format!(
                "{{\"op\":\"resolve-suspension\",\"handle\":{handle},\"value\":42}}"
            )),
            Value::Bool(true)
        );
        invoke(&format!("{{\"op\":\"resume\",\"handle\":{handle}}}"));
        invoke(&format!(
            "{{\"op\":\"run\",\"handle\":{handle},\"boundaryLimit\":1000}}"
        ));
        assert_eq!(
            invoke(&format!(
                "{{\"op\":\"result-display\",\"handle\":{handle}}}"
            )),
            Value::String("42".into())
        );
    }

    #[test]
    fn reset_rebuilds_isolated_runtime_and_deep_loop_history_stays_bounded() {
        let info = invoke(
            r#"{"op":"start","sessionId":"fixture/reset","sourceId":"reset.hal","source":"(do (def counter 1) (set! counter 42) (loop [i 0] (if (< i 100) (recur (+ i 1)) counter)))"}"#,
        );
        let handle = handle(&info);
        invoke(&format!(
            "{{\"op\":\"set-retention-limits\",\"handle\":{handle},\"history\":4}}"
        ));
        invoke(&format!(
            "{{\"op\":\"run\",\"handle\":{handle},\"boundaryLimit\":100000}}"
        ));
        assert_eq!(
            invoke(&format!(
                "{{\"op\":\"result-display\",\"handle\":{handle}}}"
            )),
            Value::String("42".into())
        );
        let history = invoke(&format!("{{\"op\":\"history\",\"handle\":{handle}}}"));
        assert_eq!(field(&history, "retained"), Some(Value::Number(4)));
        assert!(matches!(field(&history, "dropped"), Some(Value::Number(value)) if value > 100));

        let reset = invoke(&format!("{{\"op\":\"reset\",\"handle\":{handle}}}"));
        assert_eq!(field(&reset, "generation"), Some(Value::Number(1)));
        assert_eq!(field(&reset, "status"), Some(Value::String("ready".into())));
        let after = invoke(&format!("{{\"op\":\"history\",\"handle\":{handle}}}"));
        assert_eq!(field(&after, "retained"), Some(Value::Number(0)));
        assert_eq!(field(&after, "dropped"), Some(Value::Number(0)));
    }

    #[test]
    fn cancellation_and_disposal_release_session_ownership() {
        let info = invoke(
            r#"{"op":"start","sessionId":"fixture/dispose","sourceId":"dispose.hal","source":"(+ 1 2)"}"#,
        );
        let handle = handle(&info);
        let cancelled = invoke(&format!("{{\"op\":\"cancel\",\"handle\":{handle}}}"));
        assert_eq!(
            field(&cancelled, "status"),
            Some(Value::String("cancelled".into()))
        );
        assert_eq!(
            invoke(&format!("{{\"op\":\"dispose\",\"handle\":{handle}}}")),
            Value::Bool(true)
        );
        let bytes = invoke_json(&format!("{{\"op\":\"snapshot\",\"handle\":{handle}}}"));
        let response = json::read(std::str::from_utf8(&bytes).unwrap()).unwrap();
        assert_eq!(field(&response, "ok"), Some(Value::Bool(false)));
        let error = field(&response, "error").unwrap();
        assert_eq!(
            field(&error, "code"),
            Some(Value::String("interpreter-observation/no-session".into()))
        );
    }
}
