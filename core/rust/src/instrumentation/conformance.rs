//! Production-backed instrumentation conformance reports.
//!
//! The corpus is an executable contract. `capture` names the events to
//! subscribe to; it is never treated as a stream of events to replay. The
//! execution cases below drive the same `EvalFiber` and `Machine` types used
//! by the runtime, while the hub cases exercise registration and delivery
//! directly. The report deliberately exposes only portable observations so
//! Rust, Java/Truffle, and browser/Wasm can be compared without inventing
//! backend-specific event payloads.

use std::collections::{BTreeSet, HashMap};

use serde_json::{json, Map, Value};

use crate::live_session::LiveSession;

use super::{
    Capability, EventAccess, EventDelivery, EventKind, EventPhase, InstrumentFilter,
    InstrumentMode, InstrumentRegistration, InstrumentationError, InstrumentationHub,
    ProducerEvent, ProjectionLimits, ProjectionRequest, RuntimeBackend, TargetDescriptor,
    TargetKind,
};

const CORPUS_SCHEMA: &str = "hara.instrumentation.conformance-corpus/1";
const REPORT_SCHEMA: &str = "hara.instrumentation.conformance-report/1";
const SESSION_ID: &str = "instrum-freeze";

/// Produces one deterministic conformance report from the shared executable
/// corpus.
pub fn report(corpus: &Value, runtime: &str) -> Result<Value, String> {
    if corpus.get("schema").and_then(Value::as_str) != Some(CORPUS_SCHEMA) {
        return Err("unsupported instrumentation corpus schema".into());
    }
    let cases = corpus
        .get("cases")
        .and_then(Value::as_array)
        .ok_or("instrumentation corpus cases must be an array")?;
    let mut ids = BTreeSet::new();
    let mut observed = Vec::with_capacity(cases.len());
    for case in cases {
        let id = string(case, "id")?.to_owned();
        if !ids.insert(id.clone()) {
            return Err(format!("duplicate instrumentation corpus case {id}"));
        }
        observed.push(observe_case(case)?);
    }
    Ok(json!({
        "schema": REPORT_SCHEMA,
        "corpus": {
            "schema": corpus["schema"],
            "id": corpus["id"]
        },
        "runtime": runtime,
        "cases": observed
    }))
}

fn observe_case(case: &Value) -> Result<Value, String> {
    match string(case, "kind")? {
        "execution" => observe_execution(case),
        "hub" => observe_hub(case),
        "live-session" => observe_live_session(case),
        "code-vm" => observe_code_vm(case),
        other => Err(format!(
            "{}: unsupported conformance case kind {other}",
            string(case, "id")?
        )),
    }
}

