//! Portable instrumentation conformance corpus — Rust producer.
//!
//! Each test case is labeled `CONFORMS: instrum/<case-name>` and has a matching case in
//! `CrossRuntimeInstrumentationConformanceTest.java`. Both providers assert the same portable
//! invariants. Documented per-case differences (e.g., backend identifier, Rust-only
//! `enabled_events()` surface) are noted in comments.
//!
//! This is the first delivery slice of issue #937.

use std::collections::BTreeSet;

use crate::instrumentation::{
    Capability, EventDelivery, EventKind, InstrumentFilter, InstrumentMode, InstrumentRegistration,
    InstrumentationError, InstrumentationHub, ProjectionRequest, RuntimeBackend, TargetDescriptor,
    TargetKind,
};

fn set<T: Ord>(values: impl IntoIterator<Item = T>) -> BTreeSet<T> {
    values.into_iter().collect()
}

fn interpreter_target(id: &str, session: &str) -> TargetDescriptor {
    TargetDescriptor {
        target_id: id.into(),
        session_id: session.into(),
        kind: TargetKind::Interpreter,
        backend: RuntimeBackend::new("rust").expect("test backend"),
        capabilities: set([Capability::EventLifecycle]),
    }
}

fn passive(id: &str, session: &str) -> InstrumentRegistration {
    InstrumentRegistration {
        instrument_id: id.into(),
        session_id: session.into(),
        mode: InstrumentMode::Passive,
        capabilities: set([Capability::EventLifecycle]),
        events: set([EventKind::ExecutionTerminal]),
        filter: InstrumentFilter::default(),
        projection: ProjectionRequest::default(),
        delivery: EventDelivery::Queue { capacity: 8 },
    }
}

fn control_reg(id: &str, session: &str) -> InstrumentRegistration {
    InstrumentRegistration {
        instrument_id: id.into(),
        session_id: session.into(),
        mode: InstrumentMode::Control,
        capabilities: set([Capability::EventLifecycle, Capability::ControlPause]),
        events: set([EventKind::ExecutionTerminal]),
        filter: InstrumentFilter::default(),
        projection: ProjectionRequest::default(),
        delivery: EventDelivery::Queue { capacity: 8 },
    }
}

// CONFORMS: instrum/fresh-hub-zero-state
// A freshly created hub has no registered instruments, targets, or attachments.
// Rust additionally verifies enabled_events() is empty; Java exposes count APIs only.
#[test]
fn fresh_hub_has_zero_registered_state() {
    let hub = InstrumentationHub::new();
    assert_eq!(hub.registration_count(), 0);
    assert_eq!(hub.target_count(), 0);
    assert_eq!(hub.attachment_count(), 0);
    assert!(hub.enabled_events().is_empty());
}

// CONFORMS: instrum/registration-order
// Instruments registered first are delivered first; registration insertion order is preserved.
#[test]
fn attachments_follow_registration_order() {
    let mut hub = InstrumentationHub::new();
    let target = hub
        .register_target(interpreter_target("t", "s"))
        .expect("target");
    let first = hub.register(passive("first", "s")).expect("first");
    let second = hub.register(passive("second", "s")).expect("second");
    hub.attach(&first, &target).expect("first attachment");
    hub.attach(&second, &target).expect("second attachment");
    let ids: Vec<&str> = hub
        .attachments_for_target(&target)
        .expect("live target")
        .iter()
        .map(|a| a.instrument.instrument_id())
        .collect();
    assert_eq!(ids, ["first", "second"]);
}

// CONFORMS: instrum/unsupported-capability
// Requesting an event capability the target does not advertise fails with structured evidence
// identifying the target, backend, and missing capabilities.
// Rust: InstrumentationError::UnsupportedCapabilities { target_id, backend, missing }
// Java: InstrumentationException(Code.UNSUPPORTED_CAPABILITIES) with evidence map
#[test]
fn unsupported_capability_produces_exact_evidence() {
    let mut hub = InstrumentationHub::new();
    let target = hub
        .register_target(TargetDescriptor {
            target_id: "execution".into(),
            session_id: "s".into(),
            kind: TargetKind::Interpreter,
            backend: RuntimeBackend::new("rust").expect("test backend"),
            capabilities: BTreeSet::new(),
        })
        .expect("target");
    let trace = hub.register(passive("trace", "s")).expect("instrument");
    let error = hub.attach(&trace, &target).expect_err("must fail");
    assert!(
        matches!(
            error,
            InstrumentationError::UnsupportedCapabilities {
                ref target_id,
                ref missing,
                ..
            } if target_id == "execution" && missing.contains(&Capability::EventLifecycle)
        ),
        "unexpected error: {error:?}"
    );
}

