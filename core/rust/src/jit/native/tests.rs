use super::*;

fn compile(operations: Vec<TraceOp>, vectors: Vec<Vec<i64>>) -> (NativeBackend, NativeTrace) {
    let trace = Trace {
        function: 0,
        header: 0,
        resume_ip: 0,
        operations,
        vectors,
    };
    let mut backend = NativeBackend::default();
    let compiled = backend.compile(&trace).unwrap();
    (backend, compiled)
}

#[test]
fn cranelift_backend_executes_guarded_host_code() {
    let (mut backend, mut compiled) = compile(
        vec![
            TraceOp::GuardLocalI64 { local: 0 },
            TraceOp::LoadLocal { local: 0 },
            TraceOp::ConstantI64(1),
            TraceOp::BinaryI64(IntrinsicOp::Add),
            TraceOp::StoreLocal { local: 0 },
            TraceOp::LoopBackedge,
        ],
        vec![],
    );
    let mut locals = [TraceValue::I64(2)];
    assert_eq!(
        backend.enter(&mut compiled, &mut locals, 10),
        TraceOutcome::Completed { iterations: 10 }
    );
    assert_eq!(locals[0], TraceValue::I64(12));
}

#[test]
fn native_backend_multiplies_and_side_exits_on_overflow() {
    let (mut backend, mut compiled) = compile(
        vec![
            TraceOp::GuardLocalI64 { local: 0 },
            TraceOp::LoadLocal { local: 0 },
            TraceOp::ConstantI64(3),
            TraceOp::BinaryI64(IntrinsicOp::Multiply),
            TraceOp::StoreLocal { local: 0 },
            TraceOp::LoopBackedge,
        ],
        vec![],
    );
    let mut locals = [TraceValue::I64(2)];
    assert_eq!(
        backend.enter(&mut compiled, &mut locals, 3),
        TraceOutcome::Completed { iterations: 3 }
    );
    assert_eq!(locals[0], TraceValue::I64(54));
    locals[0] = TraceValue::I64(i64::MAX);
    assert!(matches!(
        backend.enter(&mut compiled, &mut locals, 1),
        TraceOutcome::SideExit {
            reason: ExitReason::Overflow,
            ..
        }
    ));
}

#[test]
fn cranelift_backend_indexes_vector_constants_and_exits_on_bounds() {
    let (mut backend, mut compiled) = compile(
        vec![
            TraceOp::ConstantVectorI64 { vector: 0 },
            TraceOp::LoadLocal { local: 0 },
            TraceOp::VectorNthI64,
            TraceOp::StoreLocal { local: 1 },
            TraceOp::LoopBackedge,
        ],
        vec![vec![10, 20, 30]],
    );
    let mut locals = [TraceValue::I64(2), TraceValue::I64(0)];
    assert_eq!(
        backend.enter(&mut compiled, &mut locals, 1),
        TraceOutcome::Completed { iterations: 1 }
    );
    assert_eq!(locals[1], TraceValue::I64(30));
    locals[0] = TraceValue::I64(-1);
    assert!(matches!(
        backend.enter(&mut compiled, &mut locals, 1),
        TraceOutcome::SideExit {
            reason: ExitReason::IndexOutOfBounds,
            ..
        }
    ));
}

#[test]
fn cranelift_backend_indexes_guarded_vector_locals() {
    let (mut backend, mut compiled) = compile(
        vec![
            TraceOp::GuardLocalVectorI64 { local: 0 },
            TraceOp::LoadLocal { local: 0 },
            TraceOp::LoadLocal { local: 1 },
            TraceOp::VectorNthI64,
            TraceOp::StoreLocal { local: 2 },
            TraceOp::LoopBackedge,
        ],
        vec![],
    );
    let mut locals = [
        TraceValue::Indexed(Box::new(crate::core::Value::Vector(
            [10, 20, 30]
                .into_iter()
                .map(crate::core::Value::Number)
                .collect(),
        ))),
        TraceValue::I64(1),
        TraceValue::I64(0),
    ];
    assert_eq!(
        backend.enter(&mut compiled, &mut locals, 1),
        TraceOutcome::Completed { iterations: 1 }
    );
    assert_eq!(locals[2], TraceValue::I64(20));
    assert!(matches!(locals[0], TraceValue::Indexed(_)));
}

#[test]
fn native_backend_restores_the_iteration_checkpoint_on_failure() {
    let trace = Trace {
        function: 0,
        header: 4,
        resume_ip: 4,
        operations: vec![
            TraceOp::LoadLocal { local: 0 },
            TraceOp::ConstantI64(1),
            TraceOp::BinaryI64(IntrinsicOp::Add),
            TraceOp::StoreLocal { local: 0 },
            TraceOp::LoadLocal { local: 0 },
            TraceOp::ConstantI64(0),
            TraceOp::BinaryI64(IntrinsicOp::Divide),
            TraceOp::Pop,
            TraceOp::LoopBackedge,
        ],
        vectors: vec![],
    };
    let mut backend = NativeBackend::default();
    let mut compiled = backend.compile(&trace).unwrap();
    let mut locals = [TraceValue::I64(9)];
    assert!(
        matches!(backend.enter(&mut compiled, &mut locals, 1), TraceOutcome::SideExit { reason: ExitReason::DivisionByZero, snapshot: ExitSnapshot { locals, .. }, .. } if locals == vec![TraceValue::I64(9)])
    );
}