fn observe_execution(case: &Value) -> Result<Value, String> {
    let id = string(case, "id")?;
    let target_kind = parse_target_kind(string(case, "targetKind")?)?;
    if target_kind == TargetKind::WholeWasm {
        return Err(format!(
            "{id}: whole-wasm execution must use the artifact conformance lane"
        ));
    }
    let events = parse_event_set(case.get("capture"), id)?;
    let projection = parse_projection(case.get("projection"), id)?;
    let mut capabilities = events
        .iter()
        .map(|event| event.required_capability())
        .collect::<BTreeSet<_>>();
    capabilities.extend(projection.required_capabilities());
    let target_capabilities = match target_kind {
        TargetKind::Interpreter => super::interpreter_capabilities(),
        #[cfg(all(feature = "bytecode-vm", feature = "bytecode-instrumentation"))]
        TargetKind::Hbc => super::hbc_capabilities(),
        #[cfg(not(all(feature = "bytecode-vm", feature = "bytecode-instrumentation")))]
        TargetKind::Hbc => {
            return Err(format!(
                "{id}: HBC instrumentation requires bytecode-vm and bytecode-instrumentation"
            ))
        }
        TargetKind::WholeWasm => unreachable!(),
    };
    let target_id = format!("{id}/target");
    let instrument_id = format!("{id}/instrument");
    let queue_capacity = case
        .get("queueCapacity")
        .map(|value| number(value, "queueCapacity"))
        .transpose()?
        .unwrap_or(256) as usize;
    if queue_capacity == 0 {
        return Err(format!("{id}: queueCapacity must be positive"));
    }
    let source_id = string(case, "sourceId")?;
    let source = string(case, "source")?;
    let mut hub = InstrumentationHub::new();
    let target = hub
        .register_target(TargetDescriptor {
            target_id,
            session_id: SESSION_ID.into(),
            kind: target_kind,
            backend: RuntimeBackend::new("rust").map_err(str::to_owned)?,
            capabilities: target_capabilities,
        })
        .map_err(|error| format!("{id}: target registration failed: {error}"))?;
    let instrument = hub
        .register(InstrumentRegistration {
            instrument_id,
            session_id: SESSION_ID.into(),
            mode: InstrumentMode::Passive,
            capabilities,
            events,
            filter: InstrumentFilter::default(),
            projection,
            delivery: EventDelivery::Queue {
                capacity: queue_capacity,
            },
        })
        .map_err(|error| format!("{id}: instrument registration failed: {error}"))?;
    hub.attach(&instrument, &target)
        .map_err(|error| format!("{id}: attachment failed: {error}"))?;

    let (status, result_type, result_display) = match target_kind {
        TargetKind::Interpreter => {
            let environment = parse_environment(case.get("environment"), id)?;
            let mut target = super::InterpreterTarget::start(
                &hub,
                target.clone(),
                source_id,
                source,
                environment,
            )
            .map_err(|error| format!("{id}: interpreter start failed: {error}"))?;
            target
                .run(&mut hub, boundary_limit(case, id)?)
                .map_err(|error| format!("{id}: interpreter execution failed: {error}"))?;
            match target.state() {
                crate::core::EvalFiberState::Completed(value) => (
                    "returned",
                    Some(crate::core::portable_type_name(&value).to_owned()),
                    Some(value.display()),
                ),
                crate::core::EvalFiberState::Failed(_) => ("failed", None, None),
                crate::core::EvalFiberState::Cancelled => ("cancelled", None, None),
                crate::core::EvalFiberState::Running => ("running", None, None),
                crate::core::EvalFiberState::Suspended => ("suspended", None, None),
            }
        }
        #[cfg(all(feature = "bytecode-vm", feature = "bytecode-instrumentation"))]
        TargetKind::Hbc => {
            let program = if case.get("program").is_some() {
                parse_program(case.get("program"), id)?
            } else {
                crate::vm::compile_source(source)
                    .map_err(|error| format!("{id}: HBC compilation failed: {error}"))?
            };
            let mut target = super::HbcTarget::new(
                &hub,
                target.clone(),
                source_id,
                crate::vm::Machine::entry(std::rc::Rc::new(program)),
            )
            .map_err(|error| format!("{id}: HBC start failed: {error}"))?;
            target
                .run(&mut hub, boundary_limit(case, id)?)
                .map_err(|error| format!("{id}: HBC execution failed: {error}"))?;
            let status = target.status();
            let result = target.result();
            (
                status,
                result
                    .as_ref()
                    .map(|value| crate::core::portable_type_name(value).to_owned()),
                result.map(|value| value.display()),
            )
        }
        #[cfg(not(all(feature = "bytecode-vm", feature = "bytecode-instrumentation")))]
        TargetKind::Hbc => unreachable!(),
        TargetKind::WholeWasm => unreachable!(),
    };

    let batch = hub
        .drain_events(&instrument)
        .map_err(|error| format!("{id}: event drain failed: {error}"))?;
    let summary = event_summary(&batch.events, status, result_type, result_display);
    validate_execution_expectations(case, id, &summary)?;
    Ok(json!({
        "id": id,
        "kind": "execution",
        "targetKind": target_kind.as_str(),
        "observation": summary
    }))
}

fn observe_hub(case: &Value) -> Result<Value, String> {
    let id = string(case, "id")?;
    match string(case, "operation")? {
        "registration-filter-order" => observe_registration_filter_order(id),
        "queue-generation" => observe_queue_generation(id),
        "control-lease" => observe_control_lease(id),
        "unsupported-capability" => observe_unsupported_capability(id),
        "zero-instrument" => observe_zero_instrument(id),
        "session-cleanup" => observe_session_cleanup(id),
        other => Err(format!("{id}: unsupported hub operation {other}")),
    }
}

