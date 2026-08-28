use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;

use crate::instrumentation::{
    Capability, EventDelivery, EventKind, InstrumentFilter, InstrumentMode, InstrumentRegistration,
    InstrumentationError, InstrumentationHub, ProjectionRequest, RuntimeBackend, TargetDescriptor,
    TargetKind,
};
use crate::live_session::{LiveSessionCommand, LiveSessionRequest, LiveSessionStatus, LiveSource};
use crate::{SessionId, SessionKernel};

use super::*;

fn set<T: Ord>(values: impl IntoIterator<Item = T>) -> BTreeSet<T> {
    values.into_iter().collect()
}

fn target(
    id: &str,
    session_id: &str,
    kind: TargetKind,
    capabilities: impl IntoIterator<Item = Capability>,
) -> TargetDescriptor {
    TargetDescriptor {
        target_id: id.into(),
        session_id: session_id.into(),
        kind,
        backend: RuntimeBackend::new("rust").expect("test backend is valid"),
        capabilities: set(capabilities),
    }
}

fn lifecycle_registration(
    id: &str,
    session_id: &str,
    target_id: &str,
    kind: TargetKind,
) -> InstrumentRegistration {
    InstrumentRegistration {
        instrument_id: id.into(),
        session_id: session_id.into(),
        mode: InstrumentMode::Passive,
        capabilities: set([Capability::EventLifecycle]),
        events: set([EventKind::ExecutionTerminal]),
        filter: InstrumentFilter {
            session_id: Some(session_id.into()),
            target_ids: set([target_id.into()]),
            target_kinds: set([kind]),
            backends: set([RuntimeBackend::new("rust").expect("Rust is a valid backend id")]),
        },
        projection: ProjectionRequest::default(),
        delivery: EventDelivery::Queue { capacity: 16 },
    }
}

fn source(id: &str, revision: &str) -> LiveSource {
    LiveSource::new(id, revision, "(+ 19 23)").expect("test source is valid")
}

fn target_identity(payload: &serde_json::Value) -> (String, u64) {
    let target = payload
        .get("target")
        .and_then(serde_json::Value::as_object)
        .expect("live-session snapshot exposes a bounded target identity");
    let id = target
        .get("id")
        .and_then(serde_json::Value::as_str)
        .expect("target id is text")
        .to_owned();
    let generation = target
        .get("generation")
        .and_then(serde_json::Value::as_u64)
        .expect("target generation is an unsigned integer");
    (id, generation)
}

#[test]
fn trusted_native_target_identity_is_generation_fenced() {
    let hub = Rc::new(RefCell::new(InstrumentationHub::new()));
    let service = NativeInstrumentation::new("session", hub.clone());
    let original = hub
        .borrow_mut()
        .register_target(target(
            "execution",
            "session",
            TargetKind::Interpreter,
            [Capability::EventLifecycle],
        ))
        .expect("original target registration");
    hub.borrow_mut()
        .remove_target(&original)
        .expect("remove original target");
    let replacement = hub
        .borrow_mut()
        .register_target(target(
            "execution",
            "session",
            TargetKind::Interpreter,
            [Capability::EventLifecycle],
        ))
        .expect("replacement target registration");

    assert_eq!(original.generation(), 0);
    assert_eq!(replacement.generation(), 1);
    assert!(matches!(
        service.bind_target_identity("execution", original.generation()),
        Err(NativeInstrumentationError::Hub(
            InstrumentationError::StaleTargetHandle {
                target_id,
                generation: 0,
            }
        )) if target_id == "execution"
    ));
    let bound = service
        .bind_target_identity("execution", replacement.generation())
        .expect("current target identity binds");
    assert_eq!(bound.target_id(), "execution");
    assert_eq!(bound.generation(), 1);
}

