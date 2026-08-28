use super::*;
use crate::vm::program::{FunctionPrototype, Program};
use crate::vm::source_map::SourceMap;
use crate::vm::{compile_source, validate};
use std::rc::Rc;

fn machine(source: &str) -> Machine {
    Machine::entry(Rc::new(
        compile_source(source).expect("source must compile"),
    ))
}

fn run_observed(machine: &mut Machine) -> (Vec<ObservationEventKind>, Value) {
    let mut kinds = Vec::new();
    for _ in 0..256 {
        let step = machine.step_observed();
        kinds.push(step.kind);
        match step.outcome {
            ObservedStepOutcome::Continue => {}
            ObservedStepOutcome::Returned(value) => return (kinds, value),
            ObservedStepOutcome::Suspended(_) => panic!("unexpected suspension"),
            ObservedStepOutcome::Yielded(_) => panic!("unexpected yield"),
            ObservedStepOutcome::Failed(error) => panic!("unexpected failure: {error}"),
        }
    }
    panic!("observed execution did not terminate");
}

#[test]
fn arithmetic_steps_project_instructions_and_return_value() {
    let mut machine = machine("(+ 1 (* 2 3))");
    let initial = machine.snapshot();
    assert_eq!(initial.status, MachineObservationStatus::Ready);
    assert_eq!(initial.program.entry, 0);

    // The production compiler folds the nested multiplication.
    // Observation must report only instructions the VM actually executes.
    let mut saw_add = false;
    let mut final_after = None;
    for _ in 0..64 {
        let step = machine.step_observed();
        if let Some(instruction) = &step.instruction {
            if instruction.opcode == "primitive"
                && instruction
                    .operands
                    .contains(&InstructionOperand::Text("+".into()))
            {
                saw_add = true;
            }
        }
        match step.outcome {
            ObservedStepOutcome::Continue => {}
            ObservedStepOutcome::Returned(value) => {
                assert_eq!(value, Value::Number(7));
                final_after = Some(step.after);
                break;
            }
            ObservedStepOutcome::Suspended(_) => panic!("unexpected suspension"),
            ObservedStepOutcome::Yielded(_) => panic!("unexpected yield"),
            ObservedStepOutcome::Failed(error) => panic!("unexpected failure: {error}"),
        }
    }
    assert!(saw_add);
    let after = final_after.expect("return snapshot");
    assert_eq!(after.status, MachineObservationStatus::Returned);
    assert_eq!(after.result.expect("result").display, "7");
}

#[test]
fn static_calls_report_enter_and_return_boundaries() {
    let mut machine = machine("(do (defn f [x] (+ x 1)) (f 41))");
    let registry = crate::embedding_namespace_registry();
    let (kinds, value) =
        crate::core::with_namespace_registry(&registry, || run_observed(&mut machine));
    assert_eq!(value, Value::Number(42));
    assert!(kinds.contains(&ObservationEventKind::CallEnter));
    assert!(kinds.contains(&ObservationEventKind::CallReturn));
}

#[test]
fn caught_runtime_errors_report_an_exact_unwind_boundary() {
    let mut machine = machine("(try (/ 1 0) (catch Exception error 42))");
    let (kinds, value) = run_observed(&mut machine);
    assert_eq!(value, Value::Number(42));
    assert!(kinds.contains(&ObservationEventKind::ExceptionUnwind));
}

#[test]
fn uncaught_runtime_errors_keep_source_and_terminal_diagnostics() {
    let mut machine = machine("(/ 1 0)");
    for _ in 0..32 {
        let step = machine.step_observed();
        match step.outcome {
            ObservedStepOutcome::Continue => {}
            ObservedStepOutcome::Failed(error) => {
                assert_eq!(step.kind, ObservationEventKind::MachineFail);
                assert_eq!(step.status, ObservationEventStatus::Error);
                assert_eq!(step.after.status, MachineObservationStatus::Failed);
                assert_eq!(step.after.error.as_deref(), Some("division by zero"));
                assert_eq!(error.message, "division by zero");
                let source = step.source.expect("source position");
                assert_eq!((source.line, source.column), (1, 1));
                return;
            }
            ObservedStepOutcome::Returned(value) => {
                panic!("unexpected return: {}", value.display())
            }
            ObservedStepOutcome::Suspended(_) => panic!("unexpected suspension"),
            ObservedStepOutcome::Yielded(_) => panic!("unexpected yield"),
        }
    }
    panic!("failure was not observed");
}

