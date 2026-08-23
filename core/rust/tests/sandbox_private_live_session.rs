use hara_wasm::lang::protocol::IComponent;
use hara_wasm::live_session::{
    LiveBackend, LiveSessionCommand, LiveSessionOperation, LiveSessionRequest, LiveSessionState,
    LiveSessionStatus, LiveSource,
};
use hara_wasm::restricted_sandbox_session;

fn source(id: &str, revision: &str, text: &str) -> LiveSource {
    LiveSource::new(id, revision, text).unwrap()
}

fn request(
    request_id: &str,
    state: &LiveSessionState,
    command: LiveSessionCommand,
) -> LiveSessionRequest {
    LiveSessionRequest::for_state(request_id, state, command)
}

#[test]
fn private_sandbox_session_owns_and_fences_live_execution() {
    let mut owner = restricted_sandbox_session("user");
    let started = owner
        .start_interpreter_live_session(
            "sandbox/live-1",
            source("lesson.hal", "sha256:first", "(+ 19 23)"),
        )
        .unwrap();
    assert_eq!(started.backend, LiveBackend::Interpreter);
    assert_eq!(started.status, LiveSessionStatus::Ready);
    assert_eq!(owner.live_session_ids(), vec!["sandbox/live-1"]);

    let run = owner
        .dispatch_live_session(request(
            "run-1",
            &started,
            LiveSessionCommand::Run {
                boundary_limit: 1_000,
            },
        ))
        .unwrap();
    assert_eq!(run.state.status, LiveSessionStatus::Returned);

    let mut stale = request("stale-step", &run.state, LiveSessionCommand::Step);
    stale.generation = Some(run.state.generation + 1);
    let error = owner.dispatch_live_session(stale).unwrap_err();
    assert_eq!(error.code(), "live-session/stale-generation");
    assert_eq!(
        owner.live_session_state("sandbox/live-1").unwrap(),
        run.state
    );
}

#[test]
fn private_session_owns_terminal_cancel_and_idempotent_disposal() {
    let mut owner = restricted_sandbox_session("user");
    let started = owner
        .start_interpreter_live_session(
            "sandbox/live-terminal",
            source("terminal.hal", "sha256:terminal", "(+ 1 2)"),
        )
        .unwrap();
    let cancelled = owner
        .dispatch_live_session(request("cancel", &started, LiveSessionCommand::Cancel))
        .unwrap();
    assert_eq!(cancelled.state.status, LiveSessionStatus::Cancelled);

    let blocked = owner
        .dispatch_live_session(request(
            "step-after-cancel",
            &cancelled.state,
            LiveSessionCommand::Step,
        ))
        .unwrap_err();
    assert_eq!(blocked.code(), "live-session/cancelled");

    let disposed = owner
        .dispatch_live_session(request(
            "dispose-first",
            &cancelled.state,
            LiveSessionCommand::Dispose,
        ))
        .unwrap();
    assert_eq!(disposed.state.status, LiveSessionStatus::Disposed);
    assert_eq!(disposed.payload, serde_json::Value::Bool(true));

    let repeated = owner
        .dispatch_live_session(request(
            "dispose-again",
            &disposed.state,
            LiveSessionCommand::Dispose,
        ))
        .unwrap();
    assert_eq!(repeated.state.status, LiveSessionStatus::Disposed);
    assert_eq!(repeated.payload, serde_json::Value::Bool(false));
}

#[test]
fn closing_private_owner_disposes_and_forgets_nested_sessions() {
    let mut owner = restricted_sandbox_session("user");
    owner
        .start_interpreter_live_session(
            "sandbox/live-close",
            source("close.hal", "sha256:close", "(+ 1 2)"),
        )
        .unwrap();
    assert_eq!(owner.live_session_count(), 1);

    owner.stop();
    assert!(owner.stopped());
    assert_eq!(owner.live_session_count(), 0);
    assert!(owner.live_session_ids().is_empty());

    let error = owner
        .start_interpreter_live_session(
            "sandbox/live-after-close",
            source("closed.hal", "sha256:closed", "(+ 1 2)"),
        )
        .unwrap_err();
    assert_eq!(error.code(), "live-session/owner-closed");
}

#[test]
fn live_session_identity_cannot_be_reused_inside_one_owner() {
    let mut owner = restricted_sandbox_session("user");
    let original = owner
        .start_interpreter_live_session(
            "sandbox/live-identity",
            source("identity.hal", "sha256:first", "(+ 1 2)"),
        )
        .unwrap();
    let error = owner
        .start_interpreter_live_session(
            "sandbox/live-identity",
            source("identity.hal", "sha256:second", "(+ 40 2)"),
        )
        .unwrap_err();
    assert_eq!(error.code(), "live-session/already-exists");
    assert_eq!(
        owner.live_session_state("sandbox/live-identity").unwrap(),
        original
    );
}

#[cfg(all(feature = "whole-wasm", not(target_arch = "wasm32")))]
#[test]
fn private_owner_hosts_whole_wasm_from_an_artifact() {
    use hara_wasm::vm::compile_source;
    use hara_wasm::whole_wasm::compile_artifact;

    let mut owner = restricted_sandbox_session("user");
    let program = compile_source("(+ 19 23)").unwrap();
    let artifact = compile_artifact(&program).unwrap();
    let started = owner
        .start_whole_wasm_live_session_from_artifact(
            "sandbox/live-whole-wasm",
            source("whole-wasm.hal", "sha256:whole-wasm", "(+ 19 23)"),
            &artifact,
        )
        .unwrap();
    let capabilities = owner
        .live_session_capabilities("sandbox/live-whole-wasm")
        .unwrap();
    assert!(capabilities.supports(LiveSessionOperation::Run));
    assert!(capabilities.supports(LiveSessionOperation::Call));
    assert!(!capabilities.supports(LiveSessionOperation::Step));

    let run = owner
        .dispatch_live_session(request(
            "run-whole-wasm",
            &started,
            LiveSessionCommand::Run {
                boundary_limit: 1_000,
            },
        ))
        .unwrap();
    assert_eq!(run.payload["result"], 42);

    owner.stop();
    assert_eq!(owner.live_session_count(), 0);
}

#[cfg(feature = "bytecode-observation")]
#[test]
fn private_owner_hosts_hbc_through_the_same_session_contract() {
    let mut owner = restricted_sandbox_session("user");
    let started = owner
        .start_hbc_live_session(
            "sandbox/live-hbc",
            source("bytecode.hal", "sha256:hbc", "(+ 19 23)"),
        )
        .unwrap();
    assert_eq!(started.backend, LiveBackend::Hbc);
    assert!(owner
        .live_session_capabilities("sandbox/live-hbc")
        .unwrap()
        .supports(LiveSessionOperation::Pause));

    let run = owner
        .dispatch_live_session(request(
            "run-hbc",
            &started,
            LiveSessionCommand::Run {
                boundary_limit: 1_000,
            },
        ))
        .unwrap();
    assert_eq!(run.state.status, LiveSessionStatus::Returned);

    owner.stop();
    assert_eq!(owner.live_session_count(), 0);
}
