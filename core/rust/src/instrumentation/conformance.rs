//! Executable producers for the shared instrumentation conformance corpus.
//!
//! This module is used by the native report binary and by the browser/Wasm
//! adapter. Keeping the producer here ensures that browser evidence exercises
//! the same authoritative instrumentation hub and state transitions as the
//! native Rust lane.

use std::collections::BTreeSet;

use serde_json::{json, Map, Value};

use super::{
    Capability, EventAccess, EventDelivery, EventKind, EventLocation, EventPhase, InstrumentFilter,
    InstrumentMode, InstrumentRegistration, InstrumentationHub, ProducerEvent, ProjectionRequest,
    RuntimeBackend, SourceSpan, TargetDescriptor, TargetKind,
};

const CORPUS_SCHEMA: &str = "hara.instrumentation.conformance-corpus/0-alpha";
const REPORT_SCHEMA: &str = "hara.instrumentation.conformance-report/0-alpha";

struct FixtureAccess {
    location: Option<EventLocation>,
}

impl EventAccess for FixtureAccess {
    fn source_location(&mut self) -> Option<EventLocation> {
        self.location.clone()
    }
}

/// Produces one deterministic conformance report from the shared corpus.
pub fn report(corpus: &Value, runtime: &str) -> Result<Value, String> {
    if corpus.get("schema").and_then(Value::as_str) != Some(CORPUS_SCHEMA) {
        return Err("unsupported instrumentation corpus schema".into());
    }
    let cases = corpus
        .get("cases")
        .and_then(Value::as_array)
        .ok_or("instrumentation corpus cases must be an array")?
        .iter()
        .map(observe_case)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(json!({
        "schema": REPORT_SCHEMA,
        "corpus": {
            "schema": corpus["schema"],
            "id": corpus["id"]
        },
        "runtime": runtime,
        "cases": cases
    }))
}

fn observe_case(case: &Value) -> Result<Value, String> {
    let id = string(case, "id")?;
    if string(case, "kind")? == "state" {
        return observe_state(case, &id);
    }
    let target_kind = parse_target_kind(string(case, "targetKind")?)?;
    let events = case["events"]
        .as_array()
        .ok_or_else(|| format!("{id}: events must be an array"))?;
    let mut event_kinds = BTreeSet::new();
    for event in events {
        event_kinds.insert(parse_event(string(event, "event")?)?);
    }
    let mut capabilities = event_kinds
        .iter()
        .map(|event| event.required_capability())
        .collect::<BTreeSet<_>>();
    capabilities.insert(Capability::InspectSourceLocation);
    let target_id = format!("{id}/target");
    let session_id = "instrum-alpha";
    let mut hub = InstrumentationHub::new();
    let target = hub
        .register_target(TargetDescriptor {
            target_id: target_id.clone(),
            session_id: session_id.into(),
            kind: target_kind,
            backend: RuntimeBackend::new("rust").map_err(str::to_owned)?,
            capabilities: capabilities.clone(),
        })
        .map_err(|error| format!("{id}: target registration failed: {error:?}"))?;
    let instrument = hub
        .register(InstrumentRegistration {
            instrument_id: format!("{id}/instrument"),
            session_id: session_id.into(),
            mode: InstrumentMode::Passive,
            capabilities,
            events: event_kinds,
            filter: InstrumentFilter::default(),
            projection: ProjectionRequest {
                source_location: true,
                ..ProjectionRequest::default()
            },
            delivery: EventDelivery::Queue { capacity: 32 },
        })
        .map_err(|error| format!("{id}: instrument registration failed: {error:?}"))?;
    hub.attach(&instrument, &target)
        .map_err(|error| format!("{id}: attachment failed: {error:?}"))?;
    for event in events {
        let mut access = FixtureAccess {
            location: parse_location(event.get("location"))?,
        };
        let producer = ProducerEvent {
            phase: parse_phase(string(event, "phase")?)?,
            event: parse_event(string(event, "event")?)?,
            data: parse_data(event.get("data"))?,
        };
        hub.emit(&target, producer, &mut access)
            .map_err(|error| format!("{id}: event delivery failed: {error:?}"))?;
    }
    let batch = hub
        .drain_events(&instrument)
        .map_err(|error| format!("{id}: event drain failed: {error:?}"))?;
    let actual = batch
        .events
        .iter()
        .map(|event| {
            json!({
                "event": event_name(event.envelope.event),
                "phase": phase_name(event.envelope.phase),
                "generation": event.envelope.generation,
                "sequence": event.envelope.sequence,
                "location": location_value(event.envelope.location.as_ref()),
                "data": event.envelope.data
            })
        })
        .collect::<Vec<_>>();
    let expected = events
        .iter()
        .map(canonical_event)
        .collect::<Result<Vec<_>, _>>()?;
    if actual != expected {
        return Err(format!("{id}: produced events differ from corpus"));
    }
    Ok(json!({
        "id": id,
        "kind": "events",
        "targetKind": target_kind.as_str(),
        "events": actual
    }))
}

