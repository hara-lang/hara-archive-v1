use super::trace_ir::{Trace, TraceOp, TraceValue};
use crate::core::{IntrinsicOp, Value};
use crate::vm::{Instruction, Program};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordError {
    InvalidRange,
    InvalidStack,
    TooLong,
    UnsupportedInstruction(u32),
    UnsupportedConstant(u32),
    UnsupportedLocal(u16),
}

pub struct TraceRecorder {
    max_operations: usize,
}

impl TraceRecorder {
    pub fn new(max_operations: usize) -> Self {
        Self { max_operations }
    }

    pub fn record_loop(
        &self,
        program: &Program,
        function: u16,
        header: u32,
        backedge: u32,
        locals: &[TraceValue],
    ) -> Result<Trace, RecordError> {
        let path = (header..=backedge).collect::<Vec<_>>();
        self.record_path(program, function, header, &path, locals)
    }

    /// Lowers the concrete instruction path observed by the VM. Forward
    /// branches disappear into the linear trace; their observed direction is
    /// retained as a guard.
    pub fn record_path(
        &self,
        program: &Program,
        function: u16,
        header: u32,
        path: &[u32],
        locals: &[TraceValue],
    ) -> Result<Trace, RecordError> {
        let prototype = program
            .functions
            .get(function as usize)
            .ok_or(RecordError::InvalidRange)?;
        if path.first() != Some(&header) || path.is_empty() {
            return Err(RecordError::InvalidRange);
        }
        let mut operations = Vec::new();
        let mut vectors = Vec::new();
        for (index, absolute) in path.iter().copied().enumerate() {
            let instruction = prototype
                .code
                .get(absolute as usize)
                .ok_or(RecordError::InvalidRange)?;
            let next = path.get(index + 1).copied().unwrap_or(header);
            match instruction {
                Instruction::LoadLocal(local) => {
                    operations.push(match locals.get(usize::from(*local)) {
                        Some(TraceValue::I64(_)) => TraceOp::GuardLocalI64 { local: *local },
                        Some(TraceValue::Bool(_)) => TraceOp::GuardLocalBool { local: *local },
                        Some(TraceValue::Nil) => TraceOp::GuardLocalNil { local: *local },
                        Some(TraceValue::Indexed(value)) if numeric_vector(value).is_some() => {
                            TraceOp::GuardLocalVectorI64 { local: *local }
                        }
                        _ => return Err(RecordError::UnsupportedLocal(*local)),
                    });
                    operations.push(TraceOp::LoadLocal { local: *local });
                }
                Instruction::StoreLocal(local) => {
                    operations.push(TraceOp::StoreLocal { local: *local })
                }
                Instruction::Constant(index) => match program.constants.get(*index as usize) {
                    Some(Value::Number(value)) => operations.push(TraceOp::ConstantI64(*value)),
                    Some(Value::Bool(value)) => operations.push(TraceOp::ConstantBool(*value)),
                    Some(Value::Nil) => operations.push(TraceOp::ConstantNil),
                    Some(value @ (Value::Tuple(_) | Value::Vector(_))) => {
                        let values = numeric_vector(value)
                            .ok_or(RecordError::UnsupportedConstant(*index))?;
                        let vector =
                            u16::try_from(vectors.len()).map_err(|_| RecordError::TooLong)?;
                        vectors.push(values);
                        operations.push(TraceOp::ConstantVectorI64 { vector });
                    }
                    _ => return Err(RecordError::UnsupportedConstant(*index)),
                },
                Instruction::Nil => operations.push(TraceOp::ConstantNil),
                Instruction::True => operations.push(TraceOp::ConstantBool(true)),
                Instruction::False => operations.push(TraceOp::ConstantBool(false)),
                Instruction::IntrinsicCall { target, argc: 2 }
                    if intrinsic_op(program, *target).is_some_and(binary_i64) =>
                {
                    operations.push(TraceOp::BinaryI64(
                        intrinsic_op(program, *target).expect("guarded intrinsic operator"),
                    ));
                }
                Instruction::IntrinsicCall { target, argc: 1 }
                    if vector_operation(program, *target).is_some() =>
                {
                    operations.push(vector_operation(program, *target).expect("guarded vector op"));
                }
                Instruction::ProtocolCall { target, argc: 1 }
                    if vector_operation(program, *target).is_some() =>
                {
                    operations.push(vector_operation(program, *target).expect("guarded vector op"));
                }
                Instruction::ProtocolCall { target, argc: 2 }
                    if vector_operation(program, *target).is_some() =>
                {
                    operations.push(vector_operation(program, *target).expect("guarded vector op"));
                }
                Instruction::JumpIfFalse(target) => {
                    let expected = next != *target;
                    if next != absolute + 1 && next != *target {
                        return Err(RecordError::InvalidRange);
                    }
                    operations.push(TraceOp::GuardTruthy { expected })
                }
                Instruction::Pop => operations.push(TraceOp::Pop),
                Instruction::Jump(target) if *target == next => {
                    if *target == header {
                        operations.push(TraceOp::LoopBackedge)
                    }
                }
                _ => return Err(RecordError::UnsupportedInstruction(absolute)),
            }
            if operations.len() > self.max_operations {
                return Err(RecordError::TooLong);
            }
        }
        if !matches!(operations.last(), Some(TraceOp::LoopBackedge)) {
            return Err(RecordError::InvalidRange);
        }
        if !valid_types(&operations, locals) {
            return Err(RecordError::InvalidStack);
        }
        Ok(Trace {
            function,
            header,
            resume_ip: header,
            operations,
            vectors,
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TraceType {
    I64,
    Bool,
    Nil,
    Vector,
    Slice,
}

fn valid_types(operations: &[TraceOp], entry_locals: &[TraceValue]) -> bool {
    use TraceType::*;
    let mut locals = entry_locals
        .iter()
        .enumerate()
        .filter_map(|(index, value)| {
            let kind = match value {
                TraceValue::I64(_) => I64,
                TraceValue::Bool(_) => Bool,
                TraceValue::Nil => Nil,
                TraceValue::Indexed(value) if numeric_vector(value).is_some() => Vector,
                _ => return None,
            };
            Some((index as u16, kind))
        })
        .collect::<std::collections::HashMap<_, _>>();
    let entry_types = locals.clone();
    let mut stack = Vec::new();
    for operation in operations {
        match *operation {
            TraceOp::GuardLocalI64 { local } => {
                locals.insert(local, I64);
            }
            TraceOp::GuardLocalBool { local } => {
                locals.insert(local, Bool);
            }
            TraceOp::GuardLocalNil { local } => {
                locals.insert(local, Nil);
            }
            TraceOp::GuardLocalVectorI64 { local } => {
                locals.insert(local, Vector);
            }
            TraceOp::LoadLocal { local } => {
                let Some(kind) = locals.get(&local).copied() else {
                    return false;
                };
                stack.push(kind);
            }
            TraceOp::ConstantI64(_) => stack.push(I64),
            TraceOp::ConstantBool(_) => stack.push(Bool),
            TraceOp::ConstantNil => stack.push(Nil),
            TraceOp::ConstantVectorI64 { .. } => stack.push(Vector),
            TraceOp::BinaryI64(op) => {
                if stack.pop() != Some(I64) || stack.pop() != Some(I64) {
                    return false;
                }
                stack.push(
                    if matches!(
                        op,
                        IntrinsicOp::Equal
                            | IntrinsicOp::Less
                            | IntrinsicOp::LessOrEqual
                            | IntrinsicOp::Greater
                            | IntrinsicOp::GreaterOrEqual
                    ) {
                        Bool
                    } else {
                        I64
                    },
                );
            }
            TraceOp::VectorCountI64 => {
                if !matches!(stack.pop(), Some(Vector | Slice)) {
                    return false;
                }
                stack.push(I64);
            }
            TraceOp::VectorFirstI64 | TraceOp::VectorSecondI64 => {
                if !matches!(stack.pop(), Some(Vector | Slice)) {
                    return false;
                }
                stack.push(I64);
            }
            TraceOp::VectorRestI64 => {
                if !matches!(stack.pop(), Some(Vector | Slice)) {
                    return false;
                }
                stack.push(Slice);
            }
            TraceOp::VectorNthI64 => {
                if stack.pop() != Some(I64) || !matches!(stack.pop(), Some(Vector | Slice)) {
                    return false;
                }
                stack.push(I64);
            }
            TraceOp::StoreLocal { local } => {
                let Some(kind) = stack.pop() else {
                    return false;
                };
                if matches!(kind, Vector | Slice)
                    || entry_types.get(&local).is_some_and(|entry| *entry != kind)
                    || !entry_types.contains_key(&local)
                {
                    return false;
                }
                locals.insert(local, kind);
            }
            TraceOp::GuardTruthy { .. } => {
                if !matches!(stack.pop(), Some(Bool | Nil)) {
                    return false;
                }
            }
            TraceOp::Pop => {
                if stack.pop().is_none() {
                    return false;
                }
            }
            TraceOp::LoopBackedge => {
                if !stack.is_empty() {
                    return false;
                }
            }
        }
    }
    stack.is_empty()
}

fn binary_i64(op: IntrinsicOp) -> bool {
    matches!(
        op,
        IntrinsicOp::Add
            | IntrinsicOp::Subtract
            | IntrinsicOp::Multiply
            | IntrinsicOp::Divide
            | IntrinsicOp::Remainder
            | IntrinsicOp::Modulo
            | IntrinsicOp::Less
            | IntrinsicOp::LessOrEqual
            | IntrinsicOp::Greater
            | IntrinsicOp::GreaterOrEqual
            | IntrinsicOp::Equal
    )
}

fn target_name(program: &Program, target: u32) -> Option<&str> {
    match program.constants.get(target as usize) {
        Some(Value::String(name)) => Some(name),
        _ => None,
    }
}

fn intrinsic_op(program: &Program, target: u32) -> Option<IntrinsicOp> {
    target_name(program, target).and_then(IntrinsicOp::from_symbol)
}

fn vector_operation(program: &Program, target: u32) -> Option<TraceOp> {
    let name = target_name(program, target)?;
    if name == "first" || name.ends_with("/first") {
        Some(TraceOp::VectorFirstI64)
    } else if name == "rest" || name.ends_with("/rest") {
        Some(TraceOp::VectorRestI64)
    } else if name == "second" || name.ends_with("/second") {
        Some(TraceOp::VectorSecondI64)
    } else if name == "count" || name.ends_with("/count") {
        Some(TraceOp::VectorCountI64)
    } else if name == "nth" || name.ends_with("/nth") {
        Some(TraceOp::VectorNthI64)
    } else {
        None
    }
}

fn numeric_vector(value: &Value) -> Option<Vec<i64>> {
    let values: Box<dyn Iterator<Item = &Value> + '_> = match value {
        Value::Tuple(values) => Box::new(values.iter()),
        Value::Vector(values) => Box::new(values.iter()),
        _ => return None,
    };
    values
        .map(|value| match value {
            Value::Number(value) => Some(*value),
            _ => None,
        })
        .collect()
}
