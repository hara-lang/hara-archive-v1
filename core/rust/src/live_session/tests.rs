use super::{
    InterpreterLiveSession, LiveBackend, LiveReplacementPolicy, LiveSession, LiveSessionCommand,
    LiveSessionOperation, LiveSessionRequest, LiveSessionState, LiveSessionStatus, LiveSource,
    LIVE_SESSION_PROTOCOL, LIVE_SESSION_STATE_SCHEMA,
};

fn source(id: &str, revision: &str, text: &str) -> LiveSource {
    LiveSource::new(id, revision, text).unwrap()
}

fn request(
    id: &str,
    session: &impl LiveSession,
    command: LiveSessionCommand,
) -> LiveSessionRequest {
    LiveSessionRequest::for_state(id, &session.state(), command)
}

fn dispatch_ok(
    session: &mut impl LiveSession,
    id: &str,
    command: LiveSessionCommand,
) -> super::LiveSessionReply {
    let request = LiveSessionRequest::for_state(id, &session.state(), command);
    session.dispatch(request).unwrap()
}

#[test]
fn state_and_capabilities_publish_one_backend_neutral_contract() {
    let state = LiveSessionState {
        session_id: "fixture/live-model".into(),
        source_id: "model.hal".into(),
        generation: 3,
        revision: "sha256:model".into(),
        sequence: 17,
        backend: LiveBackend::Interpreter,
        status: LiveSessionStatus::Suspended,
    };
    let encoded = state.to_json();
    assert_eq!(encoded["schema"], LIVE_SESSION_STATE_SCHEMA);
    assert_eq!(encoded["protocol"], LIVE_SESSION_PROTOCOL);
    assert_eq!(encoded["session-id"], "fixture/live-model");
    assert_eq!(encoded["backend"], "interpreter");
    assert_eq!(encoded["status"], "suspended");

    let mut session = InterpreterLiveSession::start(
        "fixture/live-capabilities",
        source("capabilities.hal", "sha256:capabilities", "(+ 1 2)"),
    )
    .unwrap();
    let capabilities = session.capabilities();
    assert!(capabilities.supports(LiveSessionOperation::Update));
    assert!(!capabilities.supports(LiveSessionOperation::Pause));
    assert_eq!(capabilities.to_json()["protocol"], LIVE_SESSION_PROTOCOL);
    dispatch_ok(
        &mut session,
        "dispose-capabilities",
        LiveSessionCommand::Dispose,
    );
}

#[test]
fn interpreter_adapter_normalizes_lifecycle_and_source_replacement() {
    let mut session = InterpreterLiveSession::start(
        "fixture/live-interpreter",
        source("first.hal", "sha256:first", "(+ 1 (* 2 3))"),
    )
    .unwrap();
    assert_eq!(session.state().status, LiveSessionStatus::Ready);

    let run = dispatch_ok(
        &mut session,
        "run-first",
        LiveSessionCommand::Run {
            boundary_limit: 1_000,
        },
    );
    assert_eq!(run.state.status, LiveSessionStatus::Returned);
    assert_eq!(run.payload["status"], "returned");

    let reset = dispatch_ok(&mut session, "reset-first", LiveSessionCommand::Reset);
    assert_eq!(reset.state.generation, 1);
    assert_eq!(reset.state.status, LiveSessionStatus::Ready);

    let queued = dispatch_ok(
        &mut session,
        "queue-second",
        LiveSessionCommand::Update {
            source: source("second.hal", "sha256:second", "(+ 40 2)"),
            policy: LiveReplacementPolicy::ReplaceOnNextStart,
        },
    );
    assert_eq!(queued.state.revision, "sha256:first");
    assert_eq!(session.pending_revision(), Some("sha256:second"));

    let activated = dispatch_ok(&mut session, "activate-second", LiveSessionCommand::Reset);
    assert_eq!(activated.state.generation, 2);
    assert_eq!(activated.state.revision, "sha256:second");
    assert_eq!(activated.state.source_id, "second.hal");

    let rerun = dispatch_ok(
        &mut session,
        "run-second",
        LiveSessionCommand::Run {
            boundary_limit: 1_000,
        },
    );
    assert_eq!(rerun.state.status, LiveSessionStatus::Returned);

    dispatch_ok(
        &mut session,
        "dispose-interpreter",
        LiveSessionCommand::Dispose,
    );
    assert_eq!(session.state().status, LiveSessionStatus::Disposed);
}

#[test]
fn stale_generation_is_rejected_before_backend_mutation() {
    let mut session = InterpreterLiveSession::start(
        "fixture/live-stale",
        source("stale.hal", "sha256:stale", "(+ 1 2)"),
    )
    .unwrap();
    let before = session.state();
    let mut stale = request("stale-step", &session, LiveSessionCommand::Step);
    stale.generation = Some(before.generation + 1);
    let error = session.dispatch(stale).unwrap_err();
    assert_eq!(error.code(), "live-session/stale-generation");
    assert_eq!(session.state(), before);

    dispatch_ok(&mut session, "dispose-stale", LiveSessionCommand::Dispose);
}

