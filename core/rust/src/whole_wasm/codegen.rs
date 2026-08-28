use std::collections::BTreeSet;
use wasm_encoder::{
    BlockType, CodeSection, ConstExpr, EntityType, ExportKind, ExportSection, Function,
    FunctionSection, GlobalSection, GlobalType, ImportSection, Instruction, MemArg, MemorySection,
    MemoryType, Module, TypeSection, ValType,
};

use crate::core::IntrinsicOp;

use super::bridge::{
    operation_id, HEAP_BASE, MAX_SLOTS, RESULT_BOOL, RESULT_HANDLE, RESULT_I64, SLOT_BOOL,
    SLOT_BYTES, SLOT_CONSTANT, SLOT_HANDLE, SLOT_I64, SLOT_NIL,
};
use super::ssa::{
    lower_program, operands, result as operation_result, verify, SsaEdge, SsaFunction,
    SsaOp as MirOp, SsaProgram, SsaTerminator, ValueId,
};
use crate::vm::Program;

/// Error codes published through the `hara_error` Wasm global before a trap.
pub const ERROR_INTEGER_OVERFLOW: i32 = 1;
pub const ERROR_DIVISION_BY_ZERO: i32 = 2;
pub const ERROR_ARRAY_BOUNDS: i32 = 3;
pub const ERROR_OBJECT_KEY: i32 = 4;
const HOST_TYPE_COUNT: u32 = 5;
const HOST_FUNCTION_COUNT: u32 = 5;
const HOST_CONSTANT: u32 = 0;
const HOST_BOX_I64: u32 = 1;
const HOST_UNBOX_I64: u32 = 2;
const HOST_VALUE_CONSTRUCT: u32 = 3;
const HOST_TARGET_CALL: u32 = 4;
const ARRAY_MEMORY: u32 = 0;
const ARRAY_HEAP_GLOBAL: u32 = 1;
const I64_MEMORY: MemArg = MemArg {
    offset: 0,
    align: 3,
    memory_index: ARRAY_MEMORY,
};

/// Compiles a complete eligible bytecode program into deterministic Wasm.
pub fn compile_program(program: &Program) -> Result<Vec<u8>, String> {
    emit_program(&lower_program(program)?)
}

pub(crate) fn emit_program(program: &SsaProgram) -> Result<Vec<u8>, String> {
    verify(program)?;
    let mut module = Module::new();
    let mut types = TypeSection::new();
    let mut functions = FunctionSection::new();
    types.function([ValType::I64], [ValType::I64]);
    types.function([], [ValType::I64]);
    types.function([ValType::I64, ValType::I64], [ValType::I64]);
    types.function([ValType::I64, ValType::I64, ValType::I64], [ValType::I64]);
    types.function(
        [ValType::I64, ValType::I64, ValType::I64, ValType::I64],
        [ValType::I64],
    );
    for function in &program.functions {
        types.function(
            std::iter::repeat(ValType::I64).take(usize::from(function.arity)),
            [ValType::I64],
        );
        functions.function(HOST_TYPE_COUNT + u32::from(function.id));
    }
    module.section(&types);
    let mut imports = ImportSection::new();
    for (name, ty) in [
        ("constant_handle", 0),
        ("box_i64", 0),
        ("unbox_i64", 0),
        ("value_construct", 3),
        ("target_call", 4),
    ] {
        imports.import("hara", name, EntityType::Function(ty));
    }
    module.section(&imports);
    module.section(&functions);

    let mut globals = GlobalSection::new();
    globals.global(
        GlobalType {
            val_type: ValType::I32,
            mutable: true,
        },
        &ConstExpr::i32_const(HEAP_BASE as i32),
    );
    globals.global(
        GlobalType {
            val_type: ValType::I32,
            mutable: true,
        },
        &ConstExpr::i32_const(0),
    );
    let mut memories = MemorySection::new();
    memories.memory(MemoryType {
        minimum: 1,
        maximum: Some(1),
        memory64: false,
        shared: false,
    });
    module.section(&memories);
    module.section(&globals);

    let mut exports = ExportSection::new();
    for function in &program.functions {
        exports.export(
            &format!("hara_fn_{}", function.id),
            ExportKind::Func,
            HOST_FUNCTION_COUNT + u32::from(function.id),
        );
    }
    exports.export(
        "hara_entry",
        ExportKind::Func,
        HOST_FUNCTION_COUNT + u32::from(program.entry),
    );
    exports.export("hara_error", ExportKind::Global, 0);
    exports.export("hara_heap", ExportKind::Global, ARRAY_HEAP_GLOBAL);
    exports.export("hara_memory", ExportKind::Memory, ARRAY_MEMORY);
    module.section(&exports);

    let mut code = CodeSection::new();
    for function in &program.functions {
        code.function(&emit_function(function)?);
    }
    module.section(&code);
    Ok(module.finish())
}

#[derive(Debug)]
struct LocalAllocation {
    values: Vec<u32>,
    count: u32,
}

impl LocalAllocation {
    fn get(&self, value: ValueId) -> u32 {
        self.values[value.0 as usize]
    }
}

/// Colors the SSA interference graph deterministically. Function parameters
/// remain precolored to their ABI locals; values whose lifetimes do not
/// overlap reuse those locals or the lowest available non-parameter local.
fn allocate_locals(function: &SsaFunction) -> LocalAllocation {
    let mut graph = vec![BTreeSet::<u32>::new(); function.value_count as usize];
    let mut interfere = |left: ValueId, right: ValueId| {
        if left != right {
            graph[left.0 as usize].insert(right.0);
            graph[right.0 as usize].insert(left.0);
        }
    };
    for block in &function.blocks {
        let mut live = terminator_values(&block.terminator);
        for operation in block.operations.iter().rev() {
            let destination = operation_result(operation);
            for value in &live {
                interfere(destination, ValueId(*value));
            }
            live.remove(&destination.0);
            live.extend(operands(operation).into_iter().map(|value| value.0));
        }
        for (index, parameter) in block.parameters.iter().enumerate() {
            for other in &block.parameters[index + 1..] {
                interfere(*parameter, *other);
            }
        }
    }
    drop(interfere);

    let mut values = vec![u32::MAX; function.value_count as usize];
    for parameter in 0..u32::from(function.arity) {
        values[parameter as usize] = parameter;
    }
    for value in u32::from(function.arity)..function.value_count {
        let used = graph[value as usize]
            .iter()
            .filter_map(|neighbor| {
                let color = values[*neighbor as usize];
                (color != u32::MAX).then_some(color)
            })
            .collect::<BTreeSet<_>>();
        values[value as usize] = (0..).find(|color| !used.contains(color)).unwrap();
    }
    let count = values.iter().copied().max().map_or(0, |value| value + 1);
    LocalAllocation { values, count }
}

fn terminator_values(terminator: &SsaTerminator) -> BTreeSet<u32> {
    let mut values = BTreeSet::new();
    match terminator {
        SsaTerminator::Goto(edge) => values.extend(edge.arguments.iter().map(|value| value.0)),
        SsaTerminator::BranchZero {
            condition,
            zero,
            nonzero,
            ..
        } => {
            values.insert(condition.0);
            values.extend(zero.arguments.iter().map(|value| value.0));
            values.extend(nonzero.arguments.iter().map(|value| value.0));
        }
        SsaTerminator::Return(value) => {
            values.insert(value.0);
        }
    }
    values
}