fn observe_registration_filter_order(id: &str) -> Result<Value, String> {
    let mut hub = InstrumentationHub::new();
    let hbc = register_portable_target(&mut hub, "hub/hbc", TargetKind::Hbc)?;
    let interpreter =
        register_portable_target(&mut hub, "hub/interpreter", TargetKind::Interpreter)?;
    let first = hub_result(hub.register(portable_passive("first", None, 8)))?;
    let second = hub_result(hub.register(portable_passive("second", Some(TargetKind::Hbc), 8)))?;
    hub_result(hub.attach(&first, &hbc))?;
    hub_result(hub.attach(&second, &hbc))?;
    hub_result(hub.attach(&first, &interpreter))?;
    let filtered = hub.attach(&second, &interpreter).is_err();
    let order = hub
        .attachments_for_target(&hbc)
        .map_err(|error| error.to_string())?
        .iter()
        .map(|attachment| attachment.instrument.instrument_id().to_owned())
        .collect::<Vec<_>>();
    let mut access = EmptyAccess;
    hub_result(hub.emit(
        &hbc,
        ProducerEvent::live(EventKind::ExecutionTerminal).with_data("status", "returned"),
        &mut access,
    ))?;
    let first_count = hub_result(hub.drain_events(&first))?.events.len();
    let second_count = hub_result(hub.drain_events(&second))?.events.len();
    Ok(json!({
        "id": id,
        "kind": "hub",
        "operation": "registration-filter-order",
        "attachmentOrder": order,
        "filterRejected": filtered,
        "delivered": {"first": first_count, "second": second_count}
    }))
}

fn observe_queue_generation(id: &str) -> Result<Value, String> {
    let mut hub = InstrumentationHub::new();
    let target = register_portable_target(&mut hub, "hub/queue-target", TargetKind::Hbc)?;
    let instrument = hub_result(hub.register(portable_passive("queue", None, 1)))?;
    hub_result(hub.attach(&instrument, &target))?;
    for status in ["first", "second"] {
        let mut access = EmptyAccess;
        hub_result(hub.emit(
            &target,
            ProducerEvent::live(EventKind::ExecutionTerminal).with_data("status", status),
            &mut access,
        ))?;
    }
    let batch = hub_result(hub.drain_events(&instrument))?;
    let retained = batch.events.first().ok_or("queue case retained no event")?;
    let envelope = canonical_envelope(&retained.envelope);
    let dropped = batch.dropped;
    hub_result(hub.detach(&instrument))?;
    let replacement = hub_result(hub.register(portable_passive("queue", None, 1)))?;
    hub_result(hub.remove_target(&target))?;
    let replacement_target =
        register_portable_target(&mut hub, "hub/queue-target", TargetKind::Hbc)?;
    Ok(json!({
        "id": id,
        "kind": "hub",
        "operation": "queue-generation",
        "dropped": dropped,
        "retained": envelope,
        "instrumentGeneration": replacement.generation(),
        "targetGeneration": replacement_target.generation()
    }))
}

fn observe_control_lease(id: &str) -> Result<Value, String> {
    let mut hub = InstrumentationHub::new();
    let target = hub
        .register_target(TargetDescriptor {
            target_id: "hub/lease-target".into(),
            session_id: SESSION_ID.into(),
            kind: TargetKind::Hbc,
            backend: RuntimeBackend::new("portable").expect("portable backend"),
            capabilities: BTreeSet::from([Capability::EventLifecycle, Capability::ControlPause]),
        })
        .map_err(|error| error.to_string())?;
    let first = hub_result(hub.register(portable_control("lease-first")))?;
    let second = hub_result(hub.register(portable_control("lease-second")))?;
    hub_result(hub.attach(&first, &target))?;
    hub_result(hub.attach(&second, &target))?;
    hub_result(hub.acquire_control(&first, &target))?;
    let error = hub
        .acquire_control(&second, &target)
        .expect_err("second control lease must fail");
    let (code, holder) = match error {
        InstrumentationError::ControlLeaseHeld { holder, .. } => {
            ("control-lease-conflict", Some(holder))
        }
        other => return Err(format!("{id}: unexpected lease error {other:?}")),
    };
    Ok(json!({
        "id": id,
        "kind": "hub",
        "operation": "control-lease",
        "error": {"code": code, "holder": holder}
    }))
}

fn observe_unsupported_capability(id: &str) -> Result<Value, String> {
    let mut hub = InstrumentationHub::new();
    let target = hub
        .register_target(TargetDescriptor {
            target_id: "hub/unsupported-target".into(),
            session_id: SESSION_ID.into(),
            kind: TargetKind::Interpreter,
            backend: RuntimeBackend::new("portable").expect("portable backend"),
            capabilities: BTreeSet::new(),
        })
        .map_err(|error| error.to_string())?;
    let instrument = hub_result(hub.register(portable_passive("unsupported", None, 8)))?;
    let error = hub
        .attach(&instrument, &target)
        .expect_err("attachment must fail");
    let (target_id, backend, missing) = match error {
        InstrumentationError::UnsupportedCapabilities {
            target_id,
            backend,
            missing,
        } => (target_id, backend.as_str().to_owned(), missing),
        other => return Err(format!("{id}: unexpected capability error {other:?}")),
    };
    Ok(json!({
        "id": id,
        "kind": "hub",
        "operation": "unsupported-capability",
        "error": {
            "code": "unsupported-capabilities",
            "target": target_id,
            "backend": backend,
            "requested": ["event-lifecycle"],
            "potential": [],
            "missing": missing.iter().map(|capability| capability_name(*capability)).collect::<Vec<_>>()
        }
    }))
}