fn observe_state(case: &Value, id: &str) -> Result<Value, String> {
    let mut state = case
        .get("initial")
        .and_then(Value::as_object)
        .cloned()
        .ok_or_else(|| format!("{id}: state case requires an initial object"))?;
    let operations = case
        .get("operations")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{id}: state case requires operations"))?;
    for operation in operations {
        apply_state_operation(id, &mut state, operation)?;
    }
    let actual = Value::Object(state);
    if actual != case["state"] {
        return Err(format!("{id}: state transitions differ from corpus"));
    }
    Ok(json!({"id": id, "kind": "state", "state": actual}))
}

fn apply_state_operation(
    id: &str,
    state: &mut Map<String, Value>,
    operation: &Value,
) -> Result<(), String> {
    let name = string(operation, "operation")?;
    match name {
        "run" | "evaluate" => {
            let status = string(operation, "status")?;
            state.insert("status".into(), json!(status));
            state.insert(
                "eventSequence".into(),
                json!(number(
                    operation
                        .get("eventSequence")
                        .ok_or("missing eventSequence")?,
                    "eventSequence"
                )?),
            );
            if let Some(source) = operation.get("source").and_then(Value::as_str) {
                let observed = evaluate_bytecode(source)?;
                let expected = string(operation, "result")?;
                if observed != expected {
                    return Err(format!(
                        "{id}: bytecode result mismatch: expected {expected}, got {observed}"
                    ));
                }
                state.insert("result".into(), json!(observed));
            } else if let Some(result) = operation.get("result") {
                state.insert("result".into(), result.clone());
            }
        }
        "reset" => {
            let generation = state
                .get("generation")
                .and_then(Value::as_u64)
                .ok_or_else(|| format!("{id}: state generation must be unsigned"))?;
            let delta = operation
                .get("generationDelta")
                .map(|value| number(value, "generationDelta"))
                .transpose()?
                .unwrap_or(1);
            state.insert("generation".into(), json!(generation.saturating_add(delta)));
            state.insert("status".into(), json!(string(operation, "status")?));
            state.insert(
                "eventSequence".into(),
                json!(number(
                    operation
                        .get("eventSequence")
                        .ok_or("missing eventSequence")?,
                    "eventSequence"
                )?),
            );
            if operation
                .get("removeResult")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                state.remove("result");
            }
        }
        other => return Err(format!("{id}: unsupported state operation {other}")),
    }
    Ok(())
}

#[cfg(feature = "bytecode-vm")]
fn evaluate_bytecode(source: &str) -> Result<String, String> {
    let program = crate::vm::compile_source(source).map_err(|error| error.to_string())?;
    crate::vm::execute_program(std::rc::Rc::new(program))
        .map(|value| value.display())
        .map_err(|error| error.to_string())
}

#[cfg(not(feature = "bytecode-vm"))]
fn evaluate_bytecode(_source: &str) -> Result<String, String> {
    Err("bytecode-vm feature is required for state conformance".into())
}

fn string<'a>(value: &'a Value, name: &str) -> Result<&'a str, String> {
    value
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing string field {name}"))
}

fn parse_target_kind(value: &str) -> Result<TargetKind, String> {
    match value {
        "interpreter" => Ok(TargetKind::Interpreter),
        "hbc" => Ok(TargetKind::Hbc),
        "whole-wasm" => Ok(TargetKind::WholeWasm),
        _ => Err(format!("unsupported target kind {value}")),
    }
}

fn parse_event(value: &str) -> Result<EventKind, String> {
    match value {
        "semantic-boundary" => Ok(EventKind::SemanticBoundary),
        "instruction-execute" => Ok(EventKind::InstructionExecute),
        "call-enter" => Ok(EventKind::CallEnter),
        "call-return" => Ok(EventKind::CallReturn),
        "exception-raise" => Ok(EventKind::ExceptionRaise),
        "exception-unwind" => Ok(EventKind::ExceptionUnwind),
        "var-set" => Ok(EventKind::VarSet),
        "field-set" => Ok(EventKind::FieldSet),
        "promise-suspend" => Ok(EventKind::PromiseSuspend),
        "promise-resume" => Ok(EventKind::PromiseResume),
        "machine-suspend" => Ok(EventKind::MachineSuspend),
        "machine-resume" => Ok(EventKind::MachineResume),
        "protocol-call" => Ok(EventKind::ProtocolCall),
        "execution-terminal" => Ok(EventKind::ExecutionTerminal),
        _ => Err(format!("unsupported event {value}")),
    }
}

