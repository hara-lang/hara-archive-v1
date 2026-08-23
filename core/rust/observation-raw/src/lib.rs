use hara_runtime::core::Value;
use hara_runtime::{bytecode_namespace_registry, json, kernel, task, vm};
use std::cell::RefCell;
use std::collections::HashMap;
use task::{PromiseRejection, PromiseState};
use vm::machine::observation::ObservationLimits;
use vm::session::{BytecodeObservationSession, BytecodeSessionError, SessionRetentionLimits};

pub fn embedding_namespace_registry() -> kernel::NamespaceRegistry<Value> {
    bytecode_namespace_registry()
}

const ABI_VERSION: i32 = 1;
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
const MAX_RUN_STEPS: usize = 100_000;
const MAX_SNAPSHOT_ITEMS: usize = 4_096;
const MAX_DISPLAY_CHARS: usize = 16_384;
const MAX_RETAINED_ITEMS: usize = 100_000;

struct ObservationRuntime {
    next_handle: u64,
    sessions: HashMap<u64, BytecodeObservationSession>,
}

impl ObservationRuntime {
    fn new() -> Self {
        Self {
            next_handle: 1,
            sessions: HashMap::new(),
        }
    }

    fn insert(&mut self, session: BytecodeObservationSession) -> Result<Value, String> {
        let handle = self.next_handle;
        self.next_handle = self
            .next_handle
            .checked_add(1)
            .filter(|value| *value <= MAX_SAFE_INTEGER)
            .ok_or_else(|| "BYTECODE_OBSERVATION_HANDLES_EXHAUSTED".to_string())?;
        let value = session_info(handle, &session);
        self.sessions.insert(handle, session);
        Ok(value)
    }

    fn session(&self, handle: u64) -> Result<&BytecodeObservationSession, String> {
        self.sessions
            .get(&handle)
            .ok_or_else(|| format!("NO_BYTECODE_OBSERVATION_SESSION {handle}"))
    }

    fn session_mut(&mut self, handle: u64) -> Result<&mut BytecodeObservationSession, String> {
        self.sessions
            .get_mut(&handle)
            .ok_or_else(|| format!("NO_BYTECODE_OBSERVATION_SESSION {handle}"))
    }

