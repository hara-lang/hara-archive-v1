use std::collections::BTreeSet;

use crate::instrumentation::{
    Capability, EventDelivery, EventKind, InstrumentFilter, InstrumentMode, InstrumentRegistration,
    ProjectionRequest, RuntimeBackend, TargetKind,
};
use crate::live_session::{LiveSessionCommand, LiveSessionRequest, LiveSessionStatus, LiveSource};
use crate::{SessionId, SessionKernel};

fn set<T: Ord>(values: impl IntoIterator<Item = T>) -> BTreeSet<T> {
    values.into_iter().collect()
}

fn protocol_call_registration(
    id: &str,
    session_id: &str,
    target_id: &str,
) -> InstrumentRegistration {
    InstrumentRegistration {
        instrument_id: id.into(),
        session_id: session_id.into(),
        mode: InstrumentMode::Passive,
        capabilities: set([
            Capability::EventSemanticBoundary,
            Capability::EventLifecycle,
        ]),
        events: set([EventKind::ProtocolCall, EventKind::ExecutionTerminal]),
        filter: InstrumentFilter {
            session_id: Some(session_id.into()),
            target_ids: set([target_id.into()]),
            target_kinds: set([TargetKind::WholeWasm]),
            backends: set([RuntimeBackend::new("rust").expect("Rust is a valid backend id")]),
        },
        projection: ProjectionRequest::default(),
        delivery: EventDelivery::Queue { capacity: 16 },
    }
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
fn trusted_native_binds_and_observes_real_whole_wasm_protocol_target() {
    let mut kernel = SessionKernel::new();
    let session_id = SessionId::parse("trusted-whole-wasm").unwrap();
    kernel.create_session(session_id.clone()).unwrap();
    let service = kernel
        .instrumentation(&session_id)
        .expect("trusted host service");
    let source = LiveSource::new(
        "whole-wasm.hal",
        "sha256:whole-wasm",
        "(std.protocol.icount.ICount/count [1 2 3])",
    )
    .unwrap();
    let started = kernel
        .session_mut(&session_id)
        .unwrap()
        .start_whole_wasm_live_session("debug-session", source)
        .expect("Whole-Wasm live session starts");
    let first_run = kernel
        .session_mut(&session_id)
        .unwrap()
        .dispatch_live_session(LiveSessionRequest::for_state(
            "run-whole-wasm-before-bind",
            &started,
            LiveSessionCommand::Run {
                boundary_limit: 1_000,
            },
        ))
        .expect("Whole-Wasm live target runs");
    assert_eq!(first_run.state.status, LiveSessionStatus::Returned);
    assert_eq!(first_run.payload["result"], 3);

    let (target_id, generation) = target_identity(&first_run.payload);
    let target = service
        .bind_target_identity(target_id.clone(), generation)
        .expect("trusted host binds the real Whole-Wasm target");
    let descriptor = service
        .target_descriptor(&target)
        .expect("bounded target descriptor");
    assert_eq!(descriptor.target_id, target_id);
    assert_eq!(descriptor.session_id, session_id.as_str());
    assert_eq!(descriptor.kind, TargetKind::WholeWasm);

    let instrument = service
        .register(protocol_call_registration(
            "host/whole-wasm-protocol",
            session_id.as_str(),
            target.target_id(),
        ))
        .expect("protocol-call instrument registration");
    service
        .attach(&instrument, &target)
        .expect("passive host attaches to Whole-Wasm target");

    let run = kernel
        .session_mut(&session_id)
        .unwrap()
        .dispatch_live_session(LiveSessionRequest::for_state(
            "run-whole-wasm-after-bind",
            &first_run.state,
            LiveSessionCommand::Run {
                boundary_limit: 1_000,
            },
        ))
        .expect("Whole-Wasm live target runs with instrumentation");
    assert_eq!(run.payload["result"], 3);
    let batch = service
        .drain_events(&instrument)
        .expect("host drains protocol-call events");
    assert!(batch.events.iter().any(|event| {
        event.envelope.event == EventKind::ProtocolCall
            && event.envelope.target_id == target.target_id()
            && event.envelope.session_id == session_id.as_str()
            && event.envelope.target_kind == TargetKind::WholeWasm
            && event.envelope.data.get("target")
                == Some(&"std.protocol.icount.ICount/count".to_owned())
            && event.envelope.data.get("status") == Some(&"return".to_owned())
    }));
    assert!(batch.events.iter().any(|event| {
        event.envelope.event == EventKind::ExecutionTerminal
            && event.envelope.target_id == target.target_id()
            && event.envelope.data.get("status") == Some(&"returned".to_owned())
    }));
}