// CONFORMS: instrum/exclusive-control-lease
// Only one controller may hold the control lease for a target at a time. A second request
// while a lease is held fails with a deterministic conflict error identifying the current holder.
// Rust: InstrumentationError::ControlLeaseHeld { target_id, holder }
// Java: InstrumentationException(Code.CONTROL_LEASE_CONFLICT) with evidence["holder"]
#[test]
fn exclusive_control_lease_conflict() {
    let mut hub = InstrumentationHub::new();
    let target = hub
        .register_target(TargetDescriptor {
            target_id: "execution".into(),
            session_id: "s".into(),
            kind: TargetKind::Hbc,
            backend: RuntimeBackend::new("rust").expect("test backend"),
            capabilities: set([Capability::EventLifecycle, Capability::ControlPause]),
        })
        .expect("target");
    let first = hub.register(control_reg("debugger-a", "s")).expect("first");
    let second = hub
        .register(control_reg("debugger-b", "s"))
        .expect("second");
    hub.attach(&first, &target).expect("first attach");
    hub.attach(&second, &target).expect("second attach");
    hub.acquire_control(&first, &target).expect("first lease");
    let conflict = hub
        .acquire_control(&second, &target)
        .expect_err("second must conflict");
    assert!(
        matches!(
            conflict,
            InstrumentationError::ControlLeaseHeld {
                ref holder,
                ..
            } if holder == "debugger-a"
        ),
        "unexpected error: {conflict:?}"
    );
}

// CONFORMS: instrum/stale-handle-after-detach
// After an instrument is removed, the old handle has generation 0. A replacement registered
// under the same ID gets generation 1. Using the old handle fails with a stale error.
// Rust: InstrumentationError::StaleInstrumentHandle { instrument_id, generation: 0 }
// Java: InstrumentationException(Code.STALE_INSTRUMENT)
#[test]
fn stale_handle_after_id_reuse() {
    let mut hub = InstrumentationHub::new();
    let original = hub.register(passive("trace", "s")).expect("original");
    hub.detach(&original).expect("detach original");
    let replacement = hub.register(passive("trace", "s")).expect("replacement");
    assert_eq!(original.generation(), 0);
    assert_eq!(replacement.generation(), 1);
    let stale = hub.detach(&original).expect_err("old handle must be stale");
    assert!(
        matches!(
            stale,
            InstrumentationError::StaleInstrumentHandle {
                ref instrument_id,
                generation: 0,
            } if instrument_id == "trace"
        ),
        "unexpected error: {stale:?}"
    );
}

// CONFORMS: instrum/session-cleanup
// Session cleanup removes all instruments, targets, attachments, and leases belonging to
// the session. All counts reach zero; enabled_events becomes empty.
// Rust additionally verifies enabled_events() becomes empty; Java verifies counts only.
#[test]
fn session_cleanup_removes_all_state() {
    let mut hub = InstrumentationHub::new();
    let target = hub
        .register_target(TargetDescriptor {
            target_id: "execution".into(),
            session_id: "s".into(),
            kind: TargetKind::Hbc,
            backend: RuntimeBackend::new("rust").expect("test backend"),
            capabilities: set([Capability::EventLifecycle, Capability::ControlPause]),
        })
        .expect("target");
    let ctrl = hub.register(control_reg("d", "s")).expect("controller");
    hub.attach(&ctrl, &target).expect("attachment");
    hub.acquire_control(&ctrl, &target).expect("lease");
    assert_eq!(hub.registration_count(), 1);
    assert_eq!(hub.target_count(), 1);
    assert_eq!(hub.attachment_count(), 1);
    hub.detach_session("s");
    assert_eq!(hub.registration_count(), 0);
    assert_eq!(hub.target_count(), 0);
    assert_eq!(hub.attachment_count(), 0);
    assert!(hub.enabled_events().is_empty());
}

// CONFORMS: instrum/zero-attachment-no-events
// Registering an instrument without attaching it to a target produces no event subscriptions
// for that target.
// Rust: hub.enabled_for_target(..) returns false; hub.enabled_events() is empty
// Java: hub.hasSubscribers(target, event) returns false
#[test]
fn zero_attachment_produces_no_events() {
    let mut hub = InstrumentationHub::new();
    let target = hub
        .register_target(interpreter_target("t", "s"))
        .expect("target");
    hub.register(passive("trace", "s"))
        .expect("instrument without attachment");
    assert_eq!(
        hub.enabled_for_target(&target, EventKind::ExecutionTerminal),
        Ok(false)
    );
    assert!(hub.enabled_events().is_empty());
}
