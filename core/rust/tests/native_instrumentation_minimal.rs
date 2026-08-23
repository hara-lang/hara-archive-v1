use std::collections::BTreeSet;

use hara_wasm::instrumentation::{
    Capability, EventDelivery, EventKind, InstrumentFilter, InstrumentMode, InstrumentRegistration,
    NativeInstrumentationError, ProjectionRequest,
};
use hara_wasm::{SessionId, SessionKernel};

fn registration(id: &str, session_id: &str) -> InstrumentRegistration {
    InstrumentRegistration {
        instrument_id: id.into(),
        session_id: session_id.into(),
        mode: InstrumentMode::Passive,
        capabilities: BTreeSet::from([Capability::EventCall]),
        events: BTreeSet::from([EventKind::CallEnter]),
        filter: InstrumentFilter::default(),
        projection: ProjectionRequest::default(),
        delivery: EventDelivery::Queue { capacity: 8 },
    }
}

#[test]
fn trusted_native_service_is_available_without_optional_bytecode_features() {
    let mut kernel = SessionKernel::new();
    let session_id = SessionId::parse("native-minimal").expect("valid session id");
    kernel
        .create_session(session_id.clone())
        .expect("create native host session");

    let service = kernel
        .instrumentation(&session_id)
        .expect("active Session exposes the trusted host service");
    let handle = service
        .register(registration("minimal-trace", session_id.as_str()))
        .expect("register passive instrument");
    assert_eq!(handle.instrument_id(), "minimal-trace");
    assert_eq!(handle.generation(), 0);
    assert_eq!(
        service
            .registration(&handle)
            .expect("read back registration")
            .capabilities,
        BTreeSet::from([Capability::EventCall])
    );

    let mut transform = registration("minimal-transform", session_id.as_str());
    transform.mode = InstrumentMode::Transform;
    transform.capabilities.clear();
    transform.events.clear();
    assert!(matches!(
        service.register(transform),
        Err(NativeInstrumentationError::UnsupportedMode(
            InstrumentMode::Transform
        ))
    ));

    kernel
        .close_session(&session_id)
        .expect("close native host session");
    assert!(!service.is_active());
    assert!(matches!(
        service.register(registration("late", session_id.as_str())),
        Err(NativeInstrumentationError::RuntimeClosed { session_id: closed })
            if closed == session_id.as_str()
    ));
}