#[test]
fn trusted_native_binds_and_observes_real_interpreter_live_target() {
    let mut kernel = SessionKernel::new();
    let session_id = SessionId::parse("trusted-interpreter").unwrap();
    kernel.create_session(session_id.clone()).unwrap();
    let service = kernel
        .instrumentation(&session_id)
        .expect("trusted host service");
    let started = kernel
        .session_mut(&session_id)
        .unwrap()
        .start_interpreter_live_session(
            "debug-session",
            source("interpreter.hal", "sha256:interpreter"),
        )
        .expect("interpreter live session starts");
    let snapshot = kernel
        .session_mut(&session_id)
        .unwrap()
        .dispatch_live_session(LiveSessionRequest::for_state(
            "snapshot-target",
            &started,
            LiveSessionCommand::Snapshot,
        ))
        .expect("snapshot exposes target identity");
    let (target_id, generation) = target_identity(&snapshot.payload);
    let target = service
        .bind_target_identity(target_id.clone(), generation)
        .expect("trusted host binds the real interpreter target");
    let descriptor = service
        .target_descriptor(&target)
        .expect("bounded target descriptor");
    assert_eq!(descriptor.target_id, target_id);
    assert_eq!(descriptor.session_id, session_id.as_str());
    assert_eq!(descriptor.kind, TargetKind::Interpreter);

    let instrument = service
        .register(lifecycle_registration(
            "host/interpreter-terminal",
            session_id.as_str(),
            target.target_id(),
            TargetKind::Interpreter,
        ))
        .expect("passive host instrument registration");
    service
        .attach(&instrument, &target)
        .expect("passive host attaches to interpreter target");

    let run = kernel
        .session_mut(&session_id)
        .unwrap()
        .dispatch_live_session(LiveSessionRequest::for_state(
            "run-interpreter",
            &snapshot.state,
            LiveSessionCommand::Run {
                boundary_limit: 1_000,
            },
        ))
        .expect("interpreter live target runs");
    assert_eq!(run.state.status, LiveSessionStatus::Returned);
    let batch = service
        .drain_events(&instrument)
        .expect("passive host drains its own queue");
    assert!(batch.events.iter().any(|event| {
        event.envelope.event == EventKind::ExecutionTerminal
            && event.envelope.target_id == target.target_id()
            && event.envelope.session_id == session_id.as_str()
            && event.envelope.target_kind == TargetKind::Interpreter
    }));
}

#[cfg(all(feature = "bytecode-observation", feature = "bytecode-instrumentation"))]
#[test]
fn trusted_native_binds_and_observes_real_hbc_live_target() {
    let mut kernel = SessionKernel::new();
    let session_id = SessionId::parse("trusted-hbc").unwrap();
    kernel.create_session(session_id.clone()).unwrap();
    let service = kernel
        .instrumentation(&session_id)
        .expect("trusted host service");
    let started = kernel
        .session_mut(&session_id)
        .unwrap()
        .start_hbc_live_session("debug-session", source("hbc.hal", "sha256:hbc"))
        .expect("HBC live session starts");
    let snapshot = kernel
        .session_mut(&session_id)
        .unwrap()
        .dispatch_live_session(LiveSessionRequest::for_state(
            "snapshot-target",
            &started,
            LiveSessionCommand::Snapshot,
        ))
        .expect("snapshot exposes target identity");
    let (target_id, generation) = target_identity(&snapshot.payload);
    let target = service
        .bind_target_identity(target_id.clone(), generation)
        .expect("trusted host binds the real HBC target");
    let descriptor = service
        .target_descriptor(&target)
        .expect("bounded target descriptor");
    assert_eq!(descriptor.target_id, target_id);
    assert_eq!(descriptor.session_id, session_id.as_str());
    assert_eq!(descriptor.kind, TargetKind::Hbc);

    let instrument = service
        .register(lifecycle_registration(
            "host/hbc-terminal",
            session_id.as_str(),
            target.target_id(),
            TargetKind::Hbc,
        ))
        .expect("passive host instrument registration");
    service
        .attach(&instrument, &target)
        .expect("passive host attaches to HBC target");

    let run = kernel
        .session_mut(&session_id)
        .unwrap()
        .dispatch_live_session(LiveSessionRequest::for_state(
            "run-hbc",
            &snapshot.state,
            LiveSessionCommand::Run {
                boundary_limit: 10_000,
            },
        ))
        .expect("HBC live target runs");
    assert_eq!(run.state.status, LiveSessionStatus::Returned);
    let batch = service
        .drain_events(&instrument)
        .expect("passive host drains its own queue");
    assert!(batch.events.iter().any(|event| {
        event.envelope.event == EventKind::ExecutionTerminal
            && event.envelope.target_id == target.target_id()
            && event.envelope.session_id == session_id.as_str()
            && event.envelope.target_kind == TargetKind::Hbc
    }));
}
