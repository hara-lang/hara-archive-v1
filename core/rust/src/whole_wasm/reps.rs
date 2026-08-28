use std::collections::VecDeque;

use crate::core::{IntrinsicOp, Value};
use crate::kernel::{FunctionSchema, SchemaType};
use crate::vm::{FunctionId, FunctionPrototype, Instruction, Program};

use super::ir::Rep;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RepState {
    pub locals: Vec<Rep>,
    pub stack: Vec<Rep>,
}

/// Point-sensitive representation analysis over bytecode control flow.
/// Facts attach to instruction entry states because VM stack positions are
/// reused for values with different representations.
pub(crate) fn analyze_function(
    program: &Program,
    function_id: FunctionId,
    function: &FunctionPrototype,
) -> Result<Vec<RepState>, String> {
    let mut entry = RepState {
        locals: vec![Rep::Unknown; usize::from(function.local_count)],
        stack: Vec::new(),
    };
    let native_collection = function_native_collection_parameter(program, function);
    let scalar_kernel = function_is_scalar_kernel(program, function);
    for parameter in 0..usize::from(function.arity) {
        entry.locals[parameter] =
            if let Some(rep) = declared_parameter_rep(program, function_id, function, parameter) {
                rep
            } else if parameter == 0 {
                native_collection.unwrap_or_else(|| {
                    if scalar_kernel || program.function_has_i64_parameters(function_id) {
                        Rep::I64
                    } else if function_uses_tagged_collections(program, function)
                        && !program_uses_collection_constants(program)
                    {
                        Rep::TaggedRef
                    } else {
                        Rep::TruthyHandle
                    }
                })
            } else if scalar_kernel
                || native_collection == Some(Rep::ArrayRef)
                || program.function_has_i64_parameters(function_id)
            {
                Rep::I64
            } else if function_uses_tagged_collections(program, function)
                && !program_uses_collection_constants(program)
            {
                Rep::TaggedRef
            } else {
                Rep::TruthyHandle
            };
    }
    let mut states = vec![None; function.code.len()];
    states[0] = Some(entry);
    let mut work = VecDeque::from([0usize]);
    while let Some(ip) = work.pop_front() {
        let mut output = states[ip].clone().expect("queued state");
        transfer(
            program,
            function_id,
            ip,
            &function.code[ip],
            function.code.get(ip + 1),
            &mut output,
        )?;
        for successor in successors(ip, &function.code[ip], function.code.len()) {
            let changed = match &mut states[successor] {
                Some(existing) => join(existing, &output)?,
                slot @ None => {
                    *slot = Some(output.clone());
                    true
                }
            };
            if changed {
                work.push_back(successor);
            }
        }
    }
    states
        .into_iter()
        .enumerate()
        .map(|(ip, state)| {
            state.ok_or_else(|| {
                format!("whole-Wasm representation analysis cannot reach instruction {ip}")
            })
        })
        .collect()
}

fn declared_function_arity<'a>(
    program: &'a Program,
    function_id: FunctionId,
    function: &FunctionPrototype,
) -> Option<&'a FunctionSchema> {
    let SchemaType::Function(arities) = program.function_schema(function_id)? else {
        return None;
    };
    arities.iter().find(|arity| {
        arity.fixed.len() == usize::from(function.arity)
            && arity.rest.is_some() == function.variadic
    })
}

pub(super) fn declared_parameter_rep(
    program: &Program,
    function_id: FunctionId,
    function: &FunctionPrototype,
    parameter: usize,
) -> Option<Rep> {
    let arity = declared_function_arity(program, function_id, function)?;
    arity.fixed.get(parameter).map(schema_rep)
}

pub(super) fn declared_result_rep(program: &Program, function_id: FunctionId) -> Option<Rep> {
    let function = program.functions.get(usize::from(function_id))?;
    let arity = declared_function_arity(program, function_id, function)?;
    Some(schema_rep(&arity.output))
}

fn schema_rep(schema: &SchemaType) -> Rep {
    match schema {
        SchemaType::Primitive(name) if name == "int" || name == "long" => Rep::I64,
        SchemaType::Primitive(name) if name == "bool" => Rep::Bool,
        SchemaType::Primitive(name) if name == "nil" => Rep::Nil,
        _ => Rep::TruthyHandle,
    }
}

