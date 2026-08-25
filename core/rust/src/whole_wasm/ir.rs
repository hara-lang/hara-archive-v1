use std::collections::{BTreeMap, BTreeSet};

use crate::core::{IntrinsicOp, Value};
use crate::vm::{FunctionId, Instruction, Program};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rep {
    I64,
    Bool,
    Nil,
    ArrayRef,
    ObjectRef,
    KeyRef,
    TaggedRef,
    TruthyHandle,
    FunctionRef(FunctionId),
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Op<V> {
    Constant {
        destination: V,
        value: i64,
        rep: Rep,
    },
    ConstantHandle {
        destination: V,
        constant: u32,
    },
    BoxI64 {
        destination: V,
        source: V,
    },
    UnboxI64 {
        destination: V,
        source: V,
    },
    Move {
        destination: V,
        source: V,
    },
    Binary {
        destination: V,
        left: V,
        right: V,
        op: IntrinsicOp,
    },
    BinaryConstant {
        destination: V,
        left: V,
        right: i64,
        op: IntrinsicOp,
    },
    ArrayNew {
        destination: V,
        values: Vec<V>,
    },
    ArrayGetI64 {
        destination: V,
        array: V,
        index: V,
    },
    ArrayGetI64Constant {
        destination: V,
        array: V,
        index: i64,
    },
    ArraySetI64 {
        destination: V,
        array: V,
        index: V,
        value: V,
    },
    ObjectNew {
        destination: V,
        entries: Vec<(V, V)>,
    },
    ObjectGetI64 {
        destination: V,
        object: V,
        key: V,
    },
    ObjectSetI64 {
        destination: V,
        object: V,
        key: V,
        value: V,
    },
    BuildVector {
        destination: V,
        values: Vec<V>,
    },
    NativeVector {
        destination: V,
        values: Vec<(V, Rep)>,
    },
    BuildMap {
        destination: V,
        entries: Vec<(V, V)>,
    },
    BuildMapI64Pair {
        destination: V,
        key: V,
        value: V,
    },
    Assoc {
        destination: V,
        collection: V,
        key: V,
        value: V,
    },
    AssocMapI64Pair {
        destination: V,
        collection: V,
        outer_key: V,
        inner_key: V,
        value: V,
    },
    Get {
        destination: V,
        collection: V,
        key: V,
    },
    GetI64 {
        destination: V,
        collection: V,
        key: V,
    },
    GetPathI64Constants {
        destination: V,
        collection: V,
        first_key: u32,
        second_key: u32,
    },
    IsNumber {
        destination: V,
        value: V,
    },
    TaggedIsNumber {
        destination: V,
        value: V,
    },
    Count {
        destination: V,
        collection: V,
    },
    TaggedCount {
        destination: V,
        collection: V,
    },
    Nth {
        destination: V,
        collection: V,
        index: V,
    },
    TaggedNth {
        destination: V,
        collection: V,
        index: V,
    },
    TaggedUnboxI64 {
        destination: V,
        source: V,
    },
    CallStatic {
        destination: V,
        function: FunctionId,
        arguments: Vec<V>,
    },
}

pub type MirOp = Op<u16>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MirTerminator {
    Goto(u16),
    BranchZero {
        condition: u16,
        rep: Rep,
        zero: u16,
        nonzero: u16,
    },
    Return(u16),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirBlock {
    pub id: u16,
    pub start: u32,
    pub operations: Vec<MirOp>,
    pub terminator: MirTerminator,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirFunction {
    pub id: FunctionId,
    pub name: Option<String>,
    pub arity: u16,
    pub local_count: u16,
    pub stack_count: u16,
    pub blocks: Vec<MirBlock>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirProgram {
    pub entry: FunctionId,
    pub functions: Vec<MirFunction>,
}

/// Converts every eligible bytecode function into whole-function block IR.
/// This first representation slice is deliberately strict: it accepts only
/// synchronous, capture-free, fixed-arity scalar functions and fails the
/// complete compilation if a reachable operation has no exact lowering.
pub(crate) fn lower_slot_program(program: &Program) -> Result<MirProgram, String> {
    crate::vm::validate(program).map_err(|error| error.to_string())?;
    let functions = program
        .functions
        .iter()
        .enumerate()
        .map(|(id, function)| lower_function(program, id as FunctionId, function))
        .collect::<Result<Vec<_>, _>>()?;
    let mir = MirProgram {
        entry: program.entry,
        functions,
    };
    verify(&mir)?;
    Ok(mir)
}

pub(crate) fn lower_function(
    program: &Program,
    id: FunctionId,
    function: &crate::vm::FunctionPrototype,
) -> Result<MirFunction, String> {
    if function.async_function || function.variadic || function.capture_count != 0 {
        return Err(unsupported(id, 0, "function shape"));
    }
    let heights =
        crate::vm::validate::stack_heights(program, function).map_err(|error| error.to_string())?;
    let representations = super::reps::analyze_function(program, id, function)?;
    let mut leaders = BTreeSet::from([0usize]);
    for (ip, instruction) in function.code.iter().enumerate() {
        if let Some(target) = instruction.jump_target() {
            leaders.insert(target as usize);
        }
        if matches!(
            instruction,
            Instruction::Jump(_)
                | Instruction::JumpIfFalse(_)
                | Instruction::Return
                | Instruction::Throw
                | Instruction::Rethrow
        ) && ip + 1 < function.code.len()
        {
            leaders.insert(ip + 1);
        }
    }
    let starts = leaders.into_iter().collect::<Vec<_>>();
    let ids = starts
        .iter()
        .enumerate()
        .map(|(id, start)| (*start, id as u16))
        .collect::<BTreeMap<_, _>>();
    let stack_base = function.local_count;
    let scalar_scratch = function
        .local_count
        .checked_add(function.max_stack)
        .ok_or_else(|| "whole-Wasm scalar scratch slot overflow".to_string())?;
    let mut blocks = Vec::with_capacity(starts.len());
    for (block_index, start) in starts.iter().copied().enumerate() {
        let end = starts
            .get(block_index + 1)
            .copied()
            .unwrap_or(function.code.len());
        let mut operations = Vec::new();
        let mut terminator = None;
        for ip in start..end {
            let instruction = &function.code[ip];
            let height = usize::from(heights[ip]);
            let stack = |offset: usize| -> Result<u16, String> {
                stack_base
                    .checked_add(u16::try_from(offset).map_err(|_| "operand stack exceeds u16")?)
                    .ok_or_else(|| "whole-Wasm slot index overflow".to_string())
            };
            match instruction {
                Instruction::Constant(index) => {
                    let (value, rep) = match program.constants.get(*index as usize) {
                        Some(Value::String(_)) => (i64::from(*index) + 1, Rep::KeyRef),
                        Some(Value::Nil) => (0, Rep::Nil),
                        Some(value) => {
                            if let Some(value) = scalar_constant(Some(value)) {
                                value
                            } else {
                                operations.push(MirOp::ConstantHandle {
                                    destination: stack(height)?,
                                    constant: *index,
                                });
                                continue;
                            }
                        }
                        None => return Err(unsupported(id, ip, "missing constant")),
                    };
                    operations.push(MirOp::Constant {
                        destination: stack(height)?,
                        value,
                        rep,
                    });
                }
                Instruction::True => operations.push(MirOp::Constant {
                    destination: stack(height)?,
                    value: 1,
                    rep: Rep::Bool,
                }),
                Instruction::False => operations.push(MirOp::Constant {
                    destination: stack(height)?,
                    value: 0,
                    rep: Rep::Bool,
                }),
                Instruction::Nil => operations.push(MirOp::Constant {
                    destination: stack(height)?,
                    value: 0,
                    rep: Rep::Nil,
                }),
                Instruction::Closure {
                    prototype,
                    captures: 0,
                } => operations.push(MirOp::Constant {
                    destination: stack(height)?,
                    value: i64::from(*prototype),
                    rep: Rep::FunctionRef(*prototype),
                }),
                Instruction::DefGlobal { .. } => {}
                Instruction::VarGlobal(index) | Instruction::GetGlobal(index) => {
                    let prototype = function_for_global(program, *index)
                        .ok_or_else(|| unsupported(id, ip, "global is not a compiled function"))?;
                    operations.push(MirOp::Constant {
                        destination: stack(height)?,
                        value: i64::from(prototype),
                        rep: Rep::FunctionRef(prototype),
                    });
                }
                Instruction::LoadLocal(source) => operations.push(MirOp::Move {
                    destination: stack(height)?,
                    source: *source,
                }),
                Instruction::StoreLocal(destination) => operations.push(MirOp::Move {
                    destination: *destination,
                    source: stack(height - 1)?,
                }),
                Instruction::Dup => operations.push(MirOp::Move {
                    destination: stack(height)?,
                    source: stack(height - 1)?,
                }),
                Instruction::Pop => {}
                Instruction::IntrinsicCall { target, argc: 2 }
                    if intrinsic_op(program, *target).is_some_and(scalar_binary) => {
                    let op = intrinsic_op(program, *target).expect("guarded intrinsic operator");
                    for offset in 0..2 {
                        if matches!(
                            representations[ip].stack[height - 2 + offset],
                            Rep::TruthyHandle
                        ) {
                            operations.push(MirOp::UnboxI64 {
                                destination: stack(height - 2 + offset)?,
                                source: stack(height - 2 + offset)?,
                            });
                        }
                    }
                    operations.push(MirOp::Binary {
                        destination: stack(height - 2)?,
                        left: stack(height - 2)?,
                        right: stack(height - 1)?,
                        op,
                    });
                }
                Instruction::IntrinsicCall { target, argc }
                    if *argc > 2
                        && intrinsic_op(program, *target).is_some_and(scalar_arithmetic) => {
                    let op = intrinsic_op(program, *target).expect("guarded intrinsic operator");
                    let base = height - usize::from(*argc);
                    for offset in 1..usize::from(*argc) {
                        operations.push(MirOp::Binary {
                            destination: stack(base)?,
                            left: stack(base)?,
                            right: stack(base + offset)?,
                            op,
                        });
                    }
                }
                Instruction::IntrinsicCall { target, argc }
                    if target_is(program, *target, "std.native.Arr/new") => {
                    let base = height - usize::from(*argc);
                    let values = (0..usize::from(*argc))
                        .map(|index| stack(base + index))
                        .collect::<Result<Vec<_>, _>>()?;
                    if values
                        .iter()
                        .enumerate()
                        .any(|(index, _)| representations[ip].stack[base + index] != Rep::I64)
                    {
                        return Err(unsupported(id, ip, "array constructor requires i64 values"));
                    }
                    operations.push(MirOp::ArrayNew {
                        destination: stack(base)?,
                        values,
                    });
                }
                Instruction::IntrinsicCall { target, argc: 2 }
                    if target_is(program, *target, "std.native.Arr/get") => operations.push(MirOp::ArrayGetI64 {
                    destination: stack(height - 2)?,
                    array: stack(height - 2)?,
                    index: stack(height - 1)?,
                }),
                Instruction::IntrinsicCall { target, argc: 3 }
                    if target_is(program, *target, "std.native.Arr/set") => operations.push(MirOp::ArraySetI64 {
                    destination: stack(height - 3)?,
                    array: stack(height - 3)?,
                    index: stack(height - 2)?,
                    value: stack(height - 1)?,
                }),
                Instruction::IntrinsicCall { target, argc }
                    if target_is(program, *target, "std.native.Obj/new") => {
                    if *argc % 2 != 0 {
                        return Err(unsupported(id, ip, "object constructor pairs"));
                    }
                    let base = height - usize::from(*argc);
                    let mut entries = Vec::with_capacity(usize::from(*argc) / 2);
                    for offset in (0..usize::from(*argc)).step_by(2) {
                        if representations[ip].stack[base + offset] != Rep::KeyRef
                            || representations[ip].stack[base + offset + 1] != Rep::I64
                        {
                            return Err(unsupported(
                                id,
                                ip,
                                "object constructor requires string/i64 pairs",
                            ));
                        }
                        entries.push((stack(base + offset)?, stack(base + offset + 1)?));
                    }
                    operations.push(MirOp::ObjectNew {
                        destination: stack(base)?,
                        entries,
                    });
                }
                Instruction::IntrinsicCall { target, argc: 2 }
                    if target_is(program, *target, "std.native.Obj/get") => operations.push(MirOp::ObjectGetI64 {
                    destination: stack(height - 2)?,
                    object: stack(height - 2)?,
                    key: stack(height - 1)?,
                }),
                Instruction::IntrinsicCall { target, argc: 3 }
                    if target_is(program, *target, "std.native.Obj/set") => operations.push(MirOp::ObjectSetI64 {
                    destination: stack(height - 3)?,
                    object: stack(height - 3)?,
                    key: stack(height - 2)?,
                    value: stack(height - 1)?,
                }),
                Instruction::BuildVector(count) => {
                    let base = height - usize::from(*count);
                    if super::reps::function_enables_tagged_vectors(program, id)
                        && representations[ip].stack[base..height]
                            .iter()
                            .all(|rep| matches!(rep, Rep::I64 | Rep::TaggedRef))
                    {
                        operations.push(MirOp::NativeVector {
                            destination: stack(base)?,
                            values: (0..usize::from(*count))
                                .map(|offset| {
                                    Ok((
                                        stack(base + offset)?,
                                        representations[ip].stack[base + offset],
                                    ))
                                })
                                .collect::<Result<Vec<_>, String>>()?,
                        });
                        continue;
                    }
                    if representations[ip].stack[base..height].contains(&Rep::TaggedRef) {
                        return Err(unsupported(id, ip, "tagged vector escapes native storage"));
                    }
                    let values = (0..usize::from(*count))
                        .map(|offset| {
                            let slot = stack(base + offset)?;
                            if representations[ip].stack[base + offset] == Rep::I64 {
                                operations.push(MirOp::BoxI64 {
                                    destination: slot,
                                    source: slot,
                                });
                            }
                            Ok(slot)
                        })
                        .collect::<Result<Vec<_>, String>>()?;
                    operations.push(MirOp::BuildVector {
                        destination: stack(base)?,
                        values,
                    });
                }
                Instruction::BuildMap(pairs) => {
                    let count = usize::from(*pairs) * 2;
                    let base = height - count;
                    if representations[ip].stack[base..height].contains(&Rep::TaggedRef) {
                        return Err(unsupported(
                            id,
                            ip,
                            "tagged value escapes into persistent map",
                        ));
                    }
                    if *pairs == 1
                        && representations[ip].stack[base] != Rep::I64
                        && representations[ip].stack[base + 1] == Rep::I64
                    {
                        operations.push(MirOp::BuildMapI64Pair {
                            destination: stack(base)?,
                            key: stack(base)?,
                            value: stack(base + 1)?,
                        });
                        continue;
                    }
                    let mut entries = Vec::with_capacity(usize::from(*pairs));
                    for offset in (0..count).step_by(2) {
                        let key = stack(base + offset)?;
                        let value = stack(base + offset + 1)?;
                        if representations[ip].stack[base + offset] == Rep::I64 {
                            operations.push(MirOp::BoxI64 {
                                destination: key,
                                source: key,
                            });
                        }
                        if representations[ip].stack[base + offset + 1] == Rep::I64 {
                            operations.push(MirOp::BoxI64 {
                                destination: value,
                                source: value,
                            });
                        }
                        entries.push((key, value));
                    }
                    operations.push(MirOp::BuildMap {
                        destination: stack(base)?,
                        entries,
                    });
                }
                Instruction::ProtocolCall { target, argc: 3 }
                    if declared_target_is(program, *target, "std.protocol.iassoc.IAssoc/assoc") => {
                    let base = height - 3;
                    if representations[ip].stack[base..height].contains(&Rep::TaggedRef) {
                        return Err(unsupported(id, ip, "tagged value escapes through assoc"));
                    }
                    for offset in 1..3 {
                        if representations[ip].stack[base + offset] == Rep::I64 {
                            let slot = stack(base + offset)?;
                            operations.push(MirOp::BoxI64 {
                                destination: slot,
                                source: slot,
                            });
                        }
                    }
                    operations.push(MirOp::Assoc {
                        destination: stack(base)?,
                        collection: stack(base)?,
                        key: stack(base + 1)?,
                        value: stack(base + 2)?,
                    });
                }
                Instruction::ProtocolCall { target, argc: 2 }
                    if declared_target_is(program, *target, "std.protocol.ilookup.ILookup/lookup") => {
                    let numeric_consumer =
                        function.code.get(ip + 1).is_some_and(|next| match next {
                            Instruction::IntrinsicCall { target, .. } => {
                                intrinsic_op(program, *target).is_some_and(scalar_arithmetic)
                            }
                            _ => false,
                        });
                    if numeric_consumer {
                        operations.push(MirOp::GetI64 {
                            destination: stack(height - 2)?,
                            collection: stack(height - 2)?,
                            key: stack(height - 1)?,
                        });
                    } else {
                        operations.push(MirOp::Get {
                            destination: stack(height - 2)?,
                            collection: stack(height - 2)?,
                            key: stack(height - 1)?,
                        });
                    }
                }
                Instruction::IntrinsicCall { target, argc: 1 }
                    if declared_target_is(program, *target, "std.native.Base/number?") => {
                    let operation = if representations[ip].stack[height - 1] == Rep::TaggedRef {
                        MirOp::TaggedIsNumber {
                            destination: stack(height - 1)?,
                            value: stack(height - 1)?,
                        }
                    } else {
                        MirOp::IsNumber {
                            destination: stack(height - 1)?,
                            value: stack(height - 1)?,
                        }
                    };
                    operations.push(operation);
                }
                Instruction::ProtocolCall { target, argc: 1 }
                    if declared_target_is(program, *target, "std.protocol.icount.ICount/count") => {
                    let operation = if representations[ip].stack[height - 1] == Rep::TaggedRef {
                        MirOp::TaggedCount {
                            destination: stack(height - 1)?,
                            collection: stack(height - 1)?,
                        }
                    } else {
                        MirOp::Count {
                            destination: stack(height - 1)?,
                            collection: stack(height - 1)?,
                        }
                    };
                    operations.push(operation);
                }
                Instruction::ProtocolCall { target, argc: 2 }
                    if declared_target_is(program, *target, "std.protocol.inth.INth/nth") => {
                    let operation = if representations[ip].stack[height - 2] == Rep::TaggedRef {
                        MirOp::TaggedNth {
                            destination: stack(height - 2)?,
                            collection: stack(height - 2)?,
                            index: stack(height - 1)?,
                        }
                    } else {
                        MirOp::Nth {
                            destination: stack(height - 2)?,
                            collection: stack(height - 2)?,
                            index: stack(height - 1)?,
                        }
                    };
                    operations.push(operation);
                }
                Instruction::CallStatic { prototype, argc } => {
                    super::call_boundary::lower_static(
                        &mut operations,
                        program,
                        &representations[ip].stack,
                        stack_base,
                        height,
                        *prototype,
                        *argc,
                        id,
                        ip,
                    )?;
                }
                Instruction::Call { argc } => {
                    super::call_boundary::lower_dynamic(
                        &mut operations,
                        program,
                        &representations[ip].stack,
                        stack_base,
                        height,
                        *argc,
                        id,
                        ip,
                    )?;
                }
                Instruction::Jump(target) => {
                    let target_rep = representations[*target as usize].stack.last().copied();
                    if representations[ip].stack.last() == Some(&Rep::TruthyHandle)
                        && target_rep == Some(Rep::Unknown)
                        && height != 0
                    {
                        operations.push(MirOp::UnboxI64 {
                            destination: stack(height - 1)?,
                            source: stack(height - 1)?,
                        });
                    }
                    if representations[ip].stack.last() == Some(&Rep::TaggedRef)
                        && target_rep == Some(Rep::Unknown)
                        && height != 0
                    {
                        operations.push(MirOp::TaggedUnboxI64 {
                            destination: stack(height - 1)?,
                            source: stack(height - 1)?,
                        });
                    }
                    terminator = Some(MirTerminator::Goto(block_id(&ids, *target)?));
                }
                Instruction::JumpIfFalse(target) => {
                    let rep =
                        representations[ip].stack.last().copied().ok_or_else(|| {
                            unsupported(id, ip, "condition has no representation")
                        })?;
                    if rep == Rep::Unknown {
                        return Err(unsupported(id, ip, "unproven dynamic truthiness"));
                    }
                    let fallthrough = u32::try_from(ip + 1)
                        .map_err(|_| "whole-Wasm instruction index overflow")?;
                    terminator = Some(MirTerminator::BranchZero {
                        condition: stack(height - 1)?,
                        rep,
                        zero: block_id(&ids, *target)?,
                        nonzero: block_id(&ids, fallthrough)?,
                    });
                }
                Instruction::Return => {
                    let value = super::call_boundary::lower_return(
                        &mut operations,
                        program,
                        &representations[ip].stack,
                        stack_base,
                        height,
                        id,
                        ip,
                    )?;
                    terminator = Some(MirTerminator::Return(value));
                }
                other => return Err(unsupported(id, ip, &other.to_string())),
            }
            if terminator.is_some() {
                break;
            }
        }
        let terminator = terminator.unwrap_or_else(|| {
            MirTerminator::Goto(ids.get(&end).copied().expect("validated fallthrough block"))
        });
        blocks.push(MirBlock {
            id: block_index as u16,
            start: start as u32,
            operations: forward_known_nested_fields(
                fuse_constant_get_paths(operations),
                scalar_scratch,
            ),
            terminator,
        });
    }
    virtualize_unobserved_loop_assocs(&mut blocks, function.local_count);
    Ok(MirFunction {
        id,
        name: function.name.clone(),
        arity: function.arity,
        local_count: function.local_count,
        stack_count: function
            .max_stack
            .checked_add(1)
            .ok_or_else(|| "whole-Wasm scalar scratch count overflow".to_string())?,
        blocks,
    })
}

/// Replaces a persistent assoc with a carrier move when the loop-carried
/// collection has no observable uses. Field reads must already have been
/// scalar-forwarded, and every result alias must flow only back to the same
/// local. The original immutable value remains available for deoptimization;
/// no persistent node is allocated on the optimized loop edge.
fn virtualize_unobserved_loop_assocs(blocks: &mut [MirBlock], local_count: u16) {
    let mut candidates = Vec::new();
    for (block_index, block) in blocks.iter().enumerate() {
        for (operation_index, operation) in block.operations.iter().enumerate() {
            let MirOp::AssocMapI64Pair {
                destination,
                collection,
                ..
            } = operation
            else {
                continue;
            };
            let Some(local) = originating_local(
                &block.operations[..operation_index],
                *collection,
                local_count,
            ) else {
                continue;
            };
            if operation_source_count(blocks, local) != 1
                || terminators_read_slot(blocks, local)
                || !assoc_result_returns_only_to_local(
                    &block.operations[operation_index + 1..],
                    *destination,
                    local,
                )
            {
                continue;
            }
            candidates.push((block_index, operation_index, *destination, *collection));
        }
    }
    for (block, operation, destination, collection) in candidates {
        blocks[block].operations[operation] = MirOp::Move {
            destination,
            source: collection,
        };
    }
}

fn originating_local(operations: &[MirOp], slot: u16, local_count: u16) -> Option<u16> {
    for operation in operations.iter().rev() {
        if operation_destination(operation) != Some(slot) {
            continue;
        }
        return match operation {
            MirOp::Move { source, .. } if *source < local_count => Some(*source),
            _ => None,
        };
    }
    None
}

fn operation_source_count(blocks: &[MirBlock], slot: u16) -> usize {
    blocks
        .iter()
        .flat_map(|block| &block.operations)
        .map(|operation| {
            operation_sources(operation)
                .filter(|source| *source == slot)
                .count()
        })
        .sum()
}

fn terminators_read_slot(blocks: &[MirBlock], slot: u16) -> bool {
    blocks.iter().any(|block| match block.terminator {
        MirTerminator::Goto(_) => false,
        MirTerminator::BranchZero { condition, .. } => condition == slot,
        MirTerminator::Return(value) => value == slot,
    })
}

fn assoc_result_returns_only_to_local(operations: &[MirOp], result: u16, local: u16) -> bool {
    let mut aliases = BTreeSet::from([result]);
    for operation in operations {
        if let MirOp::Move {
            destination,
            source,
        } = operation
        {
            let propagates = aliases.contains(source);
            aliases.remove(destination);
            if propagates {
                aliases.insert(*destination);
            }
            continue;
        }
        if operation_sources(operation).any(|source| aliases.contains(&source)) {
            return false;
        }
        if let Some(destination) = operation_destination(operation) {
            aliases.remove(&destination);
        }
    }
    aliases.contains(&local)
}

fn operation_destination(operation: &MirOp) -> Option<u16> {
    Some(match operation {
        MirOp::Constant { destination, .. }
        | MirOp::ConstantHandle { destination, .. }
        | MirOp::BoxI64 { destination, .. }
        | MirOp::UnboxI64 { destination, .. }
        | MirOp::Move { destination, .. }
        | MirOp::Binary { destination, .. }
        | MirOp::BinaryConstant { destination, .. }
        | MirOp::ArrayNew { destination, .. }
        | MirOp::ArrayGetI64 { destination, .. }
        | MirOp::ArrayGetI64Constant { destination, .. }
        | MirOp::ArraySetI64 { destination, .. }
        | MirOp::ObjectNew { destination, .. }
        | MirOp::ObjectGetI64 { destination, .. }
        | MirOp::ObjectSetI64 { destination, .. }
        | MirOp::BuildVector { destination, .. }
        | MirOp::NativeVector { destination, .. }
        | MirOp::BuildMap { destination, .. }
        | MirOp::BuildMapI64Pair { destination, .. }
        | MirOp::Assoc { destination, .. }
        | MirOp::AssocMapI64Pair { destination, .. }
        | MirOp::Get { destination, .. }
        | MirOp::GetI64 { destination, .. }
        | MirOp::GetPathI64Constants { destination, .. }
        | MirOp::IsNumber { destination, .. }
        | MirOp::TaggedIsNumber { destination, .. }
        | MirOp::Count { destination, .. }
        | MirOp::TaggedCount { destination, .. }
        | MirOp::Nth { destination, .. }
        | MirOp::TaggedNth { destination, .. }
        | MirOp::TaggedUnboxI64 { destination, .. }
        | MirOp::CallStatic { destination, .. } => *destination,
    })
}

fn operation_sources(operation: &MirOp) -> impl Iterator<Item = u16> {
    let mut sources = Vec::new();
    match operation {
        MirOp::Constant { .. } | MirOp::ConstantHandle { .. } => {}
        MirOp::BoxI64 { source, .. }
        | MirOp::UnboxI64 { source, .. }
        | MirOp::Move { source, .. }
        | MirOp::TaggedUnboxI64 { source, .. } => sources.push(*source),
        MirOp::Binary { left, right, .. } => sources.extend([*left, *right]),
        MirOp::BinaryConstant { left, .. } => sources.push(*left),
        MirOp::ArrayNew { values, .. } | MirOp::BuildVector { values, .. } => {
            sources.extend(values.iter().copied())
        }
        MirOp::NativeVector { values, .. } => sources.extend(values.iter().map(|(slot, _)| *slot)),
        MirOp::ArrayGetI64 { array, index, .. } => sources.extend([*array, *index]),
        MirOp::ArrayGetI64Constant { array, .. } => sources.push(*array),
        MirOp::ArraySetI64 {
            array,
            index,
            value,
            ..
        } => sources.extend([*array, *index, *value]),
        MirOp::ObjectNew { entries, .. } | MirOp::BuildMap { entries, .. } => {
            sources.extend(entries.iter().flat_map(|(key, value)| [*key, *value]))
        }
        MirOp::ObjectGetI64 { object, key, .. } => sources.extend([*object, *key]),
        MirOp::ObjectSetI64 {
            object, key, value, ..
        } => sources.extend([*object, *key, *value]),
        MirOp::BuildMapI64Pair { key, value, .. } => sources.extend([*key, *value]),
        MirOp::Assoc {
            collection,
            key,
            value,
            ..
        } => sources.extend([*collection, *key, *value]),
        MirOp::AssocMapI64Pair {
            collection,
            outer_key,
            inner_key,
            value,
            ..
        } => sources.extend([*collection, *outer_key, *inner_key, *value]),
        MirOp::Get {
            collection, key, ..
        }
        | MirOp::GetI64 {
            collection, key, ..
        } => sources.extend([*collection, *key]),
        MirOp::GetPathI64Constants { collection, .. }
        | MirOp::Count { collection, .. }
        | MirOp::TaggedCount { collection, .. } => sources.push(*collection),
        MirOp::IsNumber { value, .. } | MirOp::TaggedIsNumber { value, .. } => sources.push(*value),
        MirOp::Nth {
            collection, index, ..
        }
        | MirOp::TaggedNth {
            collection, index, ..
        } => sources.extend([*collection, *index]),
        MirOp::CallStatic { arguments, .. } => sources.extend(arguments.iter().copied()),
    }
    sources.into_iter()
}

/// Collapses `(get (get value :outer) :inner)` when its final result is
/// numeric. Both keyword constants are resolved by the host import, avoiding
/// two constant handles, an intermediate value handle, and one host crossing.
fn fuse_constant_get_paths(operations: Vec<MirOp>) -> Vec<MirOp> {
    let mut fused = Vec::with_capacity(operations.len());
    let mut index = 0;
    while index < operations.len() {
        if let [MirOp::BuildMapI64Pair {
            destination: map,
            key: inner_key,
            value,
        }, MirOp::Assoc {
            destination,
            collection,
            key: outer_key,
            value: assoc_value,
        }, ..] = &operations[index..]
        {
            if map == assoc_value {
                fused.push(MirOp::AssocMapI64Pair {
                    destination: *destination,
                    collection: *collection,
                    outer_key: *outer_key,
                    inner_key: *inner_key,
                    value: *value,
                });
                index += 2;
                continue;
            }
        }
        if let [MirOp::ConstantHandle {
            destination: first_key_slot,
            constant: first_key,
        }, MirOp::Get {
            destination: intermediate,
            collection,
            key: get_first_key,
        }, MirOp::ConstantHandle {
            destination: second_key_slot,
            constant: second_key,
        }, MirOp::GetI64 {
            destination,
            collection: get_intermediate,
            key: get_second_key,
        }, ..] = &operations[index..]
        {
            if first_key_slot == get_first_key
                && intermediate == get_intermediate
                && second_key_slot == get_second_key
            {
                fused.push(MirOp::GetPathI64Constants {
                    destination: *destination,
                    collection: *collection,
                    first_key: *first_key,
                    second_key: *second_key,
                });
                index += 4;
                continue;
            }
        }
        fused.push(operations[index].clone());
        index += 1;
    }
    fused
}

/// Gives stack-machine values stable identities within one CFG block. Known
/// nested scalar writes are copied to a compiler-reserved slot and matching
/// reads are forwarded without observing the materialized collection.
fn forward_known_nested_fields(operations: Vec<MirOp>, scratch: u16) -> Vec<MirOp> {
    let mut constants = BTreeMap::<u16, u32>::new();
    let mut nested = BTreeMap::<u16, (u32, u32, u64)>::new();
    let mut generation = 0u64;
    let mut output = Vec::with_capacity(operations.len());
    for operation in operations {
        match operation {
            MirOp::ConstantHandle {
                destination,
                constant,
            } => {
                constants.insert(destination, constant);
                nested.remove(&destination);
                output.push(MirOp::ConstantHandle {
                    destination,
                    constant,
                });
            }
            MirOp::Move {
                destination,
                source,
            } => {
                let constant = constants.get(&source).copied();
                let known = nested.get(&source).copied();
                constants.remove(&destination);
                nested.remove(&destination);
                if let Some(constant) = constant {
                    constants.insert(destination, constant);
                }
                if let Some(known) = known {
                    nested.insert(destination, known);
                }
                output.push(MirOp::Move {
                    destination,
                    source,
                });
            }
            assoc @ MirOp::AssocMapI64Pair {
                destination,
                outer_key,
                inner_key,
                value,
                ..
            } => {
                let keys = (
                    constants.get(&outer_key).copied(),
                    constants.get(&inner_key).copied(),
                );
                constants.remove(&destination);
                nested.remove(&destination);
                if let (Some(outer), Some(inner)) = keys {
                    generation = generation.wrapping_add(1);
                    output.push(MirOp::Move {
                        destination: scratch,
                        source: value,
                    });
                    nested.insert(destination, (outer, inner, generation));
                }
                output.push(assoc);
            }
            MirOp::GetPathI64Constants {
                destination,
                collection,
                first_key,
                second_key,
            } => {
                let known = nested.get(&collection).copied();
                constants.remove(&destination);
                nested.remove(&destination);
                if matches!(known, Some((outer, inner, current)) if outer == first_key && inner == second_key && current == generation)
                {
                    output.push(MirOp::Move {
                        destination,
                        source: scratch,
                    });
                } else {
                    output.push(MirOp::GetPathI64Constants {
                        destination,
                        collection,
                        first_key,
                        second_key,
                    });
                }
            }
            operation => {
                let destination = match &operation {
                    MirOp::Constant { destination, .. }
                    | MirOp::BoxI64 { destination, .. }
                    | MirOp::UnboxI64 { destination, .. }
                    | MirOp::Binary { destination, .. }
                    | MirOp::BinaryConstant { destination, .. }
                    | MirOp::ArrayNew { destination, .. }
                    | MirOp::ArrayGetI64 { destination, .. }
                    | MirOp::ArrayGetI64Constant { destination, .. }
                    | MirOp::ArraySetI64 { destination, .. }
                    | MirOp::ObjectNew { destination, .. }
                    | MirOp::ObjectGetI64 { destination, .. }
                    | MirOp::ObjectSetI64 { destination, .. }
                    | MirOp::BuildVector { destination, .. }
                    | MirOp::NativeVector { destination, .. }
                    | MirOp::BuildMap { destination, .. }
                    | MirOp::BuildMapI64Pair { destination, .. }
                    | MirOp::Assoc { destination, .. }
                    | MirOp::Get { destination, .. }
                    | MirOp::GetI64 { destination, .. }
                    | MirOp::IsNumber { destination, .. }
                    | MirOp::TaggedIsNumber { destination, .. }
                    | MirOp::Count { destination, .. }
                    | MirOp::TaggedCount { destination, .. }
                    | MirOp::Nth { destination, .. }
                    | MirOp::TaggedNth { destination, .. }
                    | MirOp::TaggedUnboxI64 { destination, .. }
                    | MirOp::CallStatic { destination, .. } => Some(*destination),
                    _ => None,
                };
                if let Some(destination) = destination {
                    constants.remove(&destination);
                    nested.remove(&destination);
                }
                output.push(operation);
            }
        }
    }
    output
}

fn scalar_constant(value: Option<&Value>) -> Option<(i64, Rep)> {
    match value? {
        Value::Number(value) => Some((*value, Rep::I64)),
        Value::Bool(value) => Some((i64::from(*value), Rep::Bool)),
        _ => None,
    }
}

fn target_name(program: &Program, target: u32) -> Option<&str> {
    match program.constants.get(target as usize) {
        Some(Value::String(name)) => Some(name),
        _ => None,
    }
}

fn target_is(program: &Program, target: u32, expected: &str) -> bool {
    target_name(program, target) == Some(expected)
}

fn declared_target_is(program: &Program, target: u32, expected: &str) -> bool {
    target_name(program, target) == Some(expected)
}

fn intrinsic_op(program: &Program, target: u32) -> Option<IntrinsicOp> {
    target_name(program, target).and_then(IntrinsicOp::from_symbol)
}

fn scalar_binary(op: IntrinsicOp) -> bool {
    matches!(
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
    )
}

fn scalar_arithmetic(op: IntrinsicOp) -> bool {
    matches!(
        op,
        IntrinsicOp::Add
            | IntrinsicOp::Subtract
            | IntrinsicOp::Multiply
            | IntrinsicOp::Divide
            | IntrinsicOp::Remainder
            | IntrinsicOp::Modulo
    )
}

fn function_for_global(program: &Program, constant: u32) -> Option<FunctionId> {
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

fn block_id(ids: &BTreeMap<usize, u16>, target: u32) -> Result<u16, String> {
    ids.get(&(target as usize))
        .copied()
        .ok_or_else(|| format!("whole-Wasm target {target} is not a block leader"))
}

fn unsupported(function: FunctionId, instruction: usize, detail: &str) -> String {
    format!("whole-Wasm function {function} instruction {instruction} unsupported: {detail}")
}

pub fn verify(program: &MirProgram) -> Result<(), String> {
    if program.functions.is_empty() || usize::from(program.entry) >= program.functions.len() {
        return Err("whole-Wasm MIR has an invalid entry".into());
    }
    for (expected, function) in program.functions.iter().enumerate() {
        if usize::from(function.id) != expected || function.blocks.is_empty() {
            return Err(format!("whole-Wasm MIR function {expected} is malformed"));
        }
        let slot_count = function
            .local_count
            .checked_add(function.stack_count)
            .ok_or("whole-Wasm MIR slot count overflow")?;
        let valid_slot = |slot: u16| slot < slot_count;
        for (expected_block, block) in function.blocks.iter().enumerate() {
            if usize::from(block.id) != expected_block {
                return Err(format!(
                    "whole-Wasm MIR block id mismatch in function {expected}"
                ));
            }
            for operation in &block.operations {
                let valid = match operation {
                    MirOp::Constant { destination, .. } => valid_slot(*destination),
                    MirOp::ConstantHandle { destination, .. } => valid_slot(*destination),
                    MirOp::BoxI64 {
                        destination,
                        source,
                    }
                    | MirOp::UnboxI64 {
                        destination,
                        source,
                    } => valid_slot(*destination) && valid_slot(*source),
                    MirOp::Move {
                        destination,
                        source,
                    } => valid_slot(*destination) && valid_slot(*source),
                    MirOp::Binary {
                        destination,
                        left,
                        right,
                        ..
                    } => valid_slot(*destination) && valid_slot(*left) && valid_slot(*right),
                    MirOp::BinaryConstant {
                        destination, left, ..
                    } => valid_slot(*destination) && valid_slot(*left),
                    MirOp::ArrayNew {
                        destination,
                        values,
                    } => valid_slot(*destination) && values.iter().all(|slot| valid_slot(*slot)),
                    MirOp::ArrayGetI64 {
                        destination,
                        array,
                        index,
                    } => valid_slot(*destination) && valid_slot(*array) && valid_slot(*index),
                    MirOp::ArrayGetI64Constant {
                        destination, array, ..
                    } => valid_slot(*destination) && valid_slot(*array),
                    MirOp::ArraySetI64 {
                        destination,
                        array,
                        index,
                        value,
                    } => {
                        valid_slot(*destination)
                            && valid_slot(*array)
                            && valid_slot(*index)
                            && valid_slot(*value)
                    }
                    MirOp::ObjectNew {
                        destination,
                        entries,
                    } => {
                        valid_slot(*destination)
                            && entries
                                .iter()
                                .all(|(key, value)| valid_slot(*key) && valid_slot(*value))
                    }
                    MirOp::ObjectGetI64 {
                        destination,
                        object,
                        key,
                    } => valid_slot(*destination) && valid_slot(*object) && valid_slot(*key),
                    MirOp::ObjectSetI64 {
                        destination,
                        object,
                        key,
                        value,
                    } => {
                        valid_slot(*destination)
                            && valid_slot(*object)
                            && valid_slot(*key)
                            && valid_slot(*value)
                    }
                    MirOp::BuildVector {
                        destination,
                        values,
                    } => valid_slot(*destination) && values.iter().all(|slot| valid_slot(*slot)),
                    MirOp::NativeVector {
                        destination,
                        values,
                    } => {
                        valid_slot(*destination)
                            && values.iter().all(|(slot, rep)| {
                                valid_slot(*slot) && matches!(rep, Rep::I64 | Rep::TaggedRef)
                            })
                    }
                    MirOp::BuildMap {
                        destination,
                        entries,
                    } => {
                        valid_slot(*destination)
                            && entries
                                .iter()
                                .all(|(key, value)| valid_slot(*key) && valid_slot(*value))
                    }
                    MirOp::BuildMapI64Pair {
                        destination,
                        key,
                        value,
                    } => valid_slot(*destination) && valid_slot(*key) && valid_slot(*value),
                    MirOp::Assoc {
                        destination,
                        collection,
                        key,
                        value,
                    } => {
                        valid_slot(*destination)
                            && valid_slot(*collection)
                            && valid_slot(*key)
                            && valid_slot(*value)
                    }
                    MirOp::AssocMapI64Pair {
                        destination,
                        collection,
                        outer_key,
                        inner_key,
                        value,
                    } => {
                        valid_slot(*destination)
                            && valid_slot(*collection)
                            && valid_slot(*outer_key)
                            && valid_slot(*inner_key)
                            && valid_slot(*value)
                    }
                    MirOp::Get {
                        destination,
                        collection,
                        key,
                    } => valid_slot(*destination) && valid_slot(*collection) && valid_slot(*key),
                    MirOp::GetI64 {
                        destination,
                        collection,
                        key,
                    } => valid_slot(*destination) && valid_slot(*collection) && valid_slot(*key),
                    MirOp::GetPathI64Constants {
                        destination,
                        collection,
                        ..
                    } => valid_slot(*destination) && valid_slot(*collection),
                    MirOp::IsNumber { destination, value } => {
                        valid_slot(*destination) && valid_slot(*value)
                    }
                    MirOp::TaggedIsNumber { destination, value } => {
                        valid_slot(*destination) && valid_slot(*value)
                    }
                    MirOp::Count {
                        destination,
                        collection,
                    } => valid_slot(*destination) && valid_slot(*collection),
                    MirOp::TaggedCount {
                        destination,
                        collection,
                    } => valid_slot(*destination) && valid_slot(*collection),
                    MirOp::Nth {
                        destination,
                        collection,
                        index,
                    } => valid_slot(*destination) && valid_slot(*collection) && valid_slot(*index),
                    MirOp::TaggedNth {
                        destination,
                        collection,
                        index,
                    } => valid_slot(*destination) && valid_slot(*collection) && valid_slot(*index),
                    MirOp::TaggedUnboxI64 {
                        destination,
                        source,
                    } => valid_slot(*destination) && valid_slot(*source),
                    MirOp::CallStatic {
                        destination,
                        function: target,
                        arguments,
                    } => {
                        valid_slot(*destination)
                            && program
                                .functions
                                .get(usize::from(*target))
                                .is_some_and(|callee| {
                                    usize::from(callee.arity) == arguments.len()
                                        && arguments.iter().all(|slot| valid_slot(*slot))
                                })
                    }
                };
                if !valid {
                    return Err(format!(
                        "whole-Wasm MIR function {expected} block {expected_block} has invalid operands"
                    ));
                }
            }
            let block_count = function.blocks.len();
            let valid = match block.terminator {
                MirTerminator::Goto(target) => usize::from(target) < block_count,
                MirTerminator::BranchZero {
                    condition,
                    rep: _,
                    zero,
                    nonzero,
                } => {
                    valid_slot(condition)
                        && usize::from(zero) < block_count
                        && usize::from(nonzero) < block_count
                }
                MirTerminator::Return(value) => valid_slot(value),
            };
            if !valid {
                return Err(format!(
                    "whole-Wasm MIR function {expected} block {expected_block} has an invalid terminator"
                ));
            }
        }
    }
    Ok(())
}