#[test]
fn pending_await_resumes_as_one_boundary() {
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
    validate(&program).expect("program must validate");
    let mut machine = Machine::entry(Rc::new(program));

    assert!(matches!(
        machine.step_observed().outcome,
        ObservedStepOutcome::Continue
    ));
    let suspended = machine.step_observed();
    assert_eq!(suspended.kind, ObservationEventKind::MachineSuspend);
    assert_eq!(suspended.after.status, MachineObservationStatus::Suspended);
    assert!(matches!(
        suspended.outcome,
        ObservedStepOutcome::Suspended(_)
    ));

    assert!(promise.resolve(Value::Number(42)));
    let resumed = machine.resume_observed(promise.state());
    assert_eq!(resumed.kind, ObservationEventKind::MachineResume);
    assert_eq!(resumed.after.ip, 2);
    assert!(matches!(resumed.outcome, ObservedStepOutcome::Continue));

    let returned = machine.step_observed();
    match returned.outcome {
        ObservedStepOutcome::Returned(value) => assert_eq!(value, Value::Number(42)),
        _ => panic!("expected return after resume"),
    }
}

#[test]
fn snapshot_limits_bound_stack_and_preserve_the_top() {
    let mut source_map = SourceMap::default();
    for _ in 0..6 {
        source_map.record(None);
    }
    let program = Program {
        namespace: None,
        constants: vec![
            Value::Number(1),
            Value::Number(2),
            Value::Number(3),
            Value::Number(4),
        ],
        var_metadata: Vec::new(),
        schema_types: Default::default(),
        function_types: Default::default(),
        inferred_function_types: Default::default(),
        functions: vec![FunctionPrototype {
            name: Some("bounded-stack-demo".into()),
            async_function: false,
            arity: 0,
            variadic: false,
            capture_count: 0,
            local_count: 0,
            max_stack: 4,
            code: vec![
                Instruction::Constant(0),
                Instruction::Constant(1),
                Instruction::Constant(2),
                Instruction::Constant(3),
                Instruction::BuildVector(4),
                Instruction::Return,
            ],
            source_map,
            handlers: Vec::new(),
        }],
        entry: 0,
    };
    validate(&program).expect("program must validate");
    let mut machine = Machine::entry(Rc::new(program));
    for _ in 0..4 {
        assert!(matches!(
            machine.step_observed().outcome,
            ObservedStepOutcome::Continue
        ));
    }
    let snapshot = machine.snapshot_with_limits(ObservationLimits {
        stack: 2,
        ..ObservationLimits::default()
    });
    assert_eq!(snapshot.stack_omitted, 2);
    assert_eq!(
        snapshot
            .stack
            .iter()
            .map(|value| value.display.as_str())
            .collect::<Vec<_>>(),
        vec!["3", "4"]
    );
}

#[test]
fn declaration_and_runtime_instructions_have_stable_observation_operands() {
    let cases = [
        (
            Instruction::DefProtocol(1),
            "def-protocol",
            vec![InstructionOperand::Unsigned(1)],
        ),
        (
            Instruction::ExtendType(2),
            "extend-type",
            vec![InstructionOperand::Unsigned(2)],
        ),
        (
            Instruction::DefMulti(3),
            "def-multi",
            vec![InstructionOperand::Unsigned(3)],
        ),
        (
            Instruction::DefMethod(4),
            "def-method",
            vec![InstructionOperand::Unsigned(4)],
        ),
        (
            Instruction::IntrinsicCall { target: 9, argc: 2 },
            "intrinsic-call",
            vec![
                InstructionOperand::Unsigned(9),
                InstructionOperand::Unsigned(2),
            ],
        ),
        (
            Instruction::BuiltinValue(5),
            "builtin-value",
            vec![InstructionOperand::Unsigned(5)],
        ),
        (
            Instruction::DynamicBind(6),
            "dynamic-bind",
            vec![InstructionOperand::Unsigned(6)],
        ),
        (
            Instruction::DynamicUnbind(7),
            "dynamic-unbind",
            vec![InstructionOperand::Unsigned(7)],
        ),
        (
            Instruction::DotCall { method: 8, argc: 2 },
            "dot-call",
            vec![
                InstructionOperand::Unsigned(8),
                InstructionOperand::Unsigned(2),
            ],
        ),
    ];

    for (instruction, opcode, operands) in cases {
        let snapshot = instruction_snapshot(&instruction);
        assert_eq!(snapshot.opcode, opcode);
        assert_eq!(snapshot.operands, operands);
    }
}
