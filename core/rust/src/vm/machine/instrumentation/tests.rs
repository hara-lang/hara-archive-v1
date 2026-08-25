use super::*;
use crate::core::Value;
use crate::vm::program::{FunctionPrototype, Program};
use crate::vm::source_map::SourceMap;
use crate::vm::{compile_source, validate, Instruction, Machine, VmOutcome};
use std::rc::Rc;

fn machine(source: &str) -> Machine {
    Machine::entry(Rc::new(
        compile_source(source).expect("source must compile"),
    ))
}

fn returned(outcome: VmOutcome) -> Value {
    match outcome {
        VmOutcome::Returned(value) => value,
        VmOutcome::Failed(error) => panic!("unexpected failure: {error}"),
        VmOutcome::Suspended(_) => panic!("unexpected suspension"),
        VmOutcome::Yielded(_) => panic!("unexpected yield"),
    }
}

#[test]
fn no_probe_preserves_the_ordinary_result() {
    let mut ordinary = machine("(+ 1 (* 2 3))");
    let mut instrumented = machine("(+ 1 (* 2 3))");
    let expected = returned(ordinary.run());
    let actual = returned(instrumented.run_instrumented(&mut NoProbe));
    assert_eq!(actual, expected);
    assert_eq!(actual, Value::Number(7));
}

#[test]
fn counters_cover_instructions_opcodes_and_depths() {
    let mut machine = machine("(+ 1 (* 2 3))");
    let mut probe = CounterProbe::default();
    assert_eq!(
        returned(machine.run_instrumented(&mut probe)),
        Value::Number(7)
    );
    let metrics = probe.metrics();
    assert_eq!(metrics.schema, BYTECODE_METRICS_SCHEMA);
    assert!(metrics.instructions >= 2);
    assert!(probe.opcode_count(Opcode::IntrinsicCall) >= 1);
    assert!(metrics
        .named_opcode_counts()
        .any(|entry| entry.opcode == "intrinsic-call" && entry.count >= 1));
    assert_eq!(metrics.terminal_returns, 1);
    assert_eq!(metrics.failures, 0);
    assert!(metrics.max_stack_depth >= 1);
}

#[test]
fn synchronous_calls_and_returns_are_counted() {
    let mut machine = machine("(do (defn f [x] (+ x 1)) (f 41))");
    let mut probe = CounterProbe::default();
    let registry = crate::embedding_namespace_registry();
    let value = crate::core::with_namespace_registry(&registry, || {
        returned(machine.run_instrumented(&mut probe))
    });
    assert_eq!(value, Value::Number(42));
    assert!(probe.metrics().calls >= 1);
    assert!(probe.metrics().returns >= 1);
    assert!(probe.metrics().max_call_depth >= 1);
}

#[test]
fn caught_failures_emit_unwind_without_terminal_failure() {
    let mut machine = machine("(try (/ 1 0) (catch Exception error 42))");
    let mut probe = CounterProbe::default();
    assert_eq!(
        returned(machine.run_instrumented(&mut probe)),
        Value::Number(42)
    );
    assert!(probe.metrics().unwinds >= 1);
    assert_eq!(probe.metrics().failures, 0);
}

#[test]
fn event_ring_is_fixed_capacity_and_reports_overwrite() {
    let mut machine = machine("(let [x 1] [x 2 3 4])");
    let mut events = EventRing::with_capacity(3);
    assert_eq!(
        returned(machine.run_instrumented(&mut events)),
        Value::Vector(
            vec![
                Value::Number(1),
                Value::Number(2),
                Value::Number(3),
                Value::Number(4),
            ]
            .into(),
        )
    );
    assert_eq!(events.schema(), BYTECODE_EVENTS_SCHEMA);
    assert_eq!(events.capacity(), 3);
    assert_eq!(events.len(), 3);
    assert!(events.dropped() > 0);
    assert!(matches!(
        events.iter().last(),
        Some(VmEvent::Terminal(TerminalEvent {
            kind: TerminalKind::Return,
            ..
        }))
    ));
}

#[test]
fn sampled_probe_keeps_control_boundaries_and_terminal_event() {
    let mut machine = machine("[1 2 3 4 5 6]");
    let ring = EventRing::with_capacity(64);
    let mut sampled = SampledProbe::new(ring, 3);
    returned(machine.run_instrumented(&mut sampled));
    let ring = sampled.into_inner();
    let instructions = ring
        .iter()
        .filter(|event| matches!(event, VmEvent::Instruction(_)))
        .count();
    assert!(instructions > 0);
    assert!(instructions < 8);
    assert!(matches!(
        ring.iter().last(),
        Some(VmEvent::Terminal(TerminalEvent {
            kind: TerminalKind::Return,
            ..
        }))
    ));
}

#[test]
fn pending_await_counts_suspend_resume_and_return() {
    let promise = crate::core::Promise::new();
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
    validate(&program).expect("program must validate");
    let mut machine = Machine::entry(Rc::new(program));
    let mut probe = CounterProbe::default();

    assert!(matches!(
        machine.run_instrumented(&mut probe),
        VmOutcome::Suspended(_)
    ));
    assert_eq!(probe.metrics().suspensions, 1);

    assert!(promise.resolve(Value::Number(42)));
    assert_eq!(
        returned(machine.resume_instrumented(promise.state(), &mut probe)),
        Value::Number(42)
    );
    assert_eq!(probe.metrics().resumptions, 1);
    assert_eq!(probe.metrics().terminal_returns, 1);
}
