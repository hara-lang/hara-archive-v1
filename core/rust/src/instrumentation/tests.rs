use std::collections::{BTreeMap, BTreeSet};

use super::*;

fn set<T: Ord>(values: impl IntoIterator<Item = T>) -> BTreeSet<T> {
    values.into_iter().collect()
}

fn passive_registration(
    id: &str,
    session_id: &str,
    events: impl IntoIterator<Item = EventKind>,
) -> InstrumentRegistration {
    let events = set(events);
    let capabilities = events
        .iter()
        .map(|event| event.required_capability())
        .chain([Capability::InspectSnapshot])
        .collect();
    InstrumentRegistration {
        instrument_id: id.into(),
        session_id: session_id.into(),
        mode: InstrumentMode::Passive,
        capabilities,
        events,
        filter: InstrumentFilter::default(),
        projection: ProjectionRequest {
            machine_snapshot: Some(ProjectionLimits::default()),
            ..ProjectionRequest::default()
        },
        delivery: EventDelivery::Queue { capacity: 32 },
    }
}

fn control_registration(id: &str, session_id: &str) -> InstrumentRegistration {
    InstrumentRegistration {
        instrument_id: id.into(),
        session_id: session_id.into(),
        mode: InstrumentMode::Control,
        capabilities: set([Capability::EventLifecycle, Capability::ControlSingleStep]),
        events: set([EventKind::ExecutionTerminal]),
        filter: InstrumentFilter::default(),
        projection: ProjectionRequest::default(),
        delivery: EventDelivery::Queue { capacity: 8 },
    }
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

#[test]
fn empty_hub_has_no_enabled_events() {
    let hub = InstrumentationHub::new();
    assert!(hub.enabled_events().is_empty());
    assert_eq!(hub.registration_count(), 0);
    assert_eq!(hub.target_count(), 0);
    assert_eq!(hub.attachment_count(), 0);
}

#[test]
fn whole_wasm_target_accepts_protocol_call_instrumentation() {
    let mut hub = InstrumentationHub::new();
    let target = hub
        .register_target(target(
            "whole-wasm",
            "session",
            TargetKind::WholeWasm,
            [
                Capability::EventSemanticBoundary,
                Capability::InspectSnapshot,
            ],
        ))
        .expect("whole-Wasm target registration");
    let instrument = hub
        .register(passive_registration(
            "bridge-trace",
            "session",
            [EventKind::ProtocolCall],
        ))
        .expect("protocol-call registration");
    hub.attach(&instrument, &target)
        .expect("protocol-call attachment");
    assert!(hub
        .enabled_for_target(&target, EventKind::ProtocolCall)
        .expect("target is live"));
}

#[test]
fn attachments_follow_registration_order() {
    let mut hub = InstrumentationHub::new();
    let first = hub
        .register(passive_registration(
            "first",
            "session",
            [EventKind::CallEnter],
        ))
        .expect("first registration");
    let second = hub
        .register(passive_registration(
            "second",
            "session",
            [EventKind::CallEnter],
        ))
        .expect("second registration");
    let target = hub
        .register_target(target(
            "execution",
            "session",
            TargetKind::Interpreter,
            [Capability::EventCall, Capability::InspectSnapshot],
        ))
        .expect("target registration");

    hub.attach(&second, &target).expect("second attachment");
    hub.attach(&first, &target).expect("first attachment");

    let ids = hub
        .attachments_for_target(&target)
        .expect("target is live")
        .iter()
        .map(|attachment| attachment.instrument.instrument_id())
        .collect::<Vec<_>>();
    assert_eq!(ids, ["first", "second"]);
    assert!(hub.enabled_events().contains(EventKind::CallEnter));
    assert!(hub
        .enabled_for_target(&target, EventKind::CallEnter)
        .expect("target is live"));
}

#[test]
fn unsupported_capabilities_report_target_and_backend() {
    let mut hub = InstrumentationHub::new();
    let instrument = hub
        .register(passive_registration(
            "trace",
            "session",
            [EventKind::SemanticBoundary],
        ))
        .expect("registration");
    let target = hub
        .register_target(target(
            "execution",
            "session",
            TargetKind::Interpreter,
            [Capability::InspectSnapshot],
        ))
        .expect("target registration");

    let error = hub
        .attach(&instrument, &target)
        .expect_err("event capability is unsupported");
    assert_eq!(
        error,
        InstrumentationError::UnsupportedCapabilities {
            target_id: "execution".into(),
            backend: RuntimeBackend::new("rust").expect("test backend is valid"),
            missing: set([Capability::EventSemanticBoundary]),
        }
    );
}

#[test]
fn projection_requests_require_explicit_inspection_capabilities() {
    let mut registration = passive_registration("trace", "session", [EventKind::CallEnter]);
    registration
        .capabilities
        .remove(&Capability::InspectSnapshot);

    assert_eq!(
        registration.validate(),
        Err("instrument projections require their inspection capability")
    );
}

#[test]
fn target_kind_rejects_foreign_event_semantics() {
    let mut hub = InstrumentationHub::new();
    let instrument = hub
        .register(passive_registration(
            "trace",
            "session",
            [EventKind::SemanticBoundary],
        ))
        .expect("registration");
    let target = hub
        .register_target(target(
            "execution",
            "session",
            TargetKind::Hbc,
            [
                Capability::EventSemanticBoundary,
                Capability::InspectSnapshot,
            ],
        ))
        .expect("target registration");

    assert_eq!(
        hub.attach(&instrument, &target),
        Err(InstrumentationError::UnsupportedEvents {
            target_id: "execution".into(),
            backend: RuntimeBackend::new("rust").expect("test backend is valid"),
            events: set([EventKind::SemanticBoundary]),
        })
    );
}

#[test]
fn only_one_controller_can_hold_a_target_lease() {
    let mut hub = InstrumentationHub::new();
    let first = hub
        .register(control_registration("debugger-a", "session"))
        .expect("first controller");
    let second = hub
        .register(control_registration("debugger-b", "session"))
        .expect("second controller");
    let target = hub
        .register_target(target(
            "execution",
            "session",
            TargetKind::Interpreter,
            [Capability::EventLifecycle, Capability::ControlSingleStep],
        ))
        .expect("target registration");
    hub.attach(&first, &target).expect("first attachment");
    hub.attach(&second, &target).expect("second attachment");

    let lease = hub
        .acquire_control(&first, &target)
        .expect("first controller acquires lease");
    assert_eq!(lease.instrument(), &first);
    assert_eq!(
        hub.acquire_control(&second, &target),
        Err(InstrumentationError::ControlLeaseHeld {
            target_id: "execution".into(),
            holder: "debugger-a".into(),
        })
    );
    hub.release_control(&lease).expect("lease release");
    hub.acquire_control(&second, &target)
        .expect("second controller acquires released lease");
}

#[test]
fn detached_handles_stay_stale_after_id_reuse() {
    let mut hub = InstrumentationHub::new();
    let original = hub
        .register(passive_registration(
            "trace",
            "session",
            [EventKind::CallEnter],
        ))
        .expect("original registration");
    hub.detach(&original).expect("detach original");
    let replacement = hub
        .register(passive_registration(
            "trace",
            "session",
            [EventKind::CallEnter],
        ))
        .expect("replacement registration");

    assert_eq!(original.generation(), 0);
    assert_eq!(replacement.generation(), 1);
    assert_eq!(
        hub.detach(&original),
        Err(InstrumentationError::StaleInstrumentHandle {
            instrument_id: "trace".into(),
            generation: 0,
        })
    );
}

#[test]
fn session_cleanup_removes_instruments_targets_attachments_and_leases() {
    let mut hub = InstrumentationHub::new();
    let instrument = hub
        .register(control_registration("debugger", "session"))
        .expect("controller registration");
    let target = hub
        .register_target(target(
            "execution",
            "session",
            TargetKind::Hbc,
            [Capability::EventLifecycle, Capability::ControlSingleStep],
        ))
        .expect("target registration");
    hub.attach(&instrument, &target).expect("attachment");
    hub.acquire_control(&instrument, &target)
        .expect("control lease");

    assert_eq!(
        hub.detach_session("session"),
        SessionCleanup {
            instruments: 1,
            targets: 1,
        }
    );
    assert_eq!(hub.registration_count(), 0);
    assert_eq!(hub.target_count(), 0);
    assert_eq!(hub.attachment_count(), 0);
    assert!(hub.enabled_events().is_empty());
    assert!(matches!(
        hub.remove_target(&target),
        Err(InstrumentationError::StaleTargetHandle { .. })
    ));
}

#[test]
fn event_envelopes_preserve_backend_semantics() {
    let base = EventEnvelope {
        schema: INSTRUMENTATION_EVENT_SCHEMA.into(),
        protocol: INSTRUMENTATION_PROTOCOL.into(),
        instrument_id: "trace".into(),
        runtime: RuntimeBackend::new("rust").expect("test backend is valid"),
        session_id: "session".into(),
        target_id: "execution".into(),
        target_kind: TargetKind::Interpreter,
        generation: 0,
        sequence: 1,
        phase: EventPhase::Live,
        event: EventKind::CallEnter,
        location: Some(EventLocation {
            source_id: Some("editor/main".into()),
            form_path: Some(vec![0, 2]),
            ..EventLocation::default()
        }),
        data: BTreeMap::<String, String>::new(),
    };
    assert_eq!(base.validate(), Ok(()));

    let mut invalid_interpreter = base.clone();
    invalid_interpreter.location = Some(EventLocation {
        instruction_pointer: Some(7),
        ..EventLocation::default()
    });
    assert_eq!(
        invalid_interpreter.validate(),
        Err("interpreter events cannot claim an instruction pointer")
    );

    let mut invalid_hbc = base;
    invalid_hbc.target_kind = TargetKind::Hbc;
    invalid_hbc.event = EventKind::InstructionExecute;
    assert_eq!(
        invalid_hbc.validate(),
        Err("HBC instruction events cannot claim an AST form path")
    );
}

#[test]
fn runtime_owns_a_private_empty_hub_without_a_hara_namespace() {
    let runtime = crate::Runtime::new();
    assert_eq!(runtime.execution.instrumentation.registration_count(), 0);
    assert!(runtime
        .namespace_registry
        .find("std.native.Instrumentation")
        .is_none());

    let sandbox = crate::Runtime::sandbox();
    assert_eq!(sandbox.execution.instrumentation.registration_count(), 0);
    assert!(sandbox
        .namespace_registry
        .find("std.native.Instrumentation")
        .is_none());
}