    fn dispatch(&mut self, request: &Value) -> Result<Value, String> {
        let operation = required_string(request, "op")?;
        match operation.as_str() {
            "compile" => {
                let session_id = required_string(request, "sessionId")?;
                let source_id = required_string(request, "sourceId")?;
                let source = required_string(request, "source")?;
                let session =
                    BytecodeObservationSession::compile_named(session_id, source_id, source)
                        .map_err(session_error)?;
                self.insert(session)
            }
            "from-artifact" => {
                let session_id = required_string(request, "sessionId")?;
                let source_id = required_string(request, "sourceId")?;
                let artifact = required_bytes(request, "artifact")?;
                let session = BytecodeObservationSession::from_artifact_named(
                    session_id, source_id, &artifact,
                )
                .map_err(session_error)?;
                self.insert(session)
            }
            "info" => {
                let handle = required_handle(request)?;
                Ok(session_info(handle, self.session(handle)?))
            }
            "snapshot" => self
                .session(required_handle(request)?)?
                .snapshot_value()
                .map_err(session_error),
            "step" => self
                .session_mut(required_handle(request)?)?
                .step()
                .map_err(session_error),
            "run" => {
                let handle = required_handle(request)?;
                let limit = bounded_usize(request, "stepLimit", MAX_RUN_STEPS)?;
                self.session_mut(handle)?.run(limit).map_err(session_error)
            }
            "pause" => {
                let handle = required_handle(request)?;
                Ok(Value::Bool(self.session_mut(handle)?.pause()))
            }
            "resume" => {
                let handle = required_handle(request)?;
                let settlement = settlement_state(field(request, "settlement"))?;
                self.session_mut(handle)?
                    .resume(settlement)
                    .map_err(session_error)
            }
            "resolve-suspension" => {
                let handle = required_handle(request)?;
                let value = field(request, "value")
                    .ok_or_else(|| "bytecode observation resolve requires value".to_string())?;
                self.session(handle)?
                    .resolve_suspension(value)
                    .map(Value::Bool)
                    .map_err(session_error)
            }
            "reject-suspension" => {
                let handle = required_handle(request)?;
                let error = field(request, "error")
                    .ok_or_else(|| "bytecode observation reject requires error".to_string())?;
                self.session(handle)?
                    .reject_suspension(error)
                    .map(Value::Bool)
                    .map_err(session_error)
            }
            "suspension-state" => {
                let handle = required_handle(request)?;
                Ok(self
                    .session(handle)?
                    .suspended_promise()
                    .map(|promise| match promise.state() {
                        PromiseState::Pending => Value::String("pending".into()),
                        PromiseState::Fulfilled(_) => Value::String("fulfilled".into()),
                        PromiseState::Rejected(_) => Value::String("rejected".into()),
                    })
                    .unwrap_or(Value::Nil))
            }
            "reset" => self
                .session_mut(required_handle(request)?)?
                .reset()
                .map_err(session_error),
            "metrics" => Ok(self.session(required_handle(request)?)?.metrics()),
            "events" => Ok(self.session(required_handle(request)?)?.events()),
            "trace" => Ok(self.session(required_handle(request)?)?.trace()),
            "result-display" => Ok(self
                .session(required_handle(request)?)?
                .result()
                .map(|value| Value::String(value.display()))
                .unwrap_or(Value::Nil)),
            "error-message" => Ok(self
                .session(required_handle(request)?)?
                .error()
                .map(|error| Value::String(error.to_string()))
                .unwrap_or(Value::Nil)),
            "set-observation-limits" => {
                let handle = required_handle(request)?;
                let limits = ObservationLimits {
                    stack: bounded_usize(request, "stack", MAX_SNAPSHOT_ITEMS)?,
                    locals: bounded_usize(request, "locals", MAX_SNAPSHOT_ITEMS)?,
                    calls: bounded_usize(request, "calls", MAX_SNAPSHOT_ITEMS)?,
                    handlers: bounded_usize(request, "handlers", MAX_SNAPSHOT_ITEMS)?,
                    display_chars: bounded_usize(request, "displayChars", MAX_DISPLAY_CHARS)?,
                };
                let session = self.session_mut(handle)?;
                session.set_observation_limits(limits);
                Ok(session_info(handle, session))
            }
            "set-retention-limits" => {
                let handle = required_handle(request)?;
                let limits = SessionRetentionLimits {
                    events: bounded_usize(request, "events", MAX_RETAINED_ITEMS)?,
                    trace: bounded_usize(request, "trace", MAX_RETAINED_ITEMS)?,
                };
                let session = self.session_mut(handle)?;
                session.set_retention_limits(limits);
                Ok(session_info(handle, session))
            }
            "dispose" => {
                let handle = required_handle(request)?;
                let mut session = self
                    .sessions
                    .remove(&handle)
                    .ok_or_else(|| format!("NO_BYTECODE_OBSERVATION_SESSION {handle}"))?;
                Ok(Value::Bool(session.dispose()))
            }
            "dispose-all" => {
                let count = self.sessions.len();
                for (_, mut session) in self.sessions.drain() {
                    session.dispose();
                }
                Ok(Value::Number(safe_i64(count as u64)))
            }
            other => Err(format!("UNKNOWN_BYTECODE_OBSERVATION_OPERATION {other}")),
        }
    }
}

impl Drop for ObservationRuntime {
    fn drop(&mut self) {
        // The raw ABI is module-lifetime and exposes explicit dispose APIs.
        // TLS destruction order is not stable relative to Hara registry TLS,
        // so an undisposed session must not re-enter a destroyed registry.
        let sessions = std::mem::take(&mut self.sessions);
        std::mem::forget(sessions);
    }
}