fn function_is_scalar_kernel(program: &Program, function: &FunctionPrototype) -> bool {
    let mut saw_numeric_operation = false;
    for instruction in &function.code {
        match instruction {
            Instruction::ProtocolCall { .. } => return false,
            Instruction::IntrinsicCall { .. } => {
                let Some(op) = instruction_intrinsic_op(program, instruction) else {
                    return false;
                };
                if !matches!(
                    op,
                    IntrinsicOp::Add
                        | IntrinsicOp::Subtract
                        | IntrinsicOp::Multiply
                        | IntrinsicOp::Divide
                        | IntrinsicOp::Remainder
                        | IntrinsicOp::Modulo
                        | IntrinsicOp::Equal
                        | IntrinsicOp::Less
                        | IntrinsicOp::LessOrEqual
                        | IntrinsicOp::Greater
                        | IntrinsicOp::GreaterOrEqual
                ) {
                    return false;
                }
                saw_numeric_operation = true;
            }
            _ => {}
        }
    }
    saw_numeric_operation
}

/// Native collection primitives take their collection as their first
/// argument. Hara collection algorithms conventionally preserve that value
/// as the first function parameter, including across recursive calls. Treat
/// those operations as a representation constraint on that parameter so a
/// linear-memory reference is never mistaken for a boxed runtime handle.
fn function_native_collection_parameter(
    program: &Program,
    function: &FunctionPrototype,
) -> Option<Rep> {
    let mut inferred = None;
    for instruction in &function.code {
        let candidate = match instruction_target(program, instruction) {
            Some("std.native.Arr/get" | "std.native.Arr/set") => Some(Rep::ArrayRef),
            Some("std.native.Obj/get" | "std.native.Obj/set") => Some(Rep::ObjectRef),
            _ => None,
        };
        if let Some(candidate) = candidate {
            match inferred {
                None => inferred = Some(candidate),
                Some(current) if current == candidate => {}
                Some(_) => return None,
            }
        }
    }
    inferred
}

fn transfer(
    program: &Program,
    function: FunctionId,
    ip: usize,
    instruction: &Instruction,
    next_instruction: Option<&Instruction>,
    state: &mut RepState,
) -> Result<(), String> {
    let pop = |state: &mut RepState| {
        state.stack.pop().ok_or_else(|| {
            format!("whole-Wasm representation stack underflow in function {function} at {ip}")
        })
    };
    match instruction {
        Instruction::Constant(index) => state.stack.push(constant_rep(
            program
                .constants
                .get(*index as usize)
                .ok_or_else(|| format!("whole-Wasm representation constant {index} is missing"))?,
        )),
        Instruction::Nil => state.stack.push(Rep::Nil),
        Instruction::True | Instruction::False => state.stack.push(Rep::Bool),
        Instruction::LoadLocal(local) => state.stack.push(state.locals[usize::from(*local)]),
        Instruction::StoreLocal(local) => state.locals[usize::from(*local)] = pop(state)?,
        Instruction::Dup => state.stack.push(*state.stack.last().ok_or_else(|| {
            format!("whole-Wasm representation stack underflow in function {function} at {ip}")
        })?),
        Instruction::Pop | Instruction::JumpIfFalse(_) => {
            pop(state)?;
        }
        Instruction::IntrinsicCall { target, argc }
        | Instruction::ProtocolCall { target, argc } => {
            let start = state.stack.len() - usize::from(*argc);
            let arguments = state.stack.split_off(start);
            let target = target_name(program, *target)?;
            state.stack.push(if target.ends_with("/lookup")
                && next_instruction.is_some_and(|next| is_scalar_numeric_consumer(program, next))
            {
                Rep::I64
            } else {
                target_rep(target, &arguments)
            });
        }
        Instruction::CallStatic { argc, prototype } => {
            let start = state.stack.len() - usize::from(*argc);
            state.stack.truncate(start);
            state
                .stack
                .push(declared_result_rep(program, *prototype).unwrap_or(Rep::I64));
        }
        Instruction::Closure {
            prototype,
            captures: 0,
        } => state.stack.push(Rep::FunctionRef(*prototype)),
        Instruction::DefGlobal { .. } => {}
        Instruction::GetGlobal(index) | Instruction::VarGlobal(index) => {
            state
                .stack
                .push(resolve_function(program, *index).map_or(Rep::Unknown, Rep::FunctionRef));
        }
        Instruction::Call { argc } => {
            let start = state.stack.len() - usize::from(*argc) - 1;
            let result = match state.stack.get(start) {
                Some(Rep::FunctionRef(prototype)) => {
                    declared_result_rep(program, *prototype).unwrap_or(Rep::I64)
                }
                _ => Rep::I64,
            };
            state.stack.truncate(start);
            state.stack.push(result);
        }
        Instruction::BuildVector(count) => {
            let start = state.stack.len() - usize::from(*count);
            let values = state.stack.split_off(start);
            state.stack.push(
                if function_enables_tagged_vectors(program, function)
                    && values
                        .iter()
                        .all(|rep| matches!(rep, Rep::I64 | Rep::TaggedRef))
                {
                    Rep::TaggedRef
                } else {
                    Rep::TruthyHandle
                },
            );
        }
        Instruction::BuildMap(pairs) => {
            let start = state.stack.len() - usize::from(*pairs) * 2;
            state.stack.truncate(start);
            state.stack.push(Rep::TruthyHandle);
        }
        Instruction::Jump(_) | Instruction::Return => {}
        _ => {
            // Preserve point-state shape through instructions that the MIR
            // lowering will subsequently reject with its more specific
            // eligibility diagnostic. Every nonterminal VM instruction has
            // a statically validated net stack effect.
            let effect = instruction.stack_effect().ok_or_else(|| {
                format!(
                    "whole-Wasm function {function} instruction {ip} has no representation transfer: {instruction}"
                )
            })?;
            let next = usize::try_from(state.stack.len() as i32 + effect).map_err(|_| {
                format!("whole-Wasm representation stack underflow in function {function} at {ip}")
            })?;
            state.stack.resize(next, Rep::Unknown);
        }
    }
    Ok(())
}

