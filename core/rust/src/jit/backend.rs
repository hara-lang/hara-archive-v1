use super::trace_ir::{
    ExitReason, ExitSnapshot, NumericVectorSlice, Trace, TraceOp, TraceOutcome, TraceValue,
};
use crate::core::{IntrinsicOp, Value};

pub trait TraceBackend {
    type Compiled;
    fn compile(&mut self, trace: &Trace) -> Result<Self::Compiled, String>;
    fn enter(
        &mut self,
        trace: &mut Self::Compiled,
        locals: &mut [TraceValue],
        max_iterations: u32,
    ) -> TraceOutcome;
}

#[derive(Default)]
pub struct CheckedBackend;

impl TraceBackend for CheckedBackend {
    type Compiled = Trace;

    fn compile(&mut self, trace: &Trace) -> Result<Trace, String> {
        Ok(trace.clone())
    }

    fn enter(
        &mut self,
        trace: &mut Trace,
        locals: &mut [TraceValue],
        max_iterations: u32,
    ) -> TraceOutcome {
        for operation in &trace.operations {
            let valid = match operation {
                TraceOp::GuardLocalI64 { local } => {
                    matches!(locals.get(usize::from(*local)), Some(TraceValue::I64(_)))
                }
                TraceOp::GuardLocalBool { local } => {
                    matches!(locals.get(usize::from(*local)), Some(TraceValue::Bool(_)))
                }
                TraceOp::GuardLocalNil { local } => {
                    matches!(locals.get(usize::from(*local)), Some(TraceValue::Nil))
                }
                TraceOp::GuardLocalVectorI64 { local } => {
                    locals.get(usize::from(*local)).is_some_and(numeric_vector)
                }
                _ => true,
            };
            if !valid {
                return TraceOutcome::SideExit {
                    reason: ExitReason::WrongTag,
                    iterations: 0,
                    snapshot: ExitSnapshot {
                        function: trace.function,
                        instruction: trace.resume_ip,
                        locals: locals.to_vec(),
                        stack: Vec::new(),
                    },
                };
            }
        }
        let mut iterations = 0;
        let mut stack = Vec::with_capacity(8);
        while iterations < max_iterations {
            let checkpoint = locals.to_vec();
            stack.clear();
            for operation in &trace.operations {
                let exit = |reason| TraceOutcome::SideExit {
                    reason,
                    iterations,
                    snapshot: ExitSnapshot {
                        function: trace.function,
                        instruction: trace.resume_ip,
                        locals: checkpoint.clone(),
                        stack: Vec::new(),
                    },
                };
                match *operation {
                    TraceOp::GuardLocalI64 { .. }
                    | TraceOp::GuardLocalBool { .. }
                    | TraceOp::GuardLocalNil { .. }
                    | TraceOp::GuardLocalVectorI64 { .. } => {}
                    TraceOp::LoadLocal { local } => match locals.get(local as usize).cloned() {
                        Some(value) => stack.push(value),
                        None => return exit(ExitReason::WrongTag),
                    },
                    TraceOp::ConstantI64(value) => stack.push(TraceValue::I64(value)),
                    TraceOp::ConstantBool(value) => stack.push(TraceValue::Bool(value)),
                    TraceOp::ConstantNil => stack.push(TraceValue::Nil),
                    TraceOp::ConstantVectorI64 { vector } => {
                        let Some(values) = trace.vectors.get(usize::from(vector)) else {
                            return exit(ExitReason::Unsupported);
                        };
                        stack.push(TraceValue::Indexed(Box::new(Value::Vector(
                            values.iter().copied().map(Value::Number).collect(),
                        ))));
                    }
                    TraceOp::StoreLocal { local } => {
                        let Some(value) = stack.pop() else {
                            return exit(ExitReason::Unsupported);
                        };
                        let Some(slot) = locals.get_mut(local as usize) else {
                            return exit(ExitReason::Unsupported);
                        };
                        *slot = value;
                    }
                    TraceOp::Pop => {
                        stack.pop();
                    }
                    TraceOp::GuardTruthy { expected } => {
                        let Some(value) = stack.pop() else {
                            return exit(ExitReason::Unsupported);
                        };
                        let truthy = !matches!(value, TraceValue::Bool(false) | TraceValue::Nil);
                        if truthy != expected {
                            return exit(ExitReason::BranchChanged);
                        }
                    }
                    TraceOp::BinaryI64(op) => {
                        let (Some(TraceValue::I64(right)), Some(TraceValue::I64(left))) =
                            (stack.pop(), stack.pop())
                        else {
                            return exit(ExitReason::WrongTag);
                        };
                        let value = match op {
                            IntrinsicOp::Add => left.checked_add(right).map(TraceValue::I64),
                            IntrinsicOp::Subtract => left.checked_sub(right).map(TraceValue::I64),
                            IntrinsicOp::Multiply => left.checked_mul(right).map(TraceValue::I64),
                            IntrinsicOp::Divide | IntrinsicOp::Remainder | IntrinsicOp::Modulo
                                if right == 0 =>
                            {
                                return exit(ExitReason::DivisionByZero)
                            }
                            IntrinsicOp::Divide => left.checked_div(right).map(TraceValue::I64),
                            IntrinsicOp::Remainder | IntrinsicOp::Modulo => {
                                if left == i64::MIN && right == -1 {
                                    Some(TraceValue::I64(0))
                                } else {
                                    let Some(remainder) = left.checked_rem(right) else {
                                        return exit(ExitReason::Overflow);
                                    };
                                    Some(TraceValue::I64(remainder))
                                }
                            }
                            IntrinsicOp::Less => Some(TraceValue::Bool(left < right)),
                            IntrinsicOp::LessOrEqual => Some(TraceValue::Bool(left <= right)),
                            IntrinsicOp::Greater => Some(TraceValue::Bool(left > right)),
                            IntrinsicOp::GreaterOrEqual => Some(TraceValue::Bool(left >= right)),
                            IntrinsicOp::Equal => Some(TraceValue::Bool(left == right)),
                            _ => return exit(ExitReason::Unsupported),
                        };
                        let Some(value) = value else {
                            return exit(ExitReason::Overflow);
                        };
                        stack.push(value);
                    }
                    TraceOp::VectorCountI64 => {
                        let Some(vector) =
                            stack.pop().and_then(|value| numeric_vector_values(&value))
                        else {
                            return exit(ExitReason::WrongTag);
                        };
                        stack.push(TraceValue::I64(vector.len() as i64));
                    }
                    TraceOp::VectorFirstI64 | TraceOp::VectorSecondI64 => {
                        let index = usize::from(matches!(*operation, TraceOp::VectorSecondI64));
                        let Some(vector) =
                            stack.pop().and_then(|value| numeric_vector_values(&value))
                        else {
                            return exit(ExitReason::WrongTag);
                        };
                        let Some(value) = vector.get(index).copied() else {
                            return exit(ExitReason::IndexOutOfBounds);
                        };
                        stack.push(TraceValue::I64(value));
                    }
                    TraceOp::VectorRestI64 => {
                        let Some(vector) =
                            stack.pop().and_then(|value| numeric_vector_values(&value))
                        else {
                            return exit(ExitReason::WrongTag);
                        };
                        stack.push(TraceValue::VectorSlice(Box::new(NumericVectorSlice {
                            start: usize::from(!vector.is_empty()),
                            values: vector,
                        })));
                    }
                    TraceOp::VectorNthI64 => {
                        let Some(TraceValue::I64(index)) = stack.pop() else {
                            return exit(ExitReason::WrongTag);
                        };
                        let Some(vector) = stack.pop() else {
                            return exit(ExitReason::WrongTag);
                        };
                        let Some(index) = usize::try_from(index).ok() else {
                            return exit(ExitReason::IndexOutOfBounds);
                        };
                        let Some(values) = numeric_vector_values(&vector) else {
                            return exit(ExitReason::WrongTag);
                        };
                        let Some(value) = values.get(index).copied() else {
                            return exit(ExitReason::IndexOutOfBounds);
                        };
                        stack.push(TraceValue::I64(value));
                    }
                    TraceOp::LoopBackedge => iterations += 1,
                }
            }
        }
        TraceOutcome::Completed { iterations }
    }
}