#[cfg(feature = "bytecode-observation")]
#[test]
fn hbc_adapter_uses_the_same_identity_and_lifecycle_contract() {
    use super::BytecodeLiveSession;

    let mut session = BytecodeLiveSession::compile(
        "fixture/live-hbc",
        source("bytecode.hal", "sha256:hbc-first", "(+ 19 23)"),
    )
    .unwrap();
    assert_eq!(session.state().backend, LiveBackend::Hbc);
    assert!(session.capabilities().supports(LiveSessionOperation::Pause));

    let run = dispatch_ok(
        &mut session,
        "run-hbc",
        LiveSessionCommand::Run {
            boundary_limit: 1_000,
        },
    );
    assert_eq!(run.state.status, LiveSessionStatus::Returned);

    dispatch_ok(&mut session, "reset-hbc", LiveSessionCommand::Reset);
    let paused = dispatch_ok(&mut session, "pause-hbc", LiveSessionCommand::Pause);
    assert_eq!(paused.state.status, LiveSessionStatus::Paused);
    let resumed = dispatch_ok(
        &mut session,
        "resume-hbc",
        LiveSessionCommand::Resume { settlement: None },
    );
    assert_eq!(resumed.state.status, LiveSessionStatus::Ready);

    let replaced = dispatch_ok(
        &mut session,
        "replace-hbc",
        LiveSessionCommand::Update {
            source: source("replacement.hal", "sha256:hbc-second", "(+ 40 2)"),
            policy: LiveReplacementPolicy::Restart,
        },
    );
    assert_eq!(replaced.state.generation, 2);
    assert_eq!(replaced.state.revision, "sha256:hbc-second");

    let cancelled = dispatch_ok(&mut session, "cancel-hbc", LiveSessionCommand::Cancel);
    assert_eq!(cancelled.state.status, LiveSessionStatus::Cancelled);
    let blocked_request = request("step-cancelled-hbc", &session, LiveSessionCommand::Step);
    let blocked = session.dispatch(blocked_request).unwrap_err();
    assert_eq!(blocked.code(), "live-session/cancelled");

    dispatch_ok(&mut session, "dispose-hbc", LiveSessionCommand::Dispose);
}

#[cfg(all(feature = "bytecode-observation", feature = "bytecode-instrumentation"))]
#[test]
fn instrumented_hbc_live_session_starts_from_validated_artifact() {
    use super::instrumented_hbc::InstrumentedHbcLiveSession;
    use crate::vm::{compile_source, encode_program};
    use crate::Runtime;

    let program = compile_source("(+ 19 23)").unwrap();
    let artifact = encode_program(&program).unwrap();
    let runtime = Runtime::core();
    let mut session = InstrumentedHbcLiveSession::start_from_artifact(
        &runtime,
        "fixture/live-artifact-owner",
        "fixture/live-artifact",
        source("artifact.hal", "sha256:artifact", "(+ 19 23)"),
        &artifact,
    )
    .unwrap();

    let run = dispatch_ok(
        &mut session,
        "run-artifact",
        LiveSessionCommand::Run {
            boundary_limit: 1_000,
        },
    );
    assert_eq!(run.state.status, LiveSessionStatus::Returned);
    assert_eq!(run.payload["result"], 42);

    dispatch_ok(
        &mut session,
        "dispose-artifact",
        LiveSessionCommand::Dispose,
    );
}

#[cfg(all(feature = "whole-wasm", not(target_arch = "wasm32")))]
#[test]
fn whole_wasm_live_session_exposes_prepared_call_contract_only() {
    use super::whole_wasm::WholeWasmLiveSession;
    let runtime = crate::Runtime::core();

    let mut session = WholeWasmLiveSession::start(
        &runtime,
        "fixture/session",
        "fixture/live-whole-wasm",
        source("whole-wasm.hal", "sha256:whole-wasm", "(+ 19 23)"),
    )
    .unwrap();
    let capabilities = session.capabilities();
    assert!(capabilities.supports(LiveSessionOperation::Run));
    assert!(capabilities.supports(LiveSessionOperation::Call));
    assert!(!capabilities.supports(LiveSessionOperation::Step));
    assert!(!capabilities.supports(LiveSessionOperation::Snapshot));

    let run = dispatch_ok(
        &mut session,
        "run-whole-wasm",
        LiveSessionCommand::Run {
            boundary_limit: 1_000,
        },
    );
    assert_eq!(run.state.status, LiveSessionStatus::Returned);
    assert_eq!(run.payload["result"], 42);

    dispatch_ok(
        &mut session,
        "dispose-whole-wasm",
        LiveSessionCommand::Dispose,
    );
}