fn observe_zero_instrument(id: &str) -> Result<Value, String> {
    let mut hub = InstrumentationHub::new();
    let target = register_portable_target(&mut hub, "hub/zero-target", TargetKind::Interpreter)?;
    hub_result(hub.register(portable_passive("zero", None, 8)))?;
    Ok(json!({
        "id": id,
        "kind": "hub",
        "operation": "zero-instrument",
        "enabled": hub_result(hub.enabled_for_target(&target, EventKind::ExecutionTerminal))?,
        "instrumentCount": hub.registration_count(),
        "targetCount": hub.target_count(),
        "attachmentCount": hub.attachment_count()
    }))
}

fn observe_session_cleanup(id: &str) -> Result<Value, String> {
    let mut hub = InstrumentationHub::new();
    let target = register_portable_target(&mut hub, "hub/cleanup-target", TargetKind::Hbc)?;
    let instrument = hub_result(hub.register(portable_passive("cleanup", None, 8)))?;
    hub_result(hub.attach(&instrument, &target))?;
    let cleanup = hub.detach_session(SESSION_ID);
    Ok(json!({
        "id": id,
        "kind": "hub",
        "operation": "session-cleanup",
        "removed": {"instruments": cleanup.instruments, "targets": cleanup.targets},
        "remaining": {
            "instruments": hub.registration_count(),
            "targets": hub.target_count(),
            "attachments": hub.attachment_count(),
            "eventsEnabled": !hub.enabled_events().is_empty()
        }
    }))
}

fn observe_live_session(case: &Value) -> Result<Value, String> {
    let id = string(case, "id")?;
    let source_id = string(case, "sourceId")?;
    let revision = string(case, "revision")?;
    let source = string(case, "source")?;
    let mut runtime = crate::Runtime::new();
    let live_source = crate::live_session::LiveSource::new(source_id, revision, source)
        .map_err(|error| format!("{id}: live source failed: {error}"))?;
    let mut session = crate::live_session::InstrumentedInterpreterLiveSession::start(
        &runtime,
        SESSION_ID,
        "live",
        live_source,
    )
    .map_err(|error| format!("{id}: live session start failed: {error}"))?;
    let initial = session.state();
    let run = session
        .dispatch(crate::live_session::LiveSessionRequest::for_state(
            "run",
            &initial,
            crate::live_session::LiveSessionCommand::Run {
                boundary_limit: boundary_limit(case, id)?,
            },
        ))
        .map_err(|error| format!("{id}: live session run failed: {error}"))?;
    let reset = session
        .dispatch(crate::live_session::LiveSessionRequest::for_state(
            "reset",
            &run.state,
            crate::live_session::LiveSessionCommand::Reset,
        ))
        .map_err(|error| format!("{id}: live session reset failed: {error}"))?;
    let dispose = session
        .dispatch(crate::live_session::LiveSessionRequest::for_state(
            "dispose",
            &reset.state,
            crate::live_session::LiveSessionCommand::Dispose,
        ))
        .map_err(|error| format!("{id}: live session dispose failed: {error}"))?;
    let run_events = run
        .payload
        .get("events")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    let expected = case.get("expect").and_then(Value::as_object);
    if let Some(expected) = expected {
        expect_string(expected, "runStatus", run.state.status.as_str(), id)?;
        expect_u64(expected, "resetGeneration", reset.state.generation, id)?;
        expect_string(expected, "disposeStatus", dispose.state.status.as_str(), id)?;
    }
    let _ = &mut runtime;
    Ok(json!({
        "id": id,
        "kind": "live-session",
        "backend": "interpreter",
        "initial": state_summary(&initial),
        "run": {
            "status": run.state.status.as_str(),
            "generation": run.state.generation,
            "advanced": run.state.sequence > 0,
            "instrumented": run_events > 0
        },
        "reset": state_summary(&reset.state),
        "dispose": state_summary(&dispose.state)
    }))
}