fn is_scalar_numeric_consumer(program: &Program, instruction: &Instruction) -> bool {
    instruction_target(program, instruction)
        .and_then(IntrinsicOp::from_symbol)
        .is_some_and(|op| {
            matches!(
                op,
                IntrinsicOp::Add
                    | IntrinsicOp::Subtract
                    | IntrinsicOp::Multiply
                    | IntrinsicOp::Divide
                    | IntrinsicOp::Remainder
                    | IntrinsicOp::Modulo
            )
        })
}

fn constant_rep(value: &Value) -> Rep {
    match value {
        Value::Number(_) => Rep::I64,
        Value::Bool(_) => Rep::Bool,
        Value::Nil => Rep::Nil,
        Value::String(_) => Rep::KeyRef,
        _ => Rep::TruthyHandle,
    }
}

fn target_rep(target: &str, arguments: &[Rep]) -> Rep {
    if let Some(op) = IntrinsicOp::from_symbol(target) {
        return match op {
        IntrinsicOp::Add
        | IntrinsicOp::Subtract
        | IntrinsicOp::Multiply
        | IntrinsicOp::Divide
        | IntrinsicOp::Remainder
        | IntrinsicOp::Modulo => {
            if arguments.iter().all(|rep| *rep == Rep::I64) {
                Rep::I64
            } else {
                Rep::Unknown
            }
        }
        IntrinsicOp::Equal
        | IntrinsicOp::Less
        | IntrinsicOp::LessOrEqual
        | IntrinsicOp::Greater
        | IntrinsicOp::GreaterOrEqual => Rep::Bool,
        };
    }
    match target {
        "std.native.Base/number?" => Rep::Bool,
        "hara.whole-wasm/map" | "hara.whole-wasm/vector" => Rep::TruthyHandle,
        "std.protocol.iassoc.IAssoc/assoc" | "std.protocol.ilookup.ILookup/lookup" => {
            Rep::TruthyHandle
        }
        "std.protocol.icount.ICount/count" => Rep::I64,
        "std.protocol.inth.INth/nth" => {
            if arguments.first() == Some(&Rep::TaggedRef) {
                Rep::TaggedRef
            } else {
                Rep::TruthyHandle
            }
        }
        _ => match target {
            "std.native.Arr/new" | "std.native.Arr/set" => Rep::ArrayRef,
            "std.native.Arr/get" => Rep::I64,
            "std.native.Obj/new" | "std.native.Obj/set" => Rep::ObjectRef,
            "std.native.Obj/get" => Rep::I64,
            _ => Rep::Unknown,
        },
    }
}

