use super::*;
use crate::vm::opcode::Instruction;
use crate::vm::program::{FunctionPrototype, Program};
use crate::vm::source_map::SourceMap;
use crate::vm::{compile_source, encode_program, validate};

fn json(value: &Value) -> String {
    crate::json::write(value).expect("evidence must remain JSON-safe")
}

#[test]
fn live_session_runs_real_bytecode_and_emits_all_three_contracts() {
    let mut session = BytecodeObservationSession::compile_named(
        "execution/arithmetic",
        "example/core.hal",
        "(+ 1 (* 2 3))",
    )
    .expect("source must compile");

    let initial = session.snapshot().expect("live snapshot");
    assert_eq!(initial.status.as_keyword(), "ready");
    let delta = session.run(64).expect("observed execution");

    assert_eq!(session.status(), BytecodeSessionStatus::Returned);
    assert_eq!(session.result(), Some(&Value::Number(7)));

    let metrics = json(&session.metrics());
    assert!(metrics.contains("\"schema\":\"hal.bytecode-metrics/0-alpha\""));
    assert!(metrics.contains("\"instructions\":"));
    assert!(metrics.contains("\"intrinsic-call\":"));

    let events = json(&session.events());
    assert!(events.contains("\"schema\":\"hal.bytecode-events/0-alpha\""));
    assert!(events.contains("\"kind\":\"terminal\""));
    assert!(events.contains("\"terminal\":\"machine/return\""));

    let trace = json(&session.trace());
    assert!(trace.contains("\"schema\":\"hal.bytecode-trace/0-alpha\""));
    assert!(trace.contains("\"sourceId\":\"example/core.hal\""));
    assert!(trace.contains("\"display\":\"7\""));
    assert!(json(&delta).contains("\"steps\":["));
}

#[test]
fn public_snapshot_exposes_deterministic_bounded_owned_globals() {
    let mut source = String::from("(do ");
    for index in (0..70).rev() {
        if index == 0 {
            source.push_str("(def g000 :abcdefgh) ");
        } else {
            source.push_str(&format!("(def g{index:03} {index}) "));
        }
    }
    source.push_str("nil)");

    let mut session =
        BytecodeObservationSession::compile_named("execution/globals", "globals.hal", source)
            .expect("global fixture must compile");
    let mut limits = session.observation_limits();
    limits.display_chars = 4;
    session.set_observation_limits(limits);
    session.run(4096).expect("global fixture must execute");

    let snapshot = json(&session.snapshot_value().expect("public snapshot"));
    assert!(snapshot.contains("\"namespace\":\"user\""));
    assert!(snapshot.contains("\"scope\":\"current-namespace-owned\""));
    assert!(snapshot.contains("\"limit\":64"));
    assert!(snapshot.contains("\"omitted\":6"));
    assert!(snapshot.contains("\"symbol\":\"user/g000\""));
    assert!(snapshot.contains("\"symbol\":\"user/g063\""));
    assert!(!snapshot.contains("\"symbol\":\"user/g064\""));
    assert!(!snapshot.contains("std.foundation/"));
    assert!(snapshot.contains("\"origin\":\"source\""));
    assert!(snapshot.contains("\"display\":\":abc…\""));
    assert!(snapshot.contains("\"truncated\":true"));

    let first = snapshot.find("user/g000").expect("first global");
    let second = snapshot.find("user/g001").expect("second global");
    assert!(first < second, "globals must be sorted by qualified symbol");
}

#[test]
fn pause_resume_reset_and_sequence_identity_are_stable() {
    let mut session = BytecodeObservationSession::compile_named(
        "execution/lifecycle",
        "lifecycle.hal",
        "(+ 20 22)",
    )
    .expect("source must compile");

    assert!(session.pause());
    assert_eq!(session.status(), BytecodeSessionStatus::Paused);
    session.resume(None).expect("paused session resumes");
    assert_eq!(session.status(), BytecodeSessionStatus::Ready);

    session.step().expect("first boundary");
    let sequence_after_step = session.sequence();
    let first_trace_id = session.trace_id().to_string();
    let events = json(&session.events());
    assert!(events.contains("execution/lifecycle/trace-0/event/0"));

    session.reset().expect("session resets");
    assert_eq!(session.status(), BytecodeSessionStatus::Ready);
    assert_ne!(session.trace_id(), first_trace_id);
    assert_eq!(session.sequence(), sequence_after_step);
    assert!(json(&session.events()).contains("\"events\":[]"));

    session.run(32).expect("reset machine runs");
    assert_eq!(session.result(), Some(&Value::Number(42)));
    assert!(json(&session.trace()).contains("execution/lifecycle/trace-1/step/"));
}

#[test]
fn suspension_retains_the_real_promise_and_resumes_one_boundary() {
    let promise = Promise::new();
    let mut source_map = SourceMap::default();
    for _ in 0..3 {
        source_map.record(None);
    }
    let program = Program {
        namespace: None,
        constants: vec![Value::Promise(promise.clone())],
        var_metadata: Vec::new(),
        schema_types: Default::default(),
        function_types: Default::default(),
        inferred_function_types: Default::default(),
        functions: vec![FunctionPrototype {
            name: Some("await-demo".into()),
            async_function: false,
            arity: 0,
            variadic: false,
            capture_count: 0,
            local_count: 0,
            max_stack: 1,
            code: vec![
                Instruction::Constant(0),
                Instruction::Await,
                Instruction::Return,
            ],
            source_map,
            handlers: Vec::new(),
        }],
        entry: 0,
    };
    validate(&program).expect("program validates");
    let mut session =
        BytecodeObservationSession::from_program("execution/suspend", "suspend.hal", program)
            .expect("session owns program");

    session.run(8).expect("run to suspension");
    assert_eq!(session.status(), BytecodeSessionStatus::Suspended);
    let retained = session.suspended_promise().expect("retained promise");
    assert!(retained.same_identity(&promise));

    assert!(session.resolve_suspension(Value::Number(42)).unwrap());
    let resumed = session.resume(None).expect("resume boundary");
    assert_eq!(session.status(), BytecodeSessionStatus::Running);
    assert!(json(&resumed).contains("\"kind\":\"machine/resume\""));

    session.run(8).expect("finish after resume");
    assert_eq!(session.result(), Some(&Value::Number(42)));
    let metrics = json(&session.metrics());
    assert!(metrics.contains("\"suspensions\":1"));
    assert!(metrics.contains("\"resumptions\":1"));
}

#[test]
fn artifacts_retention_failures_and_disposal_remain_bounded() {
    let program = compile_source("(/ 1 0)").expect("source compiles");
    let artifact = encode_program(&program).expect("artifact encodes");
    let mut session = BytecodeObservationSession::from_artifact_named(
        "execution/failure",
        "failure.hbc",
        &artifact,
    )
    .expect("artifact decodes");
    session.set_retention_limits(SessionRetentionLimits {
        events: 1,
        trace: 1,
    });

    session.run(64).expect("failure is execution evidence");
    assert_eq!(session.status(), BytecodeSessionStatus::Failed);
    assert_eq!(
        session.error().map(|error| error.message.as_str()),
        Some("division by zero")
    );
    assert!(json(&session.events()).contains("\"dropped\":"));
    assert!(json(&session.trace()).contains("\"dropped\":"));
    assert!(session.events.len() <= 1);
    assert!(session.trace_steps.len() <= 1);

    assert!(session.dispose());
    assert_eq!(session.status(), BytecodeSessionStatus::Disposed);
    assert!(session.snapshot().is_err());
    assert!(!session.dispose());
}