fn parse_phase(value: &str) -> Result<EventPhase, String> {
    match value {
        "live" => Ok(EventPhase::Live),
        "replay" => Ok(EventPhase::Replay),
        _ => Err(format!("unsupported event phase {value}")),
    }
}

fn phase_name(phase: EventPhase) -> &'static str {
    match phase {
        EventPhase::Live => "live",
        EventPhase::Replay => "replay",
    }
}

fn event_name(event: EventKind) -> &'static str {
    match event {
        EventKind::SemanticBoundary => "semantic-boundary",
        EventKind::InstructionExecute => "instruction-execute",
        EventKind::CallEnter => "call-enter",
        EventKind::CallReturn => "call-return",
        EventKind::ExceptionRaise => "exception-raise",
        EventKind::ExceptionUnwind => "exception-unwind",
        EventKind::VarSet => "var-set",
        EventKind::FieldSet => "field-set",
        EventKind::PromiseSuspend => "promise-suspend",
        EventKind::PromiseResume => "promise-resume",
        EventKind::MachineSuspend => "machine-suspend",
        EventKind::MachineResume => "machine-resume",
        EventKind::ProtocolCall => "protocol-call",
        EventKind::ExecutionTerminal => "execution-terminal",
    }
}

fn parse_data(value: Option<&Value>) -> Result<std::collections::BTreeMap<String, String>, String> {
    value
        .unwrap_or(&Value::Object(Map::new()))
        .as_object()
        .ok_or("event data must be an object")?
        .iter()
        .map(|(key, value)| {
            Ok((
                key.clone(),
                value
                    .as_str()
                    .ok_or_else(|| format!("event data {key} must be a string"))?
                    .into(),
            ))
        })
        .collect()
}

fn parse_location(value: Option<&Value>) -> Result<Option<EventLocation>, String> {
    let Some(value) = value else { return Ok(None) };
    let span = value
        .get("span")
        .and_then(Value::as_array)
        .ok_or("location span must be an array")?;
    if span.len() != 2 {
        return Err("location span must contain two values".into());
    }
    let span = SourceSpan {
        start: number(&span[0], "span start")? as usize,
        end: number(&span[1], "span end")? as usize,
    };
    let form_path = value
        .get("formPath")
        .map(|path| {
            path.as_array()
                .ok_or("location formPath must be an array")?
                .iter()
                .map(|item| Ok(number(item, "form path")? as usize))
                .collect::<Result<Vec<_>, String>>()
        })
        .transpose()?;
    Ok(Some(EventLocation {
        source_id: value
            .get("sourceId")
            .and_then(Value::as_str)
            .map(str::to_owned),
        form_path,
        span: Some(span),
        function: value
            .get("function")
            .and_then(Value::as_str)
            .map(str::to_owned),
        instruction_pointer: value
            .get("instructionPointer")
            .map(|item| number(item, "instruction pointer"))
            .transpose()?
            .map(|value| value as usize),
    }))
}

fn number(value: &Value, name: &str) -> Result<u64, String> {
    value
        .as_u64()
        .ok_or_else(|| format!("{name} must be a non-negative integer"))
}

fn canonical_event(event: &Value) -> Result<Value, String> {
    Ok(json!({
        "event": string(event, "event")?,
        "phase": string(event, "phase")?,
        "generation": number(&event["generation"], "generation")?,
        "sequence": number(&event["sequence"], "sequence")?,
        "location": location_value(parse_location(event.get("location"))?.as_ref()),
        "data": parse_data(event.get("data"))?
    }))
}

fn location_value(location: Option<&EventLocation>) -> Value {
    let Some(location) = location else {
        return Value::Null;
    };
    let mut value = Map::new();
    if let Some(source_id) = &location.source_id {
        value.insert("sourceId".into(), json!(source_id));
    }
    if let Some(form_path) = &location.form_path {
        value.insert("formPath".into(), json!(form_path));
    }
    if let Some(span) = &location.span {
        value.insert("span".into(), json!([span.start, span.end]));
    }
    if let Some(function) = &location.function {
        value.insert("function".into(), json!(function));
    }
    if let Some(instruction_pointer) = location.instruction_pointer {
        value.insert("instructionPointer".into(), json!(instruction_pointer));
    }
    Value::Object(value)
}