fn function_uses_tagged_collections(program: &Program, function: &FunctionPrototype) -> bool {
    function.arity == 1
        && [
            "std.native.Base/number?",
            "std.protocol.icount.ICount/count",
            "std.protocol.inth.INth/nth",
        ]
        .into_iter()
        .all(|required| {
            function
                .code
                .iter()
                .any(|instruction| instruction_target(program, instruction) == Some(required))
        })
}

fn target_name(program: &Program, target: u32) -> Result<&str, String> {
    match program.constants.get(target as usize) {
        Some(Value::String(name)) => Ok(name),
        _ => Err(format!("intrinsic target constant {target} is not a string")),
    }
}

fn instruction_target<'a>(program: &'a Program, instruction: &Instruction) -> Option<&'a str> {
    match instruction {
        Instruction::IntrinsicCall { target, .. } | Instruction::ProtocolCall { target, .. } => {
            target_name(program, *target).ok()
        }
        _ => None,
    }
}

fn instruction_intrinsic_op(program: &Program, instruction: &Instruction) -> Option<IntrinsicOp> {
    instruction_target(program, instruction).and_then(IntrinsicOp::from_symbol)
}

pub(crate) fn function_enables_tagged_vectors(program: &Program, function: FunctionId) -> bool {
    // Constant collections enter through the host handle table. Until the
    // whole-Wasm tier can recursively materialize those constants into its
    // tagged linear-memory layout, keep the entire call graph on the handle
    // representation. Mixing a host handle with TaggedRef is type-correct at
    // the Wasm boundary but interprets the handle number as a memory address.
    if program_uses_collection_constants(program) {
        return false;
    }
    let Some(current) = program.functions.get(usize::from(function)) else {
        return false;
    };
    function_uses_tagged_collections(program, current)
        || (current
            .code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::Call { .. }))
            && program
                .functions
                .iter()
                .any(|function| function_uses_tagged_collections(program, function)))
        || current.code.iter().any(|instruction| {
            let Instruction::CallStatic { prototype, .. } = instruction else {
                return false;
            };
            program
                .functions
                .get(usize::from(*prototype))
                .is_some_and(|function| function_uses_tagged_collections(program, function))
        })
}

fn program_uses_collection_constants(program: &Program) -> bool {
    program.constants.iter().any(|value| {
        matches!(
            value,
            Value::Vector(_)
                | Value::Tuple(_)
                | Value::List(_)
                | Value::Cons(_)
                | Value::Queue(_)
                | Value::Map(_)
                | Value::OrderedMap(_)
                | Value::SortedMap(_)
                | Value::Trie(_)
                | Value::Set(_)
                | Value::OrderedSet(_)
                | Value::SortedSet(_)
        )
    })
}

fn resolve_function(program: &Program, constant: u32) -> Option<FunctionId> {
    let Value::String(name) = program.constants.get(constant as usize)? else {
        return None;
    };
    program
        .functions
        .iter()
        .position(|function| {
            function.name.as_deref().is_some_and(|candidate| {
                candidate == name || candidate.rsplit('/').next() == name.rsplit('/').next()
            })
        })
        .and_then(|id| u16::try_from(id).ok())
}

fn successors(ip: usize, instruction: &Instruction, code_len: usize) -> Vec<usize> {
    match instruction {
        Instruction::Jump(target) => vec![*target as usize],
        Instruction::JumpIfFalse(target) => vec![ip + 1, *target as usize],
        Instruction::Return | Instruction::Throw | Instruction::Rethrow => Vec::new(),
        _ if ip + 1 < code_len => vec![ip + 1],
        _ => Vec::new(),
    }
}

fn join(existing: &mut RepState, incoming: &RepState) -> Result<bool, String> {
    if existing.locals.len() != incoming.locals.len()
        || existing.stack.len() != incoming.stack.len()
    {
        return Err("whole-Wasm representation state shape mismatch".into());
    }
    let mut changed = false;
    for (current, next) in existing
        .locals
        .iter_mut()
        .chain(existing.stack.iter_mut())
        .zip(incoming.locals.iter().chain(&incoming.stack))
    {
        let joined = if current == next {
            *current
        } else {
            Rep::Unknown
        };
        if *current != joined {
            *current = joined;
            changed = true;
        }
    }
    Ok(changed)
}