fn numeric_vector(value: &TraceValue) -> bool {
    numeric_vector_values(value).is_some()
}

fn numeric_vector_values(value: &TraceValue) -> Option<Vec<i64>> {
    match value {
        TraceValue::Indexed(value) => match value.as_ref() {
            Value::Tuple(values) => values
                .iter()
                .map(|value| match value {
                    Value::Number(value) => Some(*value),
                    _ => None,
                })
                .collect(),
            Value::Vector(values) => values
                .iter()
                .map(|value| match value {
                    Value::Number(value) => Some(*value),
                    _ => None,
                })
                .collect(),
            _ => None,
        },
        TraceValue::VectorSlice(slice) => Some(slice.values[slice.start..].to_vec()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn increment_trace() -> Trace {
        Trace {
            function: 0,
            header: 2,
            resume_ip: 2,
            operations: vec![
                TraceOp::GuardLocalI64 { local: 0 },
                TraceOp::LoadLocal { local: 0 },
                TraceOp::ConstantI64(1),
                TraceOp::BinaryI64(IntrinsicOp::Add),
                TraceOp::StoreLocal { local: 0 },
                TraceOp::LoopBackedge,
            ],
            vectors: Vec::new(),
        }
    }

    #[test]
    fn checked_backend_executes_and_guards() {
        let trace = increment_trace();
        let mut backend = CheckedBackend;
        let mut compiled = backend.compile(&trace).unwrap();
        let mut locals = [TraceValue::I64(0)];
        assert_eq!(
            backend.enter(&mut compiled, &mut locals, 5),
            TraceOutcome::Completed { iterations: 5 }
        );
        assert_eq!(locals[0], TraceValue::I64(5));
        let mut wrong = [TraceValue::Bool(false)];
        assert!(matches!(
            backend.enter(&mut compiled, &mut wrong, 1),
            TraceOutcome::SideExit {
                reason: ExitReason::WrongTag,
                ..
            }
        ));
    }

    #[test]
    fn checked_backend_indexes_numeric_vector_constants_and_exits_on_bounds() {
        let trace = Trace {
            function: 0,
            header: 0,
            resume_ip: 0,
            operations: vec![
                TraceOp::ConstantVectorI64 { vector: 0 },
                TraceOp::LoadLocal { local: 0 },
                TraceOp::VectorNthI64,
                TraceOp::StoreLocal { local: 1 },
                TraceOp::LoopBackedge,
            ],
            vectors: vec![vec![10, 20, 30]],
        };
        let mut backend = CheckedBackend;
        let mut compiled = backend.compile(&trace).unwrap();
        let mut locals = [TraceValue::I64(1), TraceValue::Nil];
        assert_eq!(
            backend.enter(&mut compiled, &mut locals, 1),
            TraceOutcome::Completed { iterations: 1 }
        );
        assert_eq!(locals[1], TraceValue::I64(20));

        locals[0] = TraceValue::I64(3);
        assert!(matches!(
            backend.enter(&mut compiled, &mut locals, 1),
            TraceOutcome::SideExit {
                reason: ExitReason::IndexOutOfBounds,
                ..
            }
        ));
    }

    #[test]
    fn checked_backend_restores_the_iteration_checkpoint_on_failure() {
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
            vectors: Vec::new(),
        };
        let mut compiled = trace.clone();
        let mut locals = [TraceValue::I64(9)];
        assert!(matches!(
            CheckedBackend.enter(&mut compiled, &mut locals, 1),
            TraceOutcome::SideExit {
                reason: ExitReason::DivisionByZero,
                snapshot: ExitSnapshot { locals, .. },
                ..
            } if locals == vec![TraceValue::I64(9)]
        ));
    }
}