fn observe_code_vm(case: &Value) -> Result<Value, String> {
    let id = string(case, "id")?;
    #[cfg(feature = "bytecode-vm")]
    {
        let program = parse_program(case.get("program"), id)?;
        let mut machine = crate::vm::Machine::entry(std::rc::Rc::new(program));
        let outcome = machine.run();
        let (status, result_type, result) = match outcome {
            crate::vm::VmOutcome::Returned(value) => (
                "returned",
                Some(crate::core::portable_type_name(&value).to_owned()),
                Some(value.display()),
            ),
            crate::vm::VmOutcome::Failed(_) => ("failed", None, None),
            crate::vm::VmOutcome::Suspended(_) => ("suspended", None, None),
            crate::vm::VmOutcome::Yielded(value) => (
                "yielded",
                Some(crate::core::portable_type_name(&value).to_owned()),
                Some(value.display()),
            ),
        };
        let expected = case.get("expect").and_then(Value::as_object);
        if let Some(expected) = expected {
            expect_string(expected, "status", status, id)?;
            if let Some(expected_type) = expected.get("resultType").and_then(Value::as_str) {
                if result_type.as_deref() != Some(expected_type) {
                    return Err(format!(
                        "{id}: expected resultType {expected_type}, got {result_type:?}"
                    ));
                }
            }
            if let Some(expected_result) = expected.get("result").and_then(Value::as_str) {
                if result.as_deref() != Some(expected_result) {
                    return Err(format!(
                        "{id}: expected result {expected_result}, got {result:?}"
                    ));
                }
            }
        }
        return Ok(json!({
            "id": id,
            "kind": "code-vm",
            "status": status,
            "resultType": result_type,
            "result": result
        }));
    }
    #[cfg(not(feature = "bytecode-vm"))]
    {
        Err(format!("{id}: code-vm feature is required"))
    }
}

fn event_summary(
    events: &[super::DeliveredEvent],
    status: &str,
    result_type: Option<String>,
    result_display: Option<String>,
) -> Value {
    let mut event_set = BTreeSet::new();
    let mut event_order = Vec::new();
    let mut phases = BTreeSet::new();
    let mut generations = BTreeSet::new();
    let mut first_sequence = None;
    let mut strict_sequence = true;
    let mut locations_present = true;
    let mut projections = BTreeSet::new();
    let mut terminal_statuses = BTreeSet::new();
    let mut portable_events = Vec::with_capacity(events.len());
    for (index, delivered) in events.iter().enumerate() {
        let envelope = &delivered.envelope;
        let name = event_name(envelope.event).to_owned();
        if event_set.insert(name.clone()) {
            event_order.push(name.clone());
        }
        phases.insert(phase_name(envelope.phase));
        generations.insert(envelope.generation);
        if index == 0 {
            first_sequence = Some(envelope.sequence);
        } else if envelope.sequence != events[index - 1].envelope.sequence + 1 {
            strict_sequence = false;
        }
        locations_present &= envelope.location.is_some();
        let mut event_projections = Vec::new();
        for (name, present) in [
            (
                "current-frame",
                delivered.projection.current_frame.is_some(),
            ),
            ("frames", delivered.projection.frames.is_some()),
            ("locals", delivered.projection.locals.is_some()),
            ("stack", delivered.projection.stack.is_some()),
            (
                "value-preview",
                delivered.projection.value_preview.is_some(),
            ),
            (
                "machine-snapshot",
                delivered.projection.machine_snapshot.is_some(),
            ),
        ] {
            if present {
                projections.insert(name);
                event_projections.push(name);
            }
        }
        if envelope.event == EventKind::ExecutionTerminal {
            if let Some(status) = envelope.data.get("status") {
                terminal_statuses.insert(normalize_status(status));
            }
        }
        portable_events.push(json!({
            "event": name,
            "phase": phase_name(envelope.phase),
            "location": envelope.location.is_some(),
            "projections": event_projections
        }));
    }
    json!({
        "status": status,
        "resultType": result_type,
        "result": result_display,
        "events": portable_events,
        "eventSet": event_set,
        "eventOrder": event_order,
        "phases": phases,
        "sequence": {
            "first": first_sequence,
            "strict": strict_sequence,
            "generations": generations
        },
        "locations": {
            "present": !events.is_empty() && locations_present,
            "any": events.iter().any(|event| event.envelope.location.is_some())
        },
        "projections": projections,
        "terminal": {
            "count": events.iter().filter(|event| event.envelope.event == EventKind::ExecutionTerminal).count(),
            "statuses": terminal_statuses
        }
    })
}