fn emit_function(mir: &SsaFunction) -> Result<Function, String> {
    if mir.value_count < u32::from(mir.arity) {
        return Err(format!("whole-Wasm function {} has too few slots", mir.id));
    }
    let locals = allocate_locals(mir);
    let temp_a = locals.count;
    let temp_b = temp_a + 1;
    let result = temp_a + 2;
    let pc = temp_a + 3;
    let shape = control_shape(mir);
    let scalar_locals = locals.count - u32::from(mir.arity) + 3;
    let mut declarations = Vec::new();
    if scalar_locals != 0 {
        declarations.push((scalar_locals, ValType::I64));
    }
    if matches!(shape, ControlShape::Dispatcher) {
        declarations.push((1, ValType::I32));
    }
    let mut out = Function::new(declarations);
    match shape {
        ControlShape::Forward => {
            emit_structured_block(&mut out, mir, 0, &locals, temp_a, temp_b, result)?;
            out.instruction(&Instruction::Unreachable);
            out.instruction(&Instruction::End);
            return Ok(out);
        }
        ControlShape::NaturalLoop(loop_shape) => {
            emit_natural_loop(&mut out, mir, loop_shape, &locals, temp_a, temp_b, result)?;
            out.instruction(&Instruction::Unreachable);
            out.instruction(&Instruction::End);
            return Ok(out);
        }
        ControlShape::Dispatcher => {}
    }
    out.instruction(&Instruction::I32Const(0));
    out.instruction(&Instruction::LocalSet(pc));
    out.instruction(&Instruction::Loop(BlockType::Empty));
    for block in &mir.blocks {
        out.instruction(&Instruction::LocalGet(pc));
        out.instruction(&Instruction::I32Const(i32::from(block.id.0)));
        out.instruction(&Instruction::I32Eq);
        out.instruction(&Instruction::If(BlockType::Empty));
        for operation in &block.operations {
            emit_operation(
                &mut out,
                operation,
                &locals,
                &mir.representations,
                temp_a,
                temp_b,
                result,
            )?;
        }
        emit_terminator(&mut out, &block.terminator, &mir.blocks, &locals, pc);
        out.instruction(&Instruction::End);
    }
    out.instruction(&Instruction::Unreachable);
    out.instruction(&Instruction::End);
    out.instruction(&Instruction::Unreachable);
    out.instruction(&Instruction::End);
    Ok(out)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ControlShape {
    Forward,
    NaturalLoop(NaturalLoop),
    Dispatcher,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct NaturalLoop {
    header: u16,
    body_entry: u16,
    exit: u16,
    exit_on_zero: bool,
}

fn control_shape(function: &SsaFunction) -> ControlShape {
    if is_forward_cfg(function) {
        return ControlShape::Forward;
    }
    natural_loop(function)
        .map(ControlShape::NaturalLoop)
        .unwrap_or(ControlShape::Dispatcher)
}

fn natural_loop(function: &SsaFunction) -> Option<NaturalLoop> {
    if function.blocks.len() < 4 {
        return None;
    }
    let SsaTerminator::Goto(entry) = &function.blocks[0].terminator else {
        return None;
    };
    let header = entry.target.0;
    let SsaTerminator::BranchZero { zero, nonzero, .. } =
        &function.blocks[usize::from(header)].terminator
    else {
        return None;
    };
    for (exit_on_zero, exit, body_entry) in [
        (true, zero.target.0, nonzero.target.0),
        (false, nonzero.target.0, zero.target.0),
    ] {
        if !matches!(
            function.blocks.get(usize::from(exit))?.terminator,
            SsaTerminator::Return(_)
        ) {
            continue;
        }
        let members = function
            .blocks
            .iter()
            .map(|block| block.id.0)
            .filter(|id| *id != 0 && *id != header && *id != exit)
            .collect::<BTreeSet<_>>();
        if !members.contains(&body_entry) {
            continue;
        }
        let mut has_backedge = false;
        let valid = members.iter().all(|id| {
            block_targets(&function.blocks[usize::from(*id)].terminator)
                .into_iter()
                .all(|target| {
                    if target == header {
                        has_backedge = true;
                        true
                    } else {
                        members.contains(&target) && target > *id
                    }
                })
        });
        if valid
            && has_backedge
            && reachable_members(function, body_entry, header, &members) == members
        {
            return Some(NaturalLoop {
                header,
                body_entry,
                exit,
                exit_on_zero,
            });
        }
    }
    None
}

fn block_targets(terminator: &SsaTerminator) -> Vec<u16> {
    match terminator {
        SsaTerminator::Goto(edge) => vec![edge.target.0],
        SsaTerminator::BranchZero { zero, nonzero, .. } => {
            vec![zero.target.0, nonzero.target.0]
        }
        SsaTerminator::Return(_) => Vec::new(),
    }
}

fn reachable_members(
    function: &SsaFunction,
    entry: u16,
    header: u16,
    members: &BTreeSet<u16>,
) -> BTreeSet<u16> {
    let mut reached = BTreeSet::new();
    let mut pending = vec![entry];
    while let Some(block) = pending.pop() {
        if block == header || !members.contains(&block) || !reached.insert(block) {
            continue;
        }
        pending.extend(block_targets(
            &function.blocks[usize::from(block)].terminator,
        ));
    }
    reached
}

fn emit_natural_loop(
    out: &mut Function,
    function: &SsaFunction,
    shape: NaturalLoop,
    locals: &LocalAllocation,
    temp_a: u32,
    temp_b: u32,
    result: u32,
) -> Result<(), String> {
    let entry = &function.blocks[0];
    for operation in &entry.operations {
        emit_operation(
            out,
            operation,
            locals,
            &function.representations,
            temp_a,
            temp_b,
            result,
        )?;
    }
    let SsaTerminator::Goto(entry_edge) = &entry.terminator else {
        unreachable!("natural loop entry verified")
    };
    emit_edge_values(out, entry_edge, &function.blocks, locals);

    out.instruction(&Instruction::Block(BlockType::Empty));
    out.instruction(&Instruction::Loop(BlockType::Empty));
    let header = &function.blocks[usize::from(shape.header)];
    for operation in &header.operations {
        emit_operation(
            out,
            operation,
            locals,
            &function.representations,
            temp_a,
            temp_b,
            result,
        )?;
    }
    let SsaTerminator::BranchZero {
        condition,
        rep,
        zero,
        nonzero,
    } = &header.terminator
    else {
        unreachable!("natural loop header verified")
    };
    emit_false_condition(out, *condition, *rep, locals);
    if !shape.exit_on_zero {
        out.instruction(&Instruction::I32Eqz);
    }
    let (exit_edge, body_edge) = if shape.exit_on_zero {
        (zero, nonzero)
    } else {
        (nonzero, zero)
    };
    out.instruction(&Instruction::If(BlockType::Empty));
    emit_edge_values(out, exit_edge, &function.blocks, locals);
    out.instruction(&Instruction::Br(2));
    out.instruction(&Instruction::Else);
    emit_edge_values(out, body_edge, &function.blocks, locals);
    emit_loop_body(
        out,
        function,
        shape.body_entry,
        shape.header,
        1,
        locals,
        temp_a,
        temp_b,
        result,
    )?;
    out.instruction(&Instruction::End);
    out.instruction(&Instruction::Unreachable);
    out.instruction(&Instruction::End);
    out.instruction(&Instruction::End);

    let exit = &function.blocks[usize::from(shape.exit)];
    for operation in &exit.operations {
        emit_operation(
            out,
            operation,
            locals,
            &function.representations,
            temp_a,
            temp_b,
            result,
        )?;
    }
    let SsaTerminator::Return(value) = exit.terminator else {
        unreachable!("natural loop exit verified")
    };
    out.instruction(&Instruction::LocalGet(locals.get(value)));
    out.instruction(&Instruction::Return);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn emit_loop_body(
    out: &mut Function,
    function: &SsaFunction,
    block_id: u16,
    header: u16,
    if_depth: u32,
    locals: &LocalAllocation,
    temp_a: u32,
    temp_b: u32,
    result: u32,
) -> Result<(), String> {
    let block = &function.blocks[usize::from(block_id)];
    for operation in &block.operations {
        emit_operation(
            out,
            operation,
            locals,
            &function.representations,
            temp_a,
            temp_b,
            result,
        )?;
    }
    let emit_successor = |out: &mut Function, edge: &SsaEdge, depth: u32| {
        emit_edge_values(out, edge, &function.blocks, locals);
        if edge.target.0 == header {
            out.instruction(&Instruction::Br(depth));
            Ok(())
        } else {
            emit_loop_body(
                out,
                function,
                edge.target.0,
                header,
                depth,
                locals,
                temp_a,
                temp_b,
                result,
            )
        }
    };
    match &block.terminator {
        SsaTerminator::Goto(edge) => emit_successor(out, edge, if_depth)?,
        SsaTerminator::BranchZero {
            condition,
            rep,
            zero,
            nonzero,
        } => {
            emit_false_condition(out, *condition, *rep, locals);
            out.instruction(&Instruction::If(BlockType::Empty));
            emit_successor(out, zero, if_depth + 1)?;
            out.instruction(&Instruction::Else);
            emit_successor(out, nonzero, if_depth + 1)?;
            out.instruction(&Instruction::End);
        }
        SsaTerminator::Return(_) => unreachable!("natural-loop body cannot return"),
    }
    Ok(())
}

fn is_forward_cfg(function: &SsaFunction) -> bool {
    function.blocks.iter().all(|block| {
        let current = block.id.0;
        match &block.terminator {
            SsaTerminator::Goto(edge) => edge.target.0 > current,
            SsaTerminator::BranchZero { zero, nonzero, .. } => {
                zero.target.0 > current && nonzero.target.0 > current
            }
            SsaTerminator::Return(_) => true,
        }
    })
}

fn emit_structured_block(
    out: &mut Function,
    function: &SsaFunction,
    block_id: u16,
    locals: &LocalAllocation,
    temp_a: u32,
    temp_b: u32,
    result: u32,
) -> Result<(), String> {
    let block = &function.blocks[usize::from(block_id)];
    for operation in &block.operations {
        emit_operation(
            out,
            operation,
            locals,
            &function.representations,
            temp_a,
            temp_b,
            result,
        )?;
    }
    match &block.terminator {
        SsaTerminator::Goto(edge) => {
            emit_edge_values(out, edge, &function.blocks, locals);
            emit_structured_block(out, function, edge.target.0, locals, temp_a, temp_b, result)?;
        }
        SsaTerminator::BranchZero {
            condition,
            rep,
            zero,
            nonzero,
        } => {
            emit_false_condition(out, *condition, *rep, locals);
            out.instruction(&Instruction::If(BlockType::Empty));
            emit_edge_values(out, zero, &function.blocks, locals);
            emit_structured_block(out, function, zero.target.0, locals, temp_a, temp_b, result)?;
            out.instruction(&Instruction::Else);
            emit_edge_values(out, nonzero, &function.blocks, locals);
            emit_structured_block(
                out,
                function,
                nonzero.target.0,
                locals,
                temp_a,
                temp_b,
                result,
            )?;
            out.instruction(&Instruction::End);
        }
        SsaTerminator::Return(value) => {
            out.instruction(&Instruction::LocalGet(locals.get(*value)));
            out.instruction(&Instruction::Return);
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
enum BridgeArg {
    Local(u32, super::ir::Rep),
    Constant(u32),
}

fn emit_target_call(
    out: &mut Function,
    target: i64,
    arguments: &[BridgeArg],
    result_mode: i64,
    destination: u32,
) -> Result<(), String> {
    emit_bridge_slots(out, arguments)?;
    out.instruction(&Instruction::I64Const(target));
    out.instruction(&Instruction::I64Const(0));
    out.instruction(&Instruction::I64Const(
        i64::try_from(arguments.len()).unwrap(),
    ));
    out.instruction(&Instruction::I64Const(result_mode));
    out.instruction(&Instruction::Call(HOST_TARGET_CALL));
    out.instruction(&Instruction::LocalSet(destination));
    Ok(())
}

fn emit_value_construct(
    out: &mut Function,
    target: i64,
    arguments: &[BridgeArg],
    destination: u32,
) -> Result<(), String> {
    emit_bridge_slots(out, arguments)?;
    out.instruction(&Instruction::I64Const(target));
    out.instruction(&Instruction::I64Const(0));
    out.instruction(&Instruction::I64Const(
        i64::try_from(arguments.len()).unwrap(),
    ));
    out.instruction(&Instruction::Call(HOST_VALUE_CONSTRUCT));
    out.instruction(&Instruction::LocalSet(destination));
    Ok(())
}

fn emit_bridge_slots(out: &mut Function, arguments: &[BridgeArg]) -> Result<(), String> {
    if arguments.len() > usize::try_from(MAX_SLOTS).expect("constant fits usize") {
        return Err("whole-Wasm bridge call has too many arguments".into());
    }
    for (index, argument) in arguments.iter().enumerate() {
        let offset = u32::try_from(index)
            .ok()
            .and_then(|index| index.checked_mul(SLOT_BYTES))
            .ok_or("whole-Wasm bridge argument offset overflow")?;
        let kind = match argument {
            BridgeArg::Constant(_) => SLOT_CONSTANT,
            BridgeArg::Local(_, super::ir::Rep::I64) => SLOT_I64,
            BridgeArg::Local(_, super::ir::Rep::Bool) => SLOT_BOOL,
            BridgeArg::Local(_, super::ir::Rep::Nil) => SLOT_NIL,
            BridgeArg::Local(_, super::ir::Rep::KeyRef) => SLOT_CONSTANT,
            BridgeArg::Local(_, _) => SLOT_HANDLE,
        };
        out.instruction(&Instruction::I32Const(0));
        out.instruction(&Instruction::I32Const(kind as i32));
        out.instruction(&Instruction::I32Store(MemArg {
            offset: u64::from(offset),
            align: 2,
            memory_index: ARRAY_MEMORY,
        }));
        out.instruction(&Instruction::I32Const(0));
        out.instruction(&Instruction::I32Const(0));
        out.instruction(&Instruction::I32Store(MemArg {
            offset: u64::from(offset + 4),
            align: 2,
            memory_index: ARRAY_MEMORY,
        }));
        out.instruction(&Instruction::I32Const(0));
        match argument {
            BridgeArg::Constant(value) => {
                out.instruction(&Instruction::I64Const(i64::from(*value)));
            }
            BridgeArg::Local(local, super::ir::Rep::KeyRef) => {
                out.instruction(&Instruction::LocalGet(*local));
                out.instruction(&Instruction::I64Const(1));
                out.instruction(&Instruction::I64Sub);
            }
            BridgeArg::Local(local, _) => {
                out.instruction(&Instruction::LocalGet(*local));
            }
        }
        out.instruction(&Instruction::I64Store(MemArg {
            offset: u64::from(offset + 8),
            align: 3,
            memory_index: ARRAY_MEMORY,
        }));
    }
    Ok(())
}

fn emit_operation(
    out: &mut Function,
    operation: &MirOp,
    locals: &LocalAllocation,
    representations: &[super::ir::Rep],
    temp_a: u32,
    temp_b: u32,
    result: u32,
) -> Result<(), String> {
    match operation {
        MirOp::Constant {
            destination, value, ..
        } => {
            out.instruction(&Instruction::I64Const(*value));
            out.instruction(&Instruction::LocalSet(locals.get(*destination)));
        }
        MirOp::ConstantHandle {
            destination,
            constant,
        } => {
            out.instruction(&Instruction::I64Const(i64::from(*constant)));
            out.instruction(&Instruction::Call(HOST_CONSTANT));
            out.instruction(&Instruction::LocalSet(locals.get(*destination)));
        }
        MirOp::BoxI64 {
            destination,
            source,
        } => {
            out.instruction(&Instruction::LocalGet(locals.get(*source)));
            out.instruction(&Instruction::Call(HOST_BOX_I64));
            out.instruction(&Instruction::LocalSet(locals.get(*destination)));
        }
        MirOp::UnboxI64 {
            destination,
            source,
        } => {
            out.instruction(&Instruction::LocalGet(locals.get(*source)));
            out.instruction(&Instruction::Call(HOST_UNBOX_I64));
            out.instruction(&Instruction::LocalSet(locals.get(*destination)));
        }
        MirOp::Move {
            destination,
            source,
        } => {
            out.instruction(&Instruction::LocalGet(locals.get(*source)));
            out.instruction(&Instruction::LocalSet(locals.get(*destination)));
        }
        MirOp::Binary {
            destination,
            left,
            right,
            op,
        } => emit_binary(
            out,
            *destination,
            |out| out.instruction(&Instruction::LocalGet(locals.get(*left))),
            |out| out.instruction(&Instruction::LocalGet(locals.get(*right))),
            *op,
            temp_a,
            temp_b,
            result,
            locals,
        )?,
        MirOp::BinaryConstant {
            destination,
            left,
            right,
            op,
        } => emit_binary(
            out,
            *destination,
            |out| out.instruction(&Instruction::LocalGet(locals.get(*left))),
            |out| out.instruction(&Instruction::I64Const(*right)),
            *op,
            temp_a,
            temp_b,
            result,
            locals,
        )?,
        MirOp::ArrayNew {
            destination,
            values,
        } => {
            let bytes = (values.len() + 1)
                .checked_mul(8)
                .and_then(|value| i32::try_from(value).ok())
                .ok_or("whole-Wasm array allocation is too large")?;
            out.instruction(&Instruction::GlobalGet(ARRAY_HEAP_GLOBAL));
            out.instruction(&Instruction::I64ExtendI32U);
            out.instruction(&Instruction::LocalSet(result));
            out.instruction(&Instruction::LocalGet(result));
            out.instruction(&Instruction::I32WrapI64);
            out.instruction(&Instruction::I64Const(values.len() as i64));
            out.instruction(&Instruction::I64Store(I64_MEMORY));
            for (index, value) in values.iter().enumerate() {
                out.instruction(&Instruction::LocalGet(result));
                out.instruction(&Instruction::I32WrapI64);
                out.instruction(&Instruction::LocalGet(locals.get(*value)));
                out.instruction(&Instruction::I64Store(MemArg {
                    offset: ((index + 1) * 8) as u64,
                    ..I64_MEMORY
                }));
            }
            out.instruction(&Instruction::GlobalGet(ARRAY_HEAP_GLOBAL));
            out.instruction(&Instruction::I32Const(bytes));
            out.instruction(&Instruction::I32Add);
            out.instruction(&Instruction::GlobalSet(ARRAY_HEAP_GLOBAL));
            out.instruction(&Instruction::LocalGet(result));
            out.instruction(&Instruction::LocalSet(locals.get(*destination)));
        }
        MirOp::ArrayGetI64 {
            destination,
            array,
            index,
        } => {
            emit_array_address(out, *array, locals, |out| {
                out.instruction(&Instruction::LocalGet(locals.get(*index)))
            });
            out.instruction(&Instruction::I64Load(I64_MEMORY));
            out.instruction(&Instruction::LocalSet(locals.get(*destination)));
        }
        MirOp::ArrayGetI64Constant {
            destination,
            array,
            index,
        } => {
            emit_array_address(out, *array, locals, |out| {
                out.instruction(&Instruction::I64Const(*index))
            });
            out.instruction(&Instruction::I64Load(I64_MEMORY));
            out.instruction(&Instruction::LocalSet(locals.get(*destination)));
        }
        MirOp::ArraySetI64 {
            destination,
            array,
            index,
            value,
        } => {
            emit_array_address(out, *array, locals, |out| {
                out.instruction(&Instruction::LocalGet(locals.get(*index)))
            });
            out.instruction(&Instruction::LocalGet(locals.get(*value)));
            out.instruction(&Instruction::I64Store(I64_MEMORY));
            out.instruction(&Instruction::LocalGet(locals.get(*array)));
            out.instruction(&Instruction::LocalSet(locals.get(*destination)));
        }
        MirOp::ObjectNew {
            destination,
            entries,
        } => {
            let bytes = entries
                .len()
                .checked_mul(16)
                .and_then(|value| value.checked_add(8))
                .and_then(|value| i32::try_from(value).ok())
                .ok_or("whole-Wasm object allocation is too large")?;
            out.instruction(&Instruction::GlobalGet(ARRAY_HEAP_GLOBAL));
            out.instruction(&Instruction::I64ExtendI32U);
            out.instruction(&Instruction::LocalSet(result));
            out.instruction(&Instruction::LocalGet(result));
            out.instruction(&Instruction::I32WrapI64);
            out.instruction(&Instruction::I64Const(entries.len() as i64));
            out.instruction(&Instruction::I64Store(I64_MEMORY));
            for (index, (key, value)) in entries.iter().enumerate() {
                let key_offset = 8 + index * 16;
                out.instruction(&Instruction::LocalGet(result));
                out.instruction(&Instruction::I32WrapI64);
                out.instruction(&Instruction::LocalGet(locals.get(*key)));
                out.instruction(&Instruction::I64Store(MemArg {
                    offset: key_offset as u64,
                    ..I64_MEMORY
                }));
                out.instruction(&Instruction::LocalGet(result));
                out.instruction(&Instruction::I32WrapI64);
                out.instruction(&Instruction::LocalGet(locals.get(*value)));
                out.instruction(&Instruction::I64Store(MemArg {
                    offset: (key_offset + 8) as u64,
                    ..I64_MEMORY
                }));
            }
            out.instruction(&Instruction::GlobalGet(ARRAY_HEAP_GLOBAL));
            out.instruction(&Instruction::I32Const(bytes));
            out.instruction(&Instruction::I32Add);
            out.instruction(&Instruction::GlobalSet(ARRAY_HEAP_GLOBAL));
            out.instruction(&Instruction::LocalGet(result));
            out.instruction(&Instruction::LocalSet(locals.get(*destination)));
        }
        MirOp::ObjectGetI64 {
            destination,
            object,
            key,
        } => {
            emit_object_value_address(out, *object, *key, locals, temp_a, result);
            out.instruction(&Instruction::I64Load(I64_MEMORY));
            out.instruction(&Instruction::LocalSet(locals.get(*destination)));
        }
        MirOp::ObjectSetI64 {
            destination,
            object,
            key,
            value,
        } => {
            emit_object_value_address(out, *object, *key, locals, temp_a, result);
            out.instruction(&Instruction::LocalGet(locals.get(*value)));
            out.instruction(&Instruction::I64Store(I64_MEMORY));
            out.instruction(&Instruction::LocalGet(locals.get(*object)));
            out.instruction(&Instruction::LocalSet(locals.get(*destination)));
        }
        MirOp::BuildVector {
            destination,
            values,
        } => {
            let arguments = values
                .iter()
                .map(|value| {
                    BridgeArg::Local(locals.get(*value), representations[value.0 as usize])
                })
                .collect::<Vec<_>>();
            emit_value_construct(
                out,
                operation_id("hara.whole-wasm/vector")?,
                &arguments,
                locals.get(*destination),
            )?;
        }
        MirOp::NativeVector {
            destination,
            values,
        } => {
            let bytes = values
                .len()
                .checked_mul(16)
                .and_then(|value| value.checked_add(24))
                .and_then(|value| i32::try_from(value).ok())
                .ok_or("whole-Wasm tagged vector allocation is too large")?;
            out.instruction(&Instruction::GlobalGet(ARRAY_HEAP_GLOBAL));
            out.instruction(&Instruction::I64ExtendI32U);
            out.instruction(&Instruction::LocalSet(result));
            out.instruction(&Instruction::LocalGet(result));
            out.instruction(&Instruction::I32WrapI64);
            out.instruction(&Instruction::I64Const(1));
            out.instruction(&Instruction::I64Store(I64_MEMORY));
            out.instruction(&Instruction::LocalGet(result));
            out.instruction(&Instruction::I32WrapI64);
            out.instruction(&Instruction::LocalGet(result));
            out.instruction(&Instruction::I64Const(16));
            out.instruction(&Instruction::I64Add);
            out.instruction(&Instruction::I64Store(MemArg {
                offset: 8,
                ..I64_MEMORY
            }));
            out.instruction(&Instruction::LocalGet(result));
            out.instruction(&Instruction::I32WrapI64);
            out.instruction(&Instruction::I64Const(values.len() as i64));
            out.instruction(&Instruction::I64Store(MemArg {
                offset: 16,
                ..I64_MEMORY
            }));
            for (index, (value, rep)) in values.iter().enumerate() {
                let tag_offset = 24 + index * 16;
                let payload_offset = tag_offset + 8;
                out.instruction(&Instruction::LocalGet(result));
                out.instruction(&Instruction::I32WrapI64);
                match rep {
                    super::ir::Rep::I64 => out.instruction(&Instruction::I64Const(0)),
                    super::ir::Rep::TaggedRef => {
                        out.instruction(&Instruction::LocalGet(locals.get(*value)));
                        out.instruction(&Instruction::I32WrapI64);
                        out.instruction(&Instruction::I64Load(I64_MEMORY))
                    }
                    _ => unreachable!("native vector reps verified by MIR"),
                };
                out.instruction(&Instruction::I64Store(MemArg {
                    offset: tag_offset as u64,
                    ..I64_MEMORY
                }));
                out.instruction(&Instruction::LocalGet(result));
                out.instruction(&Instruction::I32WrapI64);
                match rep {
                    super::ir::Rep::I64 => {
                        out.instruction(&Instruction::LocalGet(locals.get(*value)))
                    }
                    super::ir::Rep::TaggedRef => {
                        out.instruction(&Instruction::LocalGet(locals.get(*value)));
                        out.instruction(&Instruction::I32WrapI64);
                        out.instruction(&Instruction::I64Load(MemArg {
                            offset: 8,
                            ..I64_MEMORY
                        }))
                    }
                    _ => unreachable!("native vector reps verified by MIR"),
                };
                out.instruction(&Instruction::I64Store(MemArg {
                    offset: payload_offset as u64,
                    ..I64_MEMORY
                }));
            }
            out.instruction(&Instruction::GlobalGet(ARRAY_HEAP_GLOBAL));
            out.instruction(&Instruction::I32Const(bytes));
            out.instruction(&Instruction::I32Add);
            out.instruction(&Instruction::GlobalSet(ARRAY_HEAP_GLOBAL));
            out.instruction(&Instruction::LocalGet(result));
            out.instruction(&Instruction::LocalSet(locals.get(*destination)));
        }
        MirOp::BuildMap {
            destination,
            entries,
        } => {
            let arguments = entries
                .iter()
                .flat_map(|(key, value)| {
                    [
                        BridgeArg::Local(locals.get(*key), representations[key.0 as usize]),
                        BridgeArg::Local(locals.get(*value), representations[value.0 as usize]),
                    ]
                })
                .collect::<Vec<_>>();
            emit_value_construct(
                out,
                operation_id("hara.whole-wasm/map")?,
                &arguments,
                locals.get(*destination),
            )?;
        }
        MirOp::BuildMapI64Pair {
            destination,
            key,
            value,
        } => {
            emit_value_construct(
                out,
                operation_id("hara.whole-wasm/map")?,
                &[
                    BridgeArg::Local(locals.get(*key), representations[key.0 as usize]),
                    BridgeArg::Local(locals.get(*value), representations[value.0 as usize]),
                ],
                locals.get(*destination),
            )?;
        }
        MirOp::Assoc {
            destination,
            collection,
            key,
            value,
        } => {
            emit_target_call(
                out,
                operation_id("std.protocol.iassoc.IAssoc/assoc")?,
                &[
                    BridgeArg::Local(
                        locals.get(*collection),
                        representations[collection.0 as usize],
                    ),
                    BridgeArg::Local(locals.get(*key), representations[key.0 as usize]),
                    BridgeArg::Local(locals.get(*value), representations[value.0 as usize]),
                ],
                RESULT_HANDLE,
                locals.get(*destination),
            )?;
        }
        MirOp::AssocMapI64Pair {
            destination,
            collection,
            outer_key,
            inner_key,
            value,
        } => {
            emit_value_construct(
                out,
                operation_id("hara.whole-wasm/map")?,
                &[
                    BridgeArg::Local(
                        locals.get(*inner_key),
                        representations[inner_key.0 as usize],
                    ),
                    BridgeArg::Local(locals.get(*value), representations[value.0 as usize]),
                ],
                temp_a,
            )?;
            emit_target_call(
                out,
                operation_id("std.protocol.iassoc.IAssoc/assoc")?,
                &[
                    BridgeArg::Local(
                        locals.get(*collection),
                        representations[collection.0 as usize],
                    ),
                    BridgeArg::Local(
                        locals.get(*outer_key),
                        representations[outer_key.0 as usize],
                    ),
                    BridgeArg::Local(temp_a, super::ir::Rep::TruthyHandle),
                ],
                RESULT_HANDLE,
                locals.get(*destination),
            )?;
        }
        MirOp::Get {
            destination,
            collection,
            key,
        } => {
            emit_target_call(
                out,
                operation_id("std.protocol.ilookup.ILookup/lookup")?,
                &[
                    BridgeArg::Local(
                        locals.get(*collection),
                        representations[collection.0 as usize],
                    ),
                    BridgeArg::Local(locals.get(*key), representations[key.0 as usize]),
                ],
                RESULT_HANDLE,
                locals.get(*destination),
            )?;
        }
        MirOp::GetI64 {
            destination,
            collection,
            key,
        } => {
            emit_target_call(
                out,
                operation_id("std.protocol.ilookup.ILookup/lookup")?,
                &[
                    BridgeArg::Local(
                        locals.get(*collection),
                        representations[collection.0 as usize],
                    ),
                    BridgeArg::Local(locals.get(*key), representations[key.0 as usize]),
                ],
                RESULT_I64,
                locals.get(*destination),
            )?;
        }
        MirOp::GetPathI64Constants {
            destination,
            collection,
            first_key,
            second_key,
        } => {
            emit_target_call(
                out,
                operation_id("std.protocol.ilookup.ILookup/lookup")?,
                &[
                    BridgeArg::Local(
                        locals.get(*collection),
                        representations[collection.0 as usize],
                    ),
                    BridgeArg::Constant(*first_key),
                ],
                RESULT_HANDLE,
                temp_a,
            )?;
            emit_target_call(
                out,
                operation_id("std.protocol.ilookup.ILookup/lookup")?,
                &[
                    BridgeArg::Local(temp_a, super::ir::Rep::TruthyHandle),
                    BridgeArg::Constant(*second_key),
                ],
                RESULT_I64,
                locals.get(*destination),
            )?;
        }
        MirOp::IsNumber { destination, value } => {
            emit_target_call(
                out,
                operation_id("std.native.Base/number?")?,
                &[BridgeArg::Local(
                    locals.get(*value),
                    representations[value.0 as usize],
                )],
                RESULT_BOOL,
                locals.get(*destination),
            )?;
        }
        MirOp::TaggedIsNumber { destination, value } => {
            out.instruction(&Instruction::LocalGet(locals.get(*value)));
            out.instruction(&Instruction::I32WrapI64);
            out.instruction(&Instruction::I64Load(I64_MEMORY));
            out.instruction(&Instruction::I64Eqz);
            out.instruction(&Instruction::I64ExtendI32U);
            out.instruction(&Instruction::LocalSet(locals.get(*destination)));
        }
        MirOp::Count {
            destination,
            collection,
        } => {
            emit_target_call(
                out,
                operation_id("std.protocol.icount.ICount/count")?,
                &[BridgeArg::Local(
                    locals.get(*collection),
                    representations[collection.0 as usize],
                )],
                RESULT_I64,
                locals.get(*destination),
            )?;
        }
        MirOp::TaggedCount {
            destination,
            collection,
        } => {
            out.instruction(&Instruction::LocalGet(locals.get(*collection)));
            out.instruction(&Instruction::I32WrapI64);
            out.instruction(&Instruction::I64Load(MemArg {
                offset: 8,
                ..I64_MEMORY
            }));
            out.instruction(&Instruction::I32WrapI64);
            out.instruction(&Instruction::I64Load(I64_MEMORY));
            out.instruction(&Instruction::LocalSet(locals.get(*destination)));
        }
        MirOp::Nth {
            destination,
            collection,
            index,
        } => {
            emit_target_call(
                out,
                operation_id("std.protocol.inth.INth/nth")?,
                &[
                    BridgeArg::Local(
                        locals.get(*collection),
                        representations[collection.0 as usize],
                    ),
                    BridgeArg::Local(locals.get(*index), representations[index.0 as usize]),
                ],
                RESULT_HANDLE,
                locals.get(*destination),
            )?;
        }
        MirOp::TaggedNth {
            destination,
            collection,
            index,
        } => {
            out.instruction(&Instruction::LocalGet(locals.get(*collection)));
            out.instruction(&Instruction::I32WrapI64);
            out.instruction(&Instruction::I64Load(MemArg {
                offset: 8,
                ..I64_MEMORY
            }));
            out.instruction(&Instruction::LocalSet(temp_a));
            out.instruction(&Instruction::LocalGet(locals.get(*index)));
            out.instruction(&Instruction::I64Const(0));
            out.instruction(&Instruction::I64LtS);
            out.instruction(&Instruction::If(BlockType::Empty));
            emit_error(out, ERROR_ARRAY_BOUNDS);
            out.instruction(&Instruction::End);
            out.instruction(&Instruction::LocalGet(locals.get(*index)));
            out.instruction(&Instruction::LocalGet(temp_a));
            out.instruction(&Instruction::I32WrapI64);
            out.instruction(&Instruction::I64Load(I64_MEMORY));
            out.instruction(&Instruction::I64GeU);
            out.instruction(&Instruction::If(BlockType::Empty));
            emit_error(out, ERROR_ARRAY_BOUNDS);
            out.instruction(&Instruction::End);
            out.instruction(&Instruction::LocalGet(temp_a));
            out.instruction(&Instruction::LocalGet(locals.get(*index)));
            out.instruction(&Instruction::I64Const(16));
            out.instruction(&Instruction::I64Mul);
            out.instruction(&Instruction::I64Add);
            out.instruction(&Instruction::I64Const(8));
            out.instruction(&Instruction::I64Add);
            out.instruction(&Instruction::LocalSet(locals.get(*destination)));
        }
        MirOp::TaggedUnboxI64 {
            destination,
            source,
        } => {
            out.instruction(&Instruction::LocalGet(locals.get(*source)));
            out.instruction(&Instruction::I32WrapI64);
            out.instruction(&Instruction::I64Load(MemArg {
                offset: 8,
                ..I64_MEMORY
            }));
            out.instruction(&Instruction::LocalSet(locals.get(*destination)));
        }
        MirOp::CallStatic {
            destination,
            function,
            arguments,
        } => {
            for argument in arguments {
                out.instruction(&Instruction::LocalGet(locals.get(*argument)));
            }
            out.instruction(&Instruction::Call(
                HOST_FUNCTION_COUNT + u32::from(*function),
            ));
            out.instruction(&Instruction::LocalSet(locals.get(*destination)));
        }
    }
    Ok(())
}

fn emit_binary<L, R>(
    out: &mut Function,
    destination: ValueId,
    left: L,
    right: R,
    op: IntrinsicOp,
    a: u32,
    b: u32,
    result: u32,
    locals: &LocalAllocation,
) -> Result<(), String>
where
    L: Fn(&mut Function) -> &mut Function,
    R: Fn(&mut Function) -> &mut Function,
{
    left(out);
    out.instruction(&Instruction::LocalSet(a));
    right(out);
    out.instruction(&Instruction::LocalSet(b));
    match op {
        IntrinsicOp::Add | IntrinsicOp::Subtract | IntrinsicOp::Multiply => {
            out.instruction(&Instruction::LocalGet(a));
            out.instruction(&Instruction::LocalGet(b));
            out.instruction(&match op {
                IntrinsicOp::Add => Instruction::I64Add,
                IntrinsicOp::Subtract => Instruction::I64Sub,
                IntrinsicOp::Multiply => Instruction::I64Mul,
                _ => unreachable!(),
            });
            out.instruction(&Instruction::LocalSet(result));
            emit_overflow_check(out, op, a, b, result);
            out.instruction(&Instruction::LocalGet(result));
        }
        IntrinsicOp::Divide => {
            out.instruction(&Instruction::LocalGet(b));
            out.instruction(&Instruction::I64Eqz);
            out.instruction(&Instruction::If(BlockType::Empty));
            emit_error(out, ERROR_DIVISION_BY_ZERO);
            out.instruction(&Instruction::End);
            out.instruction(&Instruction::LocalGet(a));
            out.instruction(&Instruction::I64Const(i64::MIN));
            out.instruction(&Instruction::I64Eq);
            out.instruction(&Instruction::LocalGet(b));
            out.instruction(&Instruction::I64Const(-1));
            out.instruction(&Instruction::I64Eq);
            out.instruction(&Instruction::I32And);
            out.instruction(&Instruction::If(BlockType::Empty));
            emit_error(out, ERROR_INTEGER_OVERFLOW);
            out.instruction(&Instruction::End);
            out.instruction(&Instruction::LocalGet(a));
            out.instruction(&Instruction::LocalGet(b));
            out.instruction(&Instruction::I64DivS);
        }
        IntrinsicOp::Remainder | IntrinsicOp::Modulo => {
            out.instruction(&Instruction::LocalGet(b));
            out.instruction(&Instruction::I64Eqz);
            out.instruction(&Instruction::If(BlockType::Empty));
            emit_error(out, ERROR_DIVISION_BY_ZERO);
            out.instruction(&Instruction::End);
            out.instruction(&Instruction::LocalGet(a));
            out.instruction(&Instruction::LocalGet(b));
            out.instruction(&Instruction::I64RemS);
        }
        IntrinsicOp::Equal
        | IntrinsicOp::Less
        | IntrinsicOp::LessOrEqual
        | IntrinsicOp::Greater
        | IntrinsicOp::GreaterOrEqual => {
            out.instruction(&Instruction::LocalGet(a));
            out.instruction(&Instruction::LocalGet(b));
            out.instruction(&match op {
                IntrinsicOp::Equal => Instruction::I64Eq,
                IntrinsicOp::Less => Instruction::I64LtS,
                IntrinsicOp::LessOrEqual => Instruction::I64LeS,
                IntrinsicOp::Greater => Instruction::I64GtS,
                IntrinsicOp::GreaterOrEqual => Instruction::I64GeS,
                _ => unreachable!(),
            });
            out.instruction(&Instruction::I64ExtendI32U);
        }
        _ => return Err(format!("whole-Wasm cannot emit primitive {op:?}")),
    }
    out.instruction(&Instruction::LocalSet(locals.get(destination)));
    Ok(())
}

fn emit_overflow_check(out: &mut Function, op: IntrinsicOp, a: u32, b: u32, result: u32) {
    match op {
        IntrinsicOp::Add => {
            out.instruction(&Instruction::LocalGet(a));
            out.instruction(&Instruction::LocalGet(result));
            out.instruction(&Instruction::I64Xor);
            out.instruction(&Instruction::LocalGet(b));
            out.instruction(&Instruction::LocalGet(result));
            out.instruction(&Instruction::I64Xor);
            out.instruction(&Instruction::I64And);
            out.instruction(&Instruction::I64Const(0));
            out.instruction(&Instruction::I64LtS);
        }
        IntrinsicOp::Subtract => {
            out.instruction(&Instruction::LocalGet(a));
            out.instruction(&Instruction::LocalGet(b));
            out.instruction(&Instruction::I64Xor);
            out.instruction(&Instruction::LocalGet(a));
            out.instruction(&Instruction::LocalGet(result));
            out.instruction(&Instruction::I64Xor);
            out.instruction(&Instruction::I64And);
            out.instruction(&Instruction::I64Const(0));
            out.instruction(&Instruction::I64LtS);
        }
        IntrinsicOp::Multiply => {
            out.instruction(&Instruction::LocalGet(b));
            out.instruction(&Instruction::I64Eqz);
            out.instruction(&Instruction::I32Eqz);
            out.instruction(&Instruction::If(BlockType::Result(ValType::I32)));
            out.instruction(&Instruction::LocalGet(a));
            out.instruction(&Instruction::I64Const(i64::MIN));
            out.instruction(&Instruction::I64Eq);
            out.instruction(&Instruction::LocalGet(b));
            out.instruction(&Instruction::I64Const(-1));
            out.instruction(&Instruction::I64Eq);
            out.instruction(&Instruction::I32And);
            out.instruction(&Instruction::If(BlockType::Result(ValType::I32)));
            out.instruction(&Instruction::I32Const(1));
            out.instruction(&Instruction::Else);
            out.instruction(&Instruction::LocalGet(result));
            out.instruction(&Instruction::LocalGet(b));
            out.instruction(&Instruction::I64DivS);
            out.instruction(&Instruction::LocalGet(a));
            out.instruction(&Instruction::I64Ne);
            out.instruction(&Instruction::End);
            out.instruction(&Instruction::Else);
            out.instruction(&Instruction::I32Const(0));
            out.instruction(&Instruction::End);
        }
        _ => unreachable!(),
    }
    out.instruction(&Instruction::If(BlockType::Empty));
    emit_error(out, ERROR_INTEGER_OVERFLOW);
    out.instruction(&Instruction::End);
}

fn emit_error(out: &mut Function, code: i32) {
    out.instruction(&Instruction::I32Const(code));
    out.instruction(&Instruction::GlobalSet(0));
    out.instruction(&Instruction::Unreachable);
}

fn emit_array_address<I>(out: &mut Function, array: ValueId, locals: &LocalAllocation, index: I)
where
    I: Fn(&mut Function) -> &mut Function,
{
    index(out);
    out.instruction(&Instruction::I64Const(0));
    out.instruction(&Instruction::I64LtS);
    out.instruction(&Instruction::If(BlockType::Empty));
    emit_error(out, ERROR_ARRAY_BOUNDS);
    out.instruction(&Instruction::End);

    index(out);
    out.instruction(&Instruction::LocalGet(locals.get(array)));
    out.instruction(&Instruction::I32WrapI64);
    out.instruction(&Instruction::I64Load(I64_MEMORY));
    out.instruction(&Instruction::I64GeU);
    out.instruction(&Instruction::If(BlockType::Empty));
    emit_error(out, ERROR_ARRAY_BOUNDS);
    out.instruction(&Instruction::End);

    out.instruction(&Instruction::LocalGet(locals.get(array)));
    out.instruction(&Instruction::I32WrapI64);
    index(out);
    out.instruction(&Instruction::I32WrapI64);
    out.instruction(&Instruction::I32Const(8));
    out.instruction(&Instruction::I32Mul);
    out.instruction(&Instruction::I32Add);
    out.instruction(&Instruction::I32Const(8));
    out.instruction(&Instruction::I32Add);
}

fn emit_terminator(
    out: &mut Function,
    terminator: &SsaTerminator,
    blocks: &[super::ssa::SsaBlock],
    locals: &LocalAllocation,
    pc: u32,
) {
    match terminator {
        SsaTerminator::Goto(edge) => {
            emit_edge(out, edge, blocks, locals, pc);
            out.instruction(&Instruction::Br(1));
        }
        SsaTerminator::BranchZero {
            condition,
            rep,
            zero,
            nonzero,
        } => {
            emit_false_condition(out, *condition, *rep, locals);
            out.instruction(&Instruction::If(BlockType::Empty));
            emit_edge(out, zero, blocks, locals, pc);
            out.instruction(&Instruction::Else);
            emit_edge(out, nonzero, blocks, locals, pc);
            out.instruction(&Instruction::End);
            out.instruction(&Instruction::Br(1));
        }
        SsaTerminator::Return(value) => {
            out.instruction(&Instruction::LocalGet(locals.get(*value)));
            out.instruction(&Instruction::Return);
        }
    }
}

fn emit_edge(
    out: &mut Function,
    edge: &SsaEdge,
    blocks: &[super::ssa::SsaBlock],
    locals: &LocalAllocation,
    pc: u32,
) {
    emit_edge_values(out, edge, blocks, locals);
    out.instruction(&Instruction::I32Const(i32::from(edge.target.0)));
    out.instruction(&Instruction::LocalSet(pc));
}

fn emit_edge_values(
    out: &mut Function,
    edge: &SsaEdge,
    blocks: &[super::ssa::SsaBlock],
    locals: &LocalAllocation,
) {
    // Values are first placed on the operand stack so loop-edge swaps remain
    // parallel assignments rather than observing an already-updated local.
    for argument in &edge.arguments {
        out.instruction(&Instruction::LocalGet(locals.get(*argument)));
    }
    for parameter in blocks[usize::from(edge.target.0)].parameters.iter().rev() {
        out.instruction(&Instruction::LocalSet(locals.get(*parameter)));
    }
}

fn emit_false_condition(
    out: &mut Function,
    condition: ValueId,
    rep: super::ir::Rep,
    locals: &LocalAllocation,
) {
    match rep {
        super::ir::Rep::Bool => {
            out.instruction(&Instruction::LocalGet(locals.get(condition)));
            out.instruction(&Instruction::I64Eqz);
        }
        super::ir::Rep::Nil => {
            out.instruction(&Instruction::I32Const(1));
        }
        super::ir::Rep::I64
        | super::ir::Rep::ArrayRef
        | super::ir::Rep::ObjectRef
        | super::ir::Rep::KeyRef
        | super::ir::Rep::TaggedRef
        | super::ir::Rep::TruthyHandle
        | super::ir::Rep::FunctionRef(_) => {
            out.instruction(&Instruction::I32Const(0));
        }
        super::ir::Rep::Unknown => unreachable!("unknown truthiness rejected by SSA"),
    }
}

fn emit_object_value_address(
    out: &mut Function,
    object: ValueId,
    key: ValueId,
    locals: &LocalAllocation,
    cursor: u32,
    address: u32,
) {
    out.instruction(&Instruction::I64Const(0));
    out.instruction(&Instruction::LocalSet(cursor));
    out.instruction(&Instruction::Block(BlockType::Empty));
    out.instruction(&Instruction::Loop(BlockType::Empty));

    out.instruction(&Instruction::LocalGet(cursor));
    out.instruction(&Instruction::LocalGet(locals.get(object)));
    out.instruction(&Instruction::I32WrapI64);
    out.instruction(&Instruction::I64Load(I64_MEMORY));
    out.instruction(&Instruction::I64GeU);
    out.instruction(&Instruction::If(BlockType::Empty));
    emit_error(out, ERROR_OBJECT_KEY);
    out.instruction(&Instruction::End);

    out.instruction(&Instruction::LocalGet(locals.get(object)));
    out.instruction(&Instruction::I32WrapI64);
    out.instruction(&Instruction::LocalGet(cursor));
    out.instruction(&Instruction::I32WrapI64);
    out.instruction(&Instruction::I32Const(16));
    out.instruction(&Instruction::I32Mul);
    out.instruction(&Instruction::I32Add);
    out.instruction(&Instruction::I64Load(MemArg {
        offset: 8,
        ..I64_MEMORY
    }));
    out.instruction(&Instruction::LocalGet(locals.get(key)));
    out.instruction(&Instruction::I64Eq);
    out.instruction(&Instruction::If(BlockType::Empty));
    out.instruction(&Instruction::LocalGet(locals.get(object)));
    out.instruction(&Instruction::LocalGet(cursor));
    out.instruction(&Instruction::I64Const(16));
    out.instruction(&Instruction::I64Mul);
    out.instruction(&Instruction::I64Add);
    out.instruction(&Instruction::I64Const(16));
    out.instruction(&Instruction::I64Add);
    out.instruction(&Instruction::LocalSet(address));
    out.instruction(&Instruction::Br(2));
    out.instruction(&Instruction::End);

    out.instruction(&Instruction::LocalGet(cursor));
    out.instruction(&Instruction::I64Const(1));
    out.instruction(&Instruction::I64Add);
    out.instruction(&Instruction::LocalSet(cursor));
    out.instruction(&Instruction::Br(0));
    out.instruction(&Instruction::End);
    out.instruction(&Instruction::End);
    out.instruction(&Instruction::LocalGet(address));
    out.instruction(&Instruction::I32WrapI64);
}

#[cfg(test)]
mod allocation_tests {
    use super::*;
    use crate::vm::compile_source;

    #[test]
    fn colors_non_overlapping_ssa_values_and_preserves_abi_parameters() {
        let program = lower_program(
            &compile_source("(loop [i 0 acc 0] (if (< i 20) (recur (+ i 1) (+ acc i)) acc))")
                .unwrap(),
        )
        .unwrap();
        let function = &program.functions[usize::from(program.entry)];
        let allocation = allocate_locals(function);
        assert!(allocation.count < function.value_count);
        for parameter in 0..u32::from(function.arity) {
            assert_eq!(allocation.values[parameter as usize], parameter);
        }
    }

    #[test]
    fn selects_forward_and_natural_regions_but_keeps_nested_loop_fallback() {
        let conditional = lower_program(&compile_source("(if (< 1 2) 19 23)").unwrap()).unwrap();
        assert!(is_forward_cfg(&conditional.functions[0]));
        let looped =
            lower_program(&compile_source("(loop [i 0] (if (< i 2) (recur (+ i 1)) i))").unwrap())
                .unwrap();
        assert!(matches!(
            control_shape(&looped.functions[0]),
            ControlShape::NaturalLoop(_)
        ));
        let branchy = lower_program(&compile_source("(loop [i 0 acc 0] (if (< i 20) (recur (+ i 1) (if (= (mod i 3) 0) (+ acc (* i 3)) (- acc (mod i 11)))) acc))").unwrap()).unwrap();
        assert!(matches!(
            control_shape(&branchy.functions[0]),
            ControlShape::NaturalLoop(_)
        ));
        let nested = lower_program(&compile_source("(loop [i 0 acc 0] (if (< i 2) (let [next (loop [j 0 a acc] (if (< j 2) (recur (+ j 1) (+ a j)) a))] (recur (+ i 1) next)) acc))").unwrap()).unwrap();
        assert_eq!(
            control_shape(&nested.functions[0]),
            ControlShape::Dispatcher
        );
    }
}