thread_local! {
    static RUNTIME: RefCell<ObservationRuntime> = RefCell::new(ObservationRuntime::new());
}

#[no_mangle]
pub extern "C" fn observation_abi_version() -> i32 {
    ABI_VERSION
}

#[no_mangle]
pub extern "C" fn observation_alloc(size: usize) -> *mut u8 {
    allocate_bytes(size)
}

#[no_mangle]
pub extern "C" fn observation_dealloc(pointer: *mut u8, size: usize) {
    free_bytes(pointer, size);
}

/// Accepts one UTF-8 JSON request and returns a packed `(pointer << 32) | len`
/// response. Every response is a JSON object with either `{ok:true,value:...}`
/// or `{ok:false,error:{code,message}}`.
#[no_mangle]
pub extern "C" fn observation_invoke(pointer: *const u8, size: usize) -> u64 {
    let response = if pointer.is_null() {
        encode_response(Err("bytecode observation request pointer is null".into()))
    } else {
        let bytes = unsafe { std::slice::from_raw_parts(pointer, size) };
        match std::str::from_utf8(bytes) {
            Ok(source) => invoke_json(source),
            Err(_) => encode_response(Err(
                "bytecode observation request must be valid UTF-8".into()
            )),
        }
    };
    pack_response(response)
}

fn invoke_json(source: &str) -> Vec<u8> {
    let result = json::read(source)
        .map_err(|error| format!("BYTECODE_OBSERVATION_REQUEST_INVALID {error}"))
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
            "{\"ok\":false,\"error\":{\"code\":\"bytecode-observation/encode\",\"message\":\"unable to encode observation response\"}}".into()
        })
        .into_bytes()
}

fn error_code(message: &str) -> &'static str {
    if message.starts_with("NO_BYTECODE_OBSERVATION_SESSION") {
        "bytecode-observation/no-session"
    } else if message.starts_with("UNKNOWN_BYTECODE_OBSERVATION_OPERATION") {
        "bytecode-observation/unknown-operation"
    } else if message.contains("compile") || message.contains("parse") {
        "bytecode-observation/compile"
    } else {
        "bytecode-observation/error"
    }
}

fn session_error(error: BytecodeSessionError) -> String {
    error.message().into()
}