fn validate_execution_expectations(case: &Value, id: &str, summary: &Value) -> Result<(), String> {
    let Some(expect) = case.get("expect").and_then(Value::as_object) else {
        return Err(format!("{id}: execution case requires expect"));
    };
    let event_set = summary["eventSet"]
        .as_array()
        .ok_or_else(|| format!("{id}: event summary set is invalid"))?;
    if let Some(required) = expect.get("requiredEvents").and_then(Value::as_array) {
        for event in required.iter().filter_map(Value::as_str) {
            if !event_set
                .iter()
                .any(|actual| actual.as_str() == Some(event))
            {
                return Err(format!("{id}: required event {event} was not produced"));
            }
        }
    }
    if let Some(expected) = expect.get("terminalStatus").and_then(Value::as_str) {
        if summary["status"].as_str() != Some(expected) {
            return Err(format!(
                "{id}: expected terminal status {expected}, got {}",
                summary["status"]
            ));
        }
    }
    if let Some(expected) = expect.get("resultType").and_then(Value::as_str) {
        if summary["resultType"].as_str() != Some(expected) {
            return Err(format!(
                "{id}: expected result type {expected}, got {}",
                summary["resultType"]
            ));
        }
    }
    if let Some(expected) = expect.get("minimumEvents").and_then(Value::as_u64) {
        let count = summary["events"].as_array().map_or(0, Vec::len) as u64;
        if count < expected {
            return Err(format!(
                "{id}: expected at least {expected} events, got {count}"
            ));
        }
    }
    if expect.get("locationAll").and_then(Value::as_bool) == Some(true)
        && summary["locations"]["present"] != Value::Bool(true)
    {
        return Err(format!(
            "{id}: requested locations were not present on every event"
        ));
    }
    if expect.get("sequenceStrict").and_then(Value::as_bool) == Some(true)
        && summary["sequence"]["strict"] != Value::Bool(true)
    {
        return Err(format!("{id}: event sequence is not strictly increasing"));
    }
    if let Some(projection_expectations) = expect.get("projections").and_then(Value::as_object) {
        let actual = summary["projections"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        for (name, expected) in projection_expectations {
            if expected.as_bool() == Some(true)
                && !actual.iter().any(|value| value.as_str() == Some(name))
            {
                return Err(format!(
                    "{id}: expected projection {name} was not delivered"
                ));
            }
        }
    }
    Ok(())
}

fn parse_projection(value: Option<&Value>, id: &str) -> Result<ProjectionRequest, String> {
    let Some(value) = value else {
        return Ok(ProjectionRequest::default());
    };
    let object = value
        .as_object()
        .ok_or_else(|| format!("{id}: projection must be an object"))?;
    let limits = ProjectionLimits::default();
    Ok(ProjectionRequest {
        source_location: object
            .get("sourceLocation")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        current_frame: projection_flag(object, "currentFrame", limits),
        frames: projection_flag(object, "frames", limits),
        locals: projection_flag(object, "locals", limits),
        stack: projection_flag(object, "stack", limits),
        value_preview: projection_flag(object, "valuePreview", limits),
        machine_snapshot: projection_flag(object, "machineSnapshot", limits),
    })
}

fn projection_flag(
    object: &Map<String, Value>,
    name: &str,
    limits: ProjectionLimits,
) -> Option<ProjectionLimits> {
    object
        .get(name)
        .and_then(Value::as_bool)
        .filter(|value| *value)
        .map(|_| limits)
}

fn parse_event_set(value: Option<&Value>, id: &str) -> Result<BTreeSet<EventKind>, String> {
    let values = value
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{id}: capture must be an array"))?;
    values
        .iter()
        .map(|value| {
            let name = value
                .as_str()
                .ok_or_else(|| format!("{id}: capture event must be a string"))?;
            parse_event(name)
        })
        .collect()
}

fn parse_environment(
    value: Option<&Value>,
    id: &str,
) -> Result<HashMap<String, crate::core::Value>, String> {
    let Some(value) = value else {
        return Ok(HashMap::new());
    };
    let object = value
        .as_object()
        .ok_or_else(|| format!("{id}: environment must be an object"))?;
    object
        .iter()
        .map(|(name, value)| {
            let value = crate::json::read(&value.to_string())
                .map_err(|error| format!("{id}: environment value {name}: {error}"))?;
            Ok((name.clone(), value))
        })
        .collect()
}

fn boundary_limit(case: &Value, _id: &str) -> Result<usize, String> {
    Ok(case
        .get("boundaryLimit")
        .map(|value| number(value, "boundaryLimit"))
        .transpose()?
        .unwrap_or(512) as usize)
}

fn register_portable_target(
    hub: &mut InstrumentationHub,
    target_id: &str,
    kind: TargetKind,
) -> Result<super::TargetHandle, String> {
    hub.register_target(TargetDescriptor {
        target_id: target_id.into(),
        session_id: SESSION_ID.into(),
        kind,
        backend: RuntimeBackend::new("portable").map_err(str::to_owned)?,
        capabilities: BTreeSet::from([Capability::EventLifecycle]),
    })
    .map_err(|error| error.to_string())
}

fn portable_passive(
    id: &str,
    target_kind: Option<TargetKind>,
    capacity: usize,
) -> InstrumentRegistration {
    InstrumentRegistration {
        instrument_id: id.into(),
        session_id: SESSION_ID.into(),
        mode: InstrumentMode::Passive,
        capabilities: BTreeSet::from([Capability::EventLifecycle]),
        events: BTreeSet::from([EventKind::ExecutionTerminal]),
        filter: InstrumentFilter {
            target_kinds: target_kind.into_iter().collect(),
            ..InstrumentFilter::default()
        },
        projection: ProjectionRequest::default(),
        delivery: EventDelivery::Queue { capacity },
    }
}

fn portable_control(id: &str) -> InstrumentRegistration {
    InstrumentRegistration {
        instrument_id: id.into(),
        session_id: SESSION_ID.into(),
        mode: InstrumentMode::Control,
        capabilities: BTreeSet::from([Capability::EventLifecycle, Capability::ControlPause]),
        events: BTreeSet::from([EventKind::ExecutionTerminal]),
        filter: InstrumentFilter::default(),
        projection: ProjectionRequest::default(),
        delivery: EventDelivery::Queue { capacity: 8 },
    }
}

fn canonical_envelope(envelope: &super::EventEnvelope) -> Value {
    json!({
        "schema": envelope.schema,
        "protocol": envelope.protocol,
        "instrumentId": envelope.instrument_id,
        "runtime": envelope.runtime.as_str(),
        "sessionId": envelope.session_id,
        "targetId": envelope.target_id,
        "targetKind": envelope.target_kind.as_str(),
        "generation": envelope.generation,
        "sequence": envelope.sequence,
        "phase": phase_name(envelope.phase),
        "event": event_name(envelope.event),
        "location": location_value(envelope.location.as_ref()),
        "data": envelope.data
    })
}

fn state_summary(state: &crate::live_session::LiveSessionState) -> Value {
    json!({
        "sourceId": state.source_id,
        "generation": state.generation,
        "status": state.status.as_str(),
        "backend": state.backend.as_str()
    })
}

struct EmptyAccess;

impl EventAccess for EmptyAccess {}

fn hub_result<T>(result: Result<T, InstrumentationError>) -> Result<T, String> {
    result.map_err(|error| error.to_string())
}

fn string<'a>(value: &'a Value, name: &str) -> Result<&'a str, String> {
    value
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing string field {name}"))
}

