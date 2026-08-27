//! Program validation: one abstract-interpretation pass over the code
//! vector before any execution. After validation the machine indexes
//! without re-checking, and malformed programs never reach a panic.

use std::collections::HashSet;

use super::error::ValidationError;
use super::opcode::Instruction;
use super::program::{
    FunctionPrototype, Program, MAX_CONSTANTS, MAX_INSTRUCTIONS, MAX_LOCALS, MAX_OPERAND_STACK,
};
use crate::core::Value;

/// Validates a whole program. See `notes/rust-bytecode-vm.md` §9 for the
/// rule list.
pub fn validate(program: &Program) -> Result<(), ValidationError> {
    if program.constants.len() > MAX_CONSTANTS {
        return Err(ValidationError::new(
            format!("constant pool exceeds limit of {MAX_CONSTANTS}"),
            None,
        ));
    }
    if program.functions.is_empty() {
        return Err(ValidationError::new("program has no functions", None));
    }
    if program.entry as usize >= program.functions.len() {
        return Err(ValidationError::new(
            "entry function index out of range",
            None,
        ));
    }
    let multiple = program.functions.len() > 1;
    for (index, function) in program.functions.iter().enumerate() {
        validate_declared_arity(program, index)
            .and_then(|_| validate_function(program, function))
            .map_err(|mut error| {
                if multiple {
                    error.message = format!("function {index}: {}", error.message);
                }
                error
            })?;
    }
    Ok(())
}

fn validate_declared_arity(program: &Program, index: usize) -> Result<(), ValidationError> {
    let function = &program.functions[index];
    let Some(crate::kernel::SchemaType::Function(arities)) = program.function_schema(index as u16)
    else {
        return Ok(());
    };
    if arities.iter().any(|schema| {
        schema.fixed.len() == function.arity as usize && schema.rest.is_some() == function.variadic
    }) {
        return Ok(());
    }
    Err(ValidationError::new(
        format!(
            "function schema for {} has no {}-argument arity{}",
            function.name.as_deref().unwrap_or("<anonymous>"),
            function.arity,
            if function.variadic {
                " with rest arguments"
            } else {
                ""
            }
        ),
        None,
    ))
}

fn validate_function(
    program: &Program,
    function: &FunctionPrototype,
) -> Result<(), ValidationError> {
    if function.source_map.len() != function.code.len() {
        return Err(ValidationError::new(
            "source map length does not match code length",
            None,
        ));
    }
    let heights = stack_heights(program, function)?;
    let computed = heights.iter().copied().max().unwrap_or(0);
    if computed != function.max_stack {
        return Err(ValidationError::new(
            format!(
                "declared max_stack {} disagrees with computed {computed}",
                function.max_stack
            ),
            None,
        ));
    }
    validate_handlers(function, &heights)?;
    Ok(())
}

/// Checks the static handler table: ranges, targets, slots, depth
/// declarations, pending-slot presence, and clean nesting. Stack heights
/// at handler targets are already covered by the analysis, which seeds
/// them with the height computed at each entry's `start`.
fn validate_handlers(function: &FunctionPrototype, heights: &[u16]) -> Result<(), ValidationError> {
    let code_len = function.code.len();
    for (index, entry) in function.handlers.iter().enumerate() {
        let (start, end) = (entry.start as usize, entry.end as usize);
        if start >= end || end > code_len {
            return Err(ValidationError::new(
                format!("try range [{start}, {end}) out of bounds or empty"),
                Some(entry.start),
            ));
        }
        if heights[start] != entry.depth {
            return Err(ValidationError::new(
                format!(
                    "handler depth {} disagrees with computed {}",
                    entry.depth, heights[start]
                ),
                Some(entry.start),
            ));
        }
        for catch in &entry.catches {
            if catch.target as usize >= code_len {
                return Err(ValidationError::new(
                    format!("catch target {} out of range", catch.target),
                    Some(entry.start),
                ));
            }
            if catch.binding >= function.local_count {
                return Err(ValidationError::new(
                    format!("catch binding slot {} out of range", catch.binding),
                    Some(entry.start),
                ));
            }
        }
        match (entry.finally, entry.pending_value, entry.pending_error) {
            (Some(finally), Some(value), Some(flag)) => {
                if finally as usize >= code_len {
                    return Err(ValidationError::new(
                        format!("finally target {finally} out of range"),
                        Some(entry.start),
                    ));
                }
                if value >= function.local_count || flag >= function.local_count {
                    return Err(ValidationError::new(
                        "pending slot out of range",
                        Some(entry.start),
                    ));
                }
            }
            (None, None, None) => {}
            _ => {
                return Err(ValidationError::new(
                    "pending slots must be present exactly when finally is present",
                    Some(entry.start),
                ))
            }
        }
        for other in &function.handlers[index + 1..] {
            let (s1, e1) = (entry.start, entry.end);
            let (s2, e2) = (other.start, other.end);
            let disjoint = e1 <= s2 || e2 <= s1;
            let nested = (s1 <= s2 && e2 <= e1) || (s2 <= s1 && e1 <= e2);
            if !disjoint && !nested {
                return Err(ValidationError::new(
                    "try ranges must not partially overlap",
                    Some(entry.start),
                ));
            }
        }
    }
    Ok(())
}