fn session_info(handle: u64, session: &BytecodeObservationSession) -> Value {
    object([
        ("handle", Value::Number(safe_i64(handle))),
        ("sessionId", Value::String(session.session_id().to_owned())),
        ("sourceId", Value::String(session.source_id().to_owned())),
        ("traceId", Value::String(session.trace_id().to_owned())),
        (
            "status",
            Value::String(session.status().as_keyword().into()),
        ),
        ("sequence", Value::Number(safe_i64(session.sequence()))),
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
        other => Err(format!("unsupported bytecode settlement status: {other}")),
    }
}

fn required_handle(request: &Value) -> Result<u64, String> {
    match field(request, "handle") {
        Some(Value::Number(value)) if value > 0 => Ok(value as u64),
        _ => Err("bytecode observation request requires a positive handle".into()),
    }
}

fn required_string(request: &Value, name: &str) -> Result<String, String> {
    match field(request, name) {
        Some(Value::String(value)) if !value.trim().is_empty() => Ok(value),
        _ => Err(format!(
            "bytecode observation request requires non-empty {name}"
        )),
    }
}

fn required_bytes(request: &Value, name: &str) -> Result<Vec<u8>, String> {
    match field(request, name) {
        Some(Value::Bytes(bytes)) => Ok(bytes),
        Some(Value::Vector(values)) => values
            .iter()
            .map(|value| match value {
                Value::Number(value) if (0..=255).contains(value) => Ok(*value as u8),
                _ => Err(format!(
                    "bytecode observation {name} must contain byte values"
                )),
            })
            .collect(),
        _ => Err(format!(
            "bytecode observation request requires {name} bytes"
        )),
    }
}

fn bounded_usize(request: &Value, name: &str, maximum: usize) -> Result<usize, String> {
    match field(request, name) {
        Some(Value::Number(value)) if value >= 0 && value as u64 <= maximum as u64 => {
            Ok(value as usize)
        }
        _ => Err(format!(
            "bytecode observation {name} must be between 0 and {maximum}"
        )),
    }
}

fn field(value: &Value, name: &str) -> Option<Value> {
    hara_runtime::core::map_entries(value)?
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

fn allocate_bytes(size: usize) -> *mut u8 {
    let bytes = vec![0_u8; size.max(1)].into_boxed_slice();
    Box::into_raw(bytes) as *mut u8
}

fn free_bytes(pointer: *mut u8, size: usize) {
    if pointer.is_null() {
        return;
    }
    let length = size.max(1);
    unsafe {
        let slice = std::ptr::slice_from_raw_parts_mut(pointer, length);
        drop(Box::from_raw(slice));
    }
}

fn pack_response(bytes: Vec<u8>) -> u64 {
    let length = bytes.len();
    let pointer = Box::into_raw(bytes.into_boxed_slice()) as *mut u8;
    ((pointer as u64) << 32) | length as u64
}

#[cfg(test)]
mod tests {
    use super::{field, invoke_json};
    use hara_runtime::core::Value;
    use hara_runtime::json;

    fn invoke(request: &str) -> Value {
        let bytes = invoke_json(request);
        let response = json::read(std::str::from_utf8(&bytes).unwrap()).unwrap();
        assert_eq!(field(&response, "ok"), Some(Value::Bool(true)), "{request}");
        field(&response, "value").unwrap()
    }

    fn handle(value: &Value) -> i64 {
        match field(value, "handle") {
            Some(Value::Number(handle)) => handle,
            other => panic!("expected observation handle, got {other:?}"),
        }
    }

    #[test]
    fn live_session_runs_the_real_machine_and_emits_versioned_evidence() {
        let info = invoke(
            r#"{"op":"compile","sessionId":"fixture/session","sourceId":"example/core.hal","source":"(+ 1 (* 2 3))"}"#,
        );
        let handle = handle(&info);
        let trace = invoke(&format!(
            "{{\"op\":\"run\",\"handle\":{handle},\"stepLimit\":1000}}"
        ));
        assert_eq!(
            field(&trace, "schema"),
            Some(Value::String("hal.bytecode-trace/0-alpha".into()))
        );
        assert_eq!(
            invoke(&format!(
                "{{\"op\":\"result-display\",\"handle\":{handle}}}"
            )),
            Value::String("7".into())
        );
        let metrics = invoke(&format!("{{\"op\":\"metrics\",\"handle\":{handle}}}"));
        assert_eq!(
            field(&metrics, "schema"),
            Some(Value::String("hal.bytecode-metrics/0-alpha".into()))
        );
        assert!(matches!(field(&metrics, "instructions"), Some(Value::Number(value)) if value > 0));
        let events = invoke(&format!("{{\"op\":\"events\",\"handle\":{handle}}}"));
        assert_eq!(
            field(&events, "schema"),
            Some(Value::String("hal.bytecode-events/0-alpha".into()))
        );
    }

    #[test]
    fn disposal_releases_the_opaque_session_handle() {
        let info = invoke(
            r#"{"op":"compile","sessionId":"fixture/dispose","sourceId":"dispose.hal","source":"(+ 1 2)"}"#,
        );
        let handle = handle(&info);
        assert_eq!(
            invoke(&format!("{{\"op\":\"dispose\",\"handle\":{handle}}}")),
            Value::Bool(true)
        );
        let bytes = invoke_json(&format!("{{\"op\":\"metrics\",\"handle\":{handle}}}"));
        let response = json::read(std::str::from_utf8(&bytes).unwrap()).unwrap();
        assert_eq!(field(&response, "ok"), Some(Value::Bool(false)));
        let error = field(&response, "error").unwrap();
        assert_eq!(
            field(&error, "code"),
            Some(Value::String("bytecode-observation/no-session".into()))
        );
    }
}