fn number(value: &Value, name: &str) -> Result<u64, String> {
    value
        .as_u64()
        .ok_or_else(|| format!("{name} must be a non-negative integer"))
}

fn expect_string(
    object: &Map<String, Value>,
    field: &str,
    actual: &str,
    id: &str,
) -> Result<(), String> {
    if let Some(expected) = object.get(field).and_then(Value::as_str) {
        if expected != actual {
            return Err(format!("{id}: expected {field} {expected}, got {actual}"));
        }
    }
    Ok(())
}

fn expect_u64(
    object: &Map<String, Value>,
    field: &str,
    actual: u64,
    id: &str,
) -> Result<(), String> {
    if let Some(expected) = object.get(field).and_then(Value::as_u64) {
        if expected != actual {
            return Err(format!("{id}: expected {field} {expected}, got {actual}"));
        }
    }
    Ok(())
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

fn normalize_status(status: &str) -> String {
    match status {
        "return" => "returned".into(),
        "failure" => "failed".into(),
        other => other.into(),
    }
}

fn capability_name(capability: Capability) -> &'static str {
    match capability {
        Capability::EventSemanticBoundary => "event-semantic-boundary",
        Capability::EventInstruction => "event-instruction",
        Capability::EventCall => "event-call",
        Capability::EventException => "event-exception",
        Capability::EventEffect => "event-effect",
        Capability::EventSuspension => "event-suspension",
        Capability::EventLifecycle => "event-lifecycle",
        Capability::InspectSourceLocation => "inspect-source-location",
        Capability::InspectCurrentFrame => "inspect-current-frame",
        Capability::InspectFrames => "inspect-frames",
        Capability::InspectLocals => "inspect-locals",
        Capability::InspectStack => "inspect-stack",
        Capability::InspectValuePreview => "inspect-value-preview",
        Capability::InspectSnapshot => "inspect-snapshot",
        Capability::ControlPause => "control-pause",
        Capability::ControlSingleStep => "control-single-step",
        Capability::ControlResume => "control-resume",
        Capability::ControlSettle => "control-settle",
        Capability::ControlTerminate => "control-terminate",
        Capability::TransformHalc => "transform-halc",
        Capability::TransformHbc => "transform-hbc",
        Capability::RetransformHalc => "retransform-halc",
        Capability::RetransformHbc => "retransform-hbc",
    }
}