/// Computes the unique operand-stack height at every instruction while
/// checking indexes, slots, jump targets, reachability, and termination.
/// Shared by the validator and by the compiler, which uses it to fill in
/// `max_stack` for code it just emitted.
pub(crate) fn stack_heights(
    program: &Program,
    function: &FunctionPrototype,
) -> Result<Vec<u16>, ValidationError> {
    let code = &function.code;
    if code.is_empty() {
        return Err(ValidationError::new("function has no code", None));
    }
    if code.len() > MAX_INSTRUCTIONS {
        return Err(ValidationError::new(
            format!("code exceeds limit of {MAX_INSTRUCTIONS} instructions"),
            None,
        ));
    }
    if usize::from(function.local_count) > MAX_LOCALS {
        return Err(ValidationError::new("local count exceeds slot limit", None));
    }
    let mut heights: Vec<Option<u16>> = vec![None; code.len()];
    let mut worklist: Vec<(usize, u16)> = vec![(0, 0)];
    // Handler regions are reached by unwinding, not by ordinary control
    // flow; each is seeded with the height computed at its entry's start.
    let mut handler_starts: std::collections::HashMap<usize, Vec<usize>> =
        std::collections::HashMap::new();
    for (index, entry) in function.handlers.iter().enumerate() {
        handler_starts
            .entry(entry.start as usize)
            .or_default()
            .push(index);
    }
    while let Some((ip, height)) = worklist.pop() {
        if let Some(existing) = heights[ip] {
            if existing != height {
                return Err(ValidationError::new(
                    format!("inconsistent stack heights {existing} and {height} at join"),
                    Some(ip as u32),
                ));
            }
            continue;
        }
        heights[ip] = Some(height);
        if let Some(entries) = handler_starts.get(&ip) {
            for &index in entries {
                let entry = &function.handlers[index];
                for catch in &entry.catches {
                    if catch.target as usize >= code.len() {
                        return Err(ValidationError::new(
                            format!("catch target {} out of range", catch.target),
                            Some(entry.start),
                        ));
                    }
                    worklist.push((catch.target as usize, height));
                }
                if let Some(finally) = entry.finally {
                    if finally as usize >= code.len() {
                        return Err(ValidationError::new(
                            format!("finally target {finally} out of range"),
                            Some(entry.start),
                        ));
                    }
                    worklist.push((finally as usize, height));
                }
            }
        }
        let instruction = &code[ip];
        let at = Some(ip as u32);
        // Operand checks independent of control flow.
        match instruction {
            Instruction::Constant(index) if *index as usize >= program.constants.len() => {
                return Err(ValidationError::new(
                    format!("constant index {index} out of range"),
                    at,
                ));
            }
            Instruction::LoadLocal(slot) | Instruction::StoreLocal(slot)
                if *slot >= function.local_count =>
            {
                return Err(ValidationError::new(
                    format!("local slot {slot} out of range"),
                    at,
                ));
            }
            Instruction::IntrinsicCall { target, .. }
            | Instruction::ProtocolCall { target, .. }
            | Instruction::IntrinsicValue(target) => {
                string_constant(program, *target, at)?;
            }
            Instruction::DefProtocol(constant)
            | Instruction::ExtendType(constant)
            | Instruction::DefMulti(constant)
            | Instruction::DefMethod(constant) => {
                let Some(value) = program.constants.get(*constant as usize) else {
                    return Err(ValidationError::new(
                        format!("constant index {constant} out of range"),
                        at,
                    ));
                };
                let expected = match instruction {
                    Instruction::DefProtocol(_) => "defprotocol",
                    Instruction::ExtendType(_) => "extend-type",
                    Instruction::DefMulti(_) => "defmulti",
                    Instruction::DefMethod(_) => "defmethod",
                    _ => unreachable!("declaration instruction was matched"),
                };
                let valid = crate::core::value_to_form(value).is_ok_and(|form| {
                    matches!(
                        form,
                        crate::kernel::Form::List(items)
                            if matches!(items.first(), Some(crate::kernel::Form::Symbol(operator)) if operator == expected)
                    )
                });
                if !valid {
                    return Err(ValidationError::new(
                        format!("{expected} declaration constant {constant} is invalid"),
                        at,
                    ));
                }
            }
            Instruction::BuiltinValue(constant)
                if !matches!(
                    program.constants.get(*constant as usize),
                    Some(Value::String(_))
                ) =>
            {
                return Err(ValidationError::new(
                    format!("builtin name constant {constant} is invalid"),
                    at,
                ));
            }
            Instruction::NamespaceValue(constant)
                if !matches!(
                    program.constants.get(*constant as usize),
                    Some(Value::String(_))
                ) =>
            {
                return Err(ValidationError::new(
                    format!("namespace name constant {constant} is invalid"),
                    at,
                ));
            }
            Instruction::NamespaceOperation(constant) => {
                let Some(value) = program.constants.get(*constant as usize) else {
                    return Err(ValidationError::new(
                        format!("constant index {constant} out of range"),
                        at,
                    ));
                };
                let valid = crate::core::value_to_form(value).is_ok_and(|form| {
                    matches!(
                        crate::core::form_without_metadata(&form),
                        crate::kernel::Form::List(items)
                            if matches!(
                                items.first(),
                                Some(crate::kernel::Form::Symbol(operator))
                                    if matches!(operator.as_str(), "ns" | "ns+" | "require")
                            )
                    )
                });
                if !valid {
                    return Err(ValidationError::new(
                        format!("namespace-management constant {constant} is invalid"),
                        at,
                    ));
                }
            }
            Instruction::DynamicBind(constant) | Instruction::DynamicUnbind(constant)
                if !matches!(
                    program.constants.get(*constant as usize),
                    Some(Value::String(_))
                ) =>
            {
                return Err(ValidationError::new(
                    format!("binding name constant {constant} is invalid"),
                    at,
                ));
            }
            Instruction::Jump(target) | Instruction::JumpIfFalse(target)
                if *target as usize >= code.len() =>
            {
                return Err(ValidationError::new(
                    format!("jump target {target} out of range"),
                    at,
                ));
            }
            Instruction::Closure {
                prototype,
                captures,
            } => {
                let Some(target) = program.functions.get(usize::from(*prototype)) else {
                    return Err(ValidationError::new(
                        format!("closure prototype {prototype} out of range"),
                        at,
                    ));
                };
                if usize::from(*captures) != usize::from(target.capture_count) {
                    return Err(ValidationError::new(
                        format!(
                            "closure captures {captures} but prototype expects {}",
                            target.capture_count
                        ),
                        at,
                    ));
                }
            }
            Instruction::CallStatic { prototype, argc } => {
                let Some(target) = program.functions.get(usize::from(*prototype)) else {
                    return Err(ValidationError::new(
                        format!("callstatic target {prototype} out of range"),
                        at,
                    ));
                };
                let arity = usize::from(target.arity);
                let arity_ok = if target.variadic {
                    usize::from(*argc) >= arity
                } else {
                    usize::from(*argc) == arity
                };
                if !arity_ok {
                    return Err(ValidationError::new(
                        format!("callstatic argc {argc} but prototype expects {arity}"),
                        at,
                    ));
                }
                if target.capture_count != function.capture_count {
                    return Err(ValidationError::new(
                        "callstatic capture count differs from current function",
                        at,
                    ));
                }
            }
            Instruction::GetGlobal(index)
            | Instruction::SetGlobal(index)
            | Instruction::VarGlobal(index)
            | Instruction::MutableFieldGet(index)
            | Instruction::MutableFieldSet(index)
            | Instruction::DeclareGlobal(index) => {
                string_constant(program, *index, at)?;
            }
            Instruction::DefGlobal { name, metadata }
            | Instruction::DefMacro { name, metadata } => {
                string_constant(program, *name, at)?;
                if let Some(metadata) = metadata {
                    if usize::from(*metadata) >= program.var_metadata.len() {
                        return Err(ValidationError::new(
                            format!("var metadata index {metadata} out of range"),
                            at,
                        ));
                    }
                }
            }
            Instruction::DefStruct { name, fields } | Instruction::DefMutable { name, fields } => {
                string_constant(program, *name, at)?;
                let kind = if matches!(instruction, Instruction::DefMutable { .. }) {
                    "defmutable"
                } else {
                    "defstruct"
                };
                match program.constants.get(*fields as usize) {
                    Some(Value::Vector(fields)) => {
                        let mut names = HashSet::new();
                        for field in fields.iter() {
                            let named = crate::core::NamedField::from_value(field, kind)
                                .map_err(|message| ValidationError::new(message, at))?;
                            if !names.insert(named.name) {
                                return Err(ValidationError::new(
                                    format!("Duplicate {kind} field"),
                                    at,
                                ));
                            }
                        }
                    }
                    Some(_) => {
                        return Err(ValidationError::new(
                            format!("named fields constant {fields} is not a field vector"),
                            at,
                        ))
                    }
                    None => {
                        return Err(ValidationError::new(
                            format!("constant index {fields} out of range"),
                            at,
                        ))
                    }
                }
            }
            Instruction::MakeMultiArity { name, .. } => {
                string_constant(program, *name, at)?;
            }
            _ => {}
        }
        // Stack effects and successors.
        if let Instruction::Return = instruction {
            if height != 1 {
                return Err(ValidationError::new(
                    format!("return with stack height {height}, expected 1"),
                    at,
                ));
            }
            continue;
        }
        if matches!(instruction, Instruction::Throw | Instruction::Rethrow) {
            if height < 1 {
                return Err(ValidationError::new("stack underflow", at));
            }
            continue;
        }
        let effect = instruction
            .stack_effect()
            .expect("non-terminal instruction");
        let next = height as i32 + effect;
        if next < 0 {
            return Err(ValidationError::new("stack underflow", at));
        }
        if next as usize > MAX_OPERAND_STACK {
            return Err(ValidationError::new(
                format!("operand stack exceeds limit of {MAX_OPERAND_STACK}"),
                at,
            ));
        }
        let next = next as u16;
        match instruction {
            Instruction::Jump(target) => worklist.push((*target as usize, next)),
            Instruction::JumpIfFalse(target) => {
                worklist.push((*target as usize, next));
                push_fallthrough(code, ip, next, &mut worklist)?;
            }
            _ => push_fallthrough(code, ip, next, &mut worklist)?,
        }
    }
    let mut result = Vec::with_capacity(code.len());
    for (ip, height) in heights.into_iter().enumerate() {
        match height {
            Some(height) => result.push(height),
            None => {
                return Err(ValidationError::new(
                    "unreachable instruction",
                    Some(ip as u32),
                ))
            }
        }
    }
    Ok(result)
}

fn push_fallthrough(
    code: &[Instruction],
    ip: usize,
    height: u16,
    worklist: &mut Vec<(usize, u16)>,
) -> Result<(), ValidationError> {
    if ip + 1 == code.len() {
        return Err(ValidationError::new(
            "missing return: control falls off the end of the function",
            Some(ip as u32),
        ));
    }
    debug_assert!(code[ip].falls_through());
    worklist.push((ip + 1, height));
    Ok(())
}

/// Global-instruction name operands must index a string constant.
fn string_constant(program: &Program, index: u32, at: Option<u32>) -> Result<(), ValidationError> {
    match program.constants.get(index as usize) {
        Some(Value::String(_)) => Ok(()),
        Some(_) => Err(ValidationError::new(
            format!("global name constant {index} is not a string"),
            at,
        )),
        None => Err(ValidationError::new(
            format!("constant index {index} out of range"),
            at,
        )),
    }
}