fn location_value(location: Option<&super::EventLocation>) -> Value {
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

#[cfg(feature = "bytecode-vm")]
fn parse_program(value: Option<&Value>, id: &str) -> Result<crate::vm::Program, String> {
    let object = value
        .and_then(Value::as_object)
        .ok_or_else(|| format!("{id}: code-vm case requires program"))?;
    let constants = object
        .get("constants")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{id}: program constants must be an array"))?
        .iter()
        .map(|value| {
            crate::json::read(&value.to_string())
                .map_err(|error| format!("{id}: program constant: {error}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let functions = object
        .get("functions")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{id}: program functions must be an array"))?
        .iter()
        .map(|value| parse_function(value, id))
        .collect::<Result<Vec<_>, _>>()?;
    let entry = object
        .get("entry")
        .map(|value| number(value, "entry"))
        .transpose()?
        .unwrap_or(0);
    let entry = u16::try_from(entry).map_err(|_| format!("{id}: entry is too large"))?;
    Ok(crate::vm::Program {
        namespace: object
            .get("namespace")
            .and_then(Value::as_str)
            .map(str::to_owned),
        constants,
        var_metadata: Vec::new(),
        schema_types: HashMap::new(),
        function_types: HashMap::new(),
        inferred_function_types: HashMap::new(),
        functions,
        entry,
    })
}

#[cfg(feature = "bytecode-vm")]
fn parse_function(value: &Value, id: &str) -> Result<crate::vm::FunctionPrototype, String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("{id}: function must be an object"))?;
    let code = object
        .get("code")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{id}: function code must be an array"))?
        .iter()
        .map(|value| parse_instruction(value, id))
        .collect::<Result<Vec<_>, _>>()?;
    let mut source_map = crate::vm::source_map::SourceMap::default();
    if let Some(positions) = object.get("sourceMap").and_then(Value::as_array) {
        for position in positions {
            let position = match position.as_object() {
                Some(position) => Some(crate::kernel::Position {
                    offset: number(&position["offset"], "source offset")? as usize,
                    line: number(&position["line"], "source line")? as usize,
                    column: number(&position["column"], "source column")? as usize,
                }),
                None => None,
            };
            source_map.record(position);
        }
    }
    let max_stack = object
        .get("maxStack")
        .map(|value| number(value, "maxStack"))
        .transpose()?
        .unwrap_or(4);
    Ok(crate::vm::FunctionPrototype {
        name: object
            .get("name")
            .and_then(Value::as_str)
            .map(str::to_owned),
        async_function: object
            .get("asyncFunction")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        arity: object
            .get("arity")
            .map(|value| number(value, "arity"))
            .transpose()?
            .unwrap_or(0) as u16,
        variadic: object
            .get("variadic")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        capture_count: object
            .get("captureCount")
            .map(|value| number(value, "captureCount"))
            .transpose()?
            .unwrap_or(0) as u16,
        local_count: object
            .get("localCount")
            .map(|value| number(value, "localCount"))
            .transpose()?
            .unwrap_or(0) as u16,
        max_stack: max_stack as u16,
        code,
        source_map,
        handlers: Vec::new(),
    })
}

#[cfg(feature = "bytecode-vm")]
fn parse_instruction(value: &Value, id: &str) -> Result<crate::vm::Instruction, String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("{id}: instruction must be an object"))?;
    let opcode = object
        .get("opcode")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{id}: instruction opcode must be a string"))?;
    let operand = |name: &str| -> Result<u64, String> {
        object
            .get(name)
            .map(|value| number(value, name))
            .transpose()
            .map(|value| value.unwrap_or(0))
    };
    Ok(match opcode {
        "CONSTANT" => crate::vm::Instruction::Constant(operand("first")? as u32),
        "NIL" => crate::vm::Instruction::Nil,
        "TRUE" => crate::vm::Instruction::True,
        "FALSE" => crate::vm::Instruction::False,
        "PRIMITIVE" | "INTRINSIC_CALL" => crate::vm::Instruction::IntrinsicCall {
            target: operand("first")? as u32,
            argc: operand("second")? as u8,
        },
        "RETURN" => crate::vm::Instruction::Return,
        other => return Err(format!("{id}: unsupported program opcode {other}")),
    })
}
