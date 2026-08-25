use std::collections::BTreeSet;

use crate::vm::{FunctionId, Program};

use super::ir::{self, MirOp, MirTerminator, Op, Rep};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ValueId(pub u32);

impl From<ValueId> for u32 {
    fn from(value: ValueId) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlockId(pub u16);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SsaEdge {
    pub target: BlockId,
    pub arguments: Vec<ValueId>,
}

pub type SsaOp = Op<ValueId>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SsaTerminator {
    Goto(SsaEdge),
    BranchZero {
        condition: ValueId,
        rep: Rep,
        zero: SsaEdge,
        nonzero: SsaEdge,
    },
    Return(ValueId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SsaBlock {
    pub id: BlockId,
    pub parameters: Vec<ValueId>,
    pub operations: Vec<SsaOp>,
    pub terminator: SsaTerminator,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SsaFunction {
    pub id: FunctionId,
    pub name: Option<String>,
    pub arity: u16,
    pub value_count: u32,
    /// Point-sensitive representation facts indexed by `ValueId`.
    pub representations: Vec<Rep>,
    pub blocks: Vec<SsaBlock>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SsaProgram {
    pub entry: FunctionId,
    pub functions: Vec<SsaFunction>,
}

/// Lowers validated bytecode through the private slot CFG into deterministic
/// block-argument SSA. Every slot is initially carried on every edge; later
/// liveness passes can trim this conservative representation without changing
/// the public IR contract.
pub fn lower_program(program: &Program) -> Result<SsaProgram, String> {
    let slot_program = ir::lower_slot_program(program)?;
    let functions = slot_program
        .functions
        .iter()
        .map(lower_function)
        .collect::<Result<Vec<_>, _>>()?;
    let program = SsaProgram {
        entry: slot_program.entry,
        functions,
    };
    verify(&program)?;
    Ok(program)
}

fn lower_function(function: &ir::MirFunction) -> Result<SsaFunction, String> {
    let slot_count = function
        .local_count
        .checked_add(function.stack_count)
        .ok_or("whole-Wasm SSA slot count overflow")?;
    let parameter_slots = live_block_parameters(function, slot_count);
    let mut next = 0u32;
    let mut parameters = Vec::with_capacity(function.blocks.len());
    for slots in &parameter_slots {
        let block_parameters = slots.iter().map(|_| fresh(&mut next)).collect::<Vec<_>>();
        parameters.push(block_parameters);
    }
    let mut blocks = Vec::with_capacity(function.blocks.len());
    for (block_index, block) in function.blocks.iter().enumerate() {
        let mut slots = vec![None; usize::from(slot_count)];
        for (slot, parameter) in parameter_slots[block_index]
            .iter()
            .zip(&parameters[block_index])
        {
            slots[usize::from(*slot)] = Some(*parameter);
        }
        let mut operations = Vec::with_capacity(block.operations.len());
        for operation in &block.operations {
            let destination = destination(operation);
            let result = fresh(&mut next);
            let mapped = map_operation(
                operation,
                |slot| slots[usize::from(slot)].expect("live SSA operand must have a definition"),
                result,
            );
            slots[usize::from(destination)] = Some(result);
            operations.push(mapped);
        }
        let edge = |target: u16| -> Result<SsaEdge, String> {
            if usize::from(target) >= parameters.len() {
                return Err(format!("whole-Wasm SSA has invalid block target {target}"));
            }
            Ok(SsaEdge {
                target: BlockId(target),
                arguments: parameter_slots[usize::from(target)]
                    .iter()
                    .map(|slot| {
                        slots[usize::from(*slot)]
                            .expect("live SSA edge argument must have a definition")
                    })
                    .collect(),
            })
        };
        let terminator = match block.terminator {
            MirTerminator::Goto(target) => SsaTerminator::Goto(edge(target)?),
            MirTerminator::BranchZero {
                condition,
                rep,
                zero,
                nonzero,
            } => SsaTerminator::BranchZero {
                condition: slots[usize::from(condition)]
                    .expect("live SSA condition must have a definition"),
                rep,
                zero: edge(zero)?,
                nonzero: edge(nonzero)?,
            },
            MirTerminator::Return(value) => SsaTerminator::Return(
                slots[usize::from(value)].expect("live SSA return must have a definition"),
            ),
        };
        blocks.push(SsaBlock {
            id: BlockId(block.id),
            parameters: parameters[block_index].clone(),
            operations,
            terminator,
        });
    }
    let mut lowered = SsaFunction {
        id: function.id,
        name: function.name.clone(),
        arity: function.arity,
        value_count: next,
        representations: vec![Rep::Unknown; next as usize],
        blocks,
    };
    infer_representations(&mut lowered);
    Ok(lowered)
}

fn infer_representations(function: &mut SsaFunction) {
    loop {
        let previous = function.representations.clone();
        for block in &function.blocks {
            for operation in &block.operations {
                let representation = operation_rep(operation, &previous);
                function.representations[result(operation).0 as usize] = representation;
            }
            for edge in terminator_edges(&block.terminator) {
                let target = &function.blocks[usize::from(edge.target.0)];
                for (parameter, argument) in target.parameters.iter().zip(&edge.arguments) {
                    let incoming = previous[argument.0 as usize];
                    let current = function.representations[parameter.0 as usize];
                    function.representations[parameter.0 as usize] = join_rep(current, incoming);
                }
            }
        }
        if function.representations == previous {
            break;
        }
    }
}

fn operation_rep(operation: &SsaOp, reps: &[Rep]) -> Rep {
    let value_rep = |value: ValueId| reps[value.0 as usize];
    match operation {
        Op::Constant { rep, .. } => *rep,
        Op::ConstantHandle { .. } => Rep::Unknown,
        Op::BoxI64 { .. } => Rep::TruthyHandle,
        Op::UnboxI64 { .. } | Op::TaggedUnboxI64 { .. } => Rep::I64,
        Op::Move { source, .. } => value_rep(*source),
        Op::Binary { op, .. } | Op::BinaryConstant { op, .. } => match op {
            crate::core::IntrinsicOp::Add
            | crate::core::IntrinsicOp::Subtract
            | crate::core::IntrinsicOp::Multiply
            | crate::core::IntrinsicOp::Divide
            | crate::core::IntrinsicOp::Remainder
            | crate::core::IntrinsicOp::Modulo => Rep::I64,
            _ => Rep::Bool,
        },
        Op::ArrayNew { .. } | Op::ArraySetI64 { .. } => Rep::ArrayRef,
        Op::ArrayGetI64 { .. }
        | Op::ArrayGetI64Constant { .. }
        | Op::ObjectGetI64 { .. }
        | Op::GetI64 { .. }
        | Op::GetPathI64Constants { .. }
        | Op::Count { .. }
        | Op::TaggedCount { .. } => Rep::I64,
        Op::ObjectNew { .. } | Op::ObjectSetI64 { .. } => Rep::ObjectRef,
        Op::NativeVector { .. } | Op::TaggedNth { .. } => Rep::TaggedRef,
        Op::BuildVector { .. }
        | Op::BuildMap { .. }
        | Op::BuildMapI64Pair { .. }
        | Op::Assoc { .. }
        | Op::AssocMapI64Pair { .. } => Rep::TruthyHandle,
        Op::IsNumber { .. } | Op::TaggedIsNumber { .. } => Rep::Bool,
        Op::Get { .. } | Op::Nth { .. } | Op::CallStatic { .. } => Rep::Unknown,
    }
}

fn join_rep(current: Rep, incoming: Rep) -> Rep {
    if current == Rep::Unknown {
        incoming
    } else if incoming == Rep::Unknown || current == incoming {
        current
    } else {
        Rep::Unknown
    }
}

fn terminator_edges(terminator: &SsaTerminator) -> Vec<&SsaEdge> {
    match terminator {
        SsaTerminator::Goto(edge) => vec![edge],
        SsaTerminator::BranchZero { zero, nonzero, .. } => vec![zero, nonzero],
        SsaTerminator::Return(_) => Vec::new(),
    }
}

fn live_block_parameters(function: &ir::MirFunction, slot_count: u16) -> Vec<Vec<u16>> {
    let count = function.blocks.len();
    let mut uses = vec![BTreeSet::new(); count];
    let mut definitions = vec![BTreeSet::new(); count];
    for (index, block) in function.blocks.iter().enumerate() {
        for operation in &block.operations {
            for source in source_slots(operation) {
                if !definitions[index].contains(&source) {
                    uses[index].insert(source);
                }
            }
            definitions[index].insert(destination(operation));
        }
        let terminator_source = match block.terminator {
            MirTerminator::Goto(_) => None,
            MirTerminator::BranchZero { condition, .. } => Some(condition),
            MirTerminator::Return(value) => Some(value),
        };
        if let Some(source) = terminator_source {
            if !definitions[index].contains(&source) {
                uses[index].insert(source);
            }
        }
    }
    let mut live_in = uses.clone();
    loop {
        let mut changed = false;
        for index in (0..count).rev() {
            let mut incoming = uses[index].clone();
            for successor in successors(&function.blocks[index].terminator) {
                for slot in &live_in[usize::from(successor)] {
                    if !definitions[index].contains(slot) {
                        incoming.insert(*slot);
                    }
                }
            }
            if incoming != live_in[index] {
                live_in[index] = incoming;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    for argument in 0..function.arity {
        live_in[0].insert(argument);
    }
    debug_assert!(live_in.iter().flatten().all(|slot| *slot < slot_count));
    live_in
        .into_iter()
        .map(|slots| slots.into_iter().collect())
        .collect()
}

fn successors(terminator: &MirTerminator) -> Vec<u16> {
    match terminator {
        MirTerminator::Goto(target) => vec![*target],
        MirTerminator::BranchZero { zero, nonzero, .. } => vec![*zero, *nonzero],
        MirTerminator::Return(_) => Vec::new(),
    }
}

fn source_slots(operation: &MirOp) -> Vec<u16> {
    let mut values = Vec::new();
    match operation {
        Op::Constant { .. } | Op::ConstantHandle { .. } => {}
        Op::BoxI64 { source, .. }
        | Op::UnboxI64 { source, .. }
        | Op::Move { source, .. }
        | Op::TaggedUnboxI64 { source, .. } => values.push(*source),
        Op::Binary { left, right, .. } => values.extend([*left, *right]),
        Op::BinaryConstant { left, .. } => values.push(*left),
        Op::ArrayNew { values: items, .. } | Op::BuildVector { values: items, .. } => {
            values.extend(items)
        }
        Op::NativeVector { values: items, .. } => {
            values.extend(items.iter().map(|(value, _)| value))
        }
        Op::ArrayGetI64 { array, index, .. } => values.extend([array, index]),
        Op::ArrayGetI64Constant { array, .. } => values.push(*array),
        Op::ArraySetI64 {
            array,
            index,
            value,
            ..
        } => values.extend([array, index, value]),
        Op::ObjectNew { entries, .. } | Op::BuildMap { entries, .. } => {
            values.extend(entries.iter().flat_map(|(key, value)| [key, value]))
        }
        Op::ObjectGetI64 { object, key, .. } => values.extend([object, key]),
        Op::ObjectSetI64 {
            object, key, value, ..
        } => values.extend([object, key, value]),
        Op::BuildMapI64Pair { key, value, .. } => values.extend([key, value]),
        Op::Assoc {
            collection,
            key,
            value,
            ..
        } => values.extend([collection, key, value]),
        Op::AssocMapI64Pair {
            collection,
            outer_key,
            inner_key,
            value,
            ..
        } => values.extend([collection, outer_key, inner_key, value]),
        Op::Get {
            collection, key, ..
        }
        | Op::GetI64 {
            collection, key, ..
        } => values.extend([collection, key]),
        Op::GetPathI64Constants { collection, .. }
        | Op::Count { collection, .. }
        | Op::TaggedCount { collection, .. } => values.push(*collection),
        Op::IsNumber { value, .. } | Op::TaggedIsNumber { value, .. } => values.push(*value),
        Op::Nth {
            collection, index, ..
        }
        | Op::TaggedNth {
            collection, index, ..
        } => values.extend([collection, index]),
        Op::CallStatic { arguments, .. } => values.extend(arguments),
    }
    values
}

fn fresh(next: &mut u32) -> ValueId {
    let value = ValueId(*next);
    *next = next.checked_add(1).expect("whole-Wasm SSA value overflow");
    value
}

fn destination<V: Copy>(operation: &Op<V>) -> V {
    match operation {
        Op::Constant { destination, .. }
        | Op::ConstantHandle { destination, .. }
        | Op::BoxI64 { destination, .. }
        | Op::UnboxI64 { destination, .. }
        | Op::Move { destination, .. }
        | Op::Binary { destination, .. }
        | Op::BinaryConstant { destination, .. }
        | Op::ArrayNew { destination, .. }
        | Op::ArrayGetI64 { destination, .. }
        | Op::ArrayGetI64Constant { destination, .. }
        | Op::ArraySetI64 { destination, .. }
        | Op::ObjectNew { destination, .. }
        | Op::ObjectGetI64 { destination, .. }
        | Op::ObjectSetI64 { destination, .. }
        | Op::BuildVector { destination, .. }
        | Op::NativeVector { destination, .. }
        | Op::BuildMap { destination, .. }
        | Op::BuildMapI64Pair { destination, .. }
        | Op::Assoc { destination, .. }
        | Op::AssocMapI64Pair { destination, .. }
        | Op::Get { destination, .. }
        | Op::GetI64 { destination, .. }
        | Op::GetPathI64Constants { destination, .. }
        | Op::IsNumber { destination, .. }
        | Op::TaggedIsNumber { destination, .. }
        | Op::Count { destination, .. }
        | Op::TaggedCount { destination, .. }
        | Op::Nth { destination, .. }
        | Op::TaggedNth { destination, .. }
        | Op::TaggedUnboxI64 { destination, .. }
        | Op::CallStatic { destination, .. } => *destination,
    }
}

fn map_operation<F>(operation: &MirOp, mut value: F, result: ValueId) -> SsaOp
where
    F: FnMut(u16) -> ValueId,
{
    macro_rules! unary {
        ($variant:ident, $source:ident) => {
            Op::$variant {
                destination: result,
                source: value(*$source),
            }
        };
    }
    match operation {
        Op::Constant {
            value: scalar, rep, ..
        } => Op::Constant {
            destination: result,
            value: *scalar,
            rep: *rep,
        },
        Op::ConstantHandle { constant, .. } => Op::ConstantHandle {
            destination: result,
            constant: *constant,
        },
        Op::BoxI64 { source, .. } => unary!(BoxI64, source),
        Op::UnboxI64 { source, .. } => unary!(UnboxI64, source),
        Op::Move { source, .. } => unary!(Move, source),
        Op::Binary {
            left, right, op, ..
        } => Op::Binary {
            destination: result,
            left: value(*left),
            right: value(*right),
            op: *op,
        },
        Op::BinaryConstant {
            left, right, op, ..
        } => Op::BinaryConstant {
            destination: result,
            left: value(*left),
            right: *right,
            op: *op,
        },
        Op::ArrayNew { values, .. } => Op::ArrayNew {
            destination: result,
            values: values.iter().copied().map(&mut value).collect(),
        },
        Op::ArrayGetI64 { array, index, .. } => Op::ArrayGetI64 {
            destination: result,
            array: value(*array),
            index: value(*index),
        },
        Op::ArrayGetI64Constant { array, index, .. } => Op::ArrayGetI64Constant {
            destination: result,
            array: value(*array),
            index: *index,
        },
        Op::ArraySetI64 {
            array,
            index,
            value: item,
            ..
        } => Op::ArraySetI64 {
            destination: result,
            array: value(*array),
            index: value(*index),
            value: value(*item),
        },
        Op::ObjectNew { entries, .. } => Op::ObjectNew {
            destination: result,
            entries: entries
                .iter()
                .map(|(key, item)| (value(*key), value(*item)))
                .collect(),
        },
        Op::ObjectGetI64 { object, key, .. } => Op::ObjectGetI64 {
            destination: result,
            object: value(*object),
            key: value(*key),
        },
        Op::ObjectSetI64 {
            object,
            key,
            value: item,
            ..
        } => Op::ObjectSetI64 {
            destination: result,
            object: value(*object),
            key: value(*key),
            value: value(*item),
        },
        Op::BuildVector { values, .. } => Op::BuildVector {
            destination: result,
            values: values.iter().copied().map(&mut value).collect(),
        },
        Op::NativeVector { values, .. } => Op::NativeVector {
            destination: result,
            values: values
                .iter()
                .map(|(slot, rep)| (value(*slot), *rep))
                .collect(),
        },
        Op::BuildMap { entries, .. } => Op::BuildMap {
            destination: result,
            entries: entries
                .iter()
                .map(|(key, item)| (value(*key), value(*item)))
                .collect(),
        },
        Op::BuildMapI64Pair {
            key, value: item, ..
        } => Op::BuildMapI64Pair {
            destination: result,
            key: value(*key),
            value: value(*item),
        },
        Op::Assoc {
            collection,
            key,
            value: item,
            ..
        } => Op::Assoc {
            destination: result,
            collection: value(*collection),
            key: value(*key),
            value: value(*item),
        },
        Op::AssocMapI64Pair {
            collection,
            outer_key,
            inner_key,
            value: item,
            ..
        } => Op::AssocMapI64Pair {
            destination: result,
            collection: value(*collection),
            outer_key: value(*outer_key),
            inner_key: value(*inner_key),
            value: value(*item),
        },
        Op::Get {
            collection, key, ..
        } => Op::Get {
            destination: result,
            collection: value(*collection),
            key: value(*key),
        },
        Op::GetI64 {
            collection, key, ..
        } => Op::GetI64 {
            destination: result,
            collection: value(*collection),
            key: value(*key),
        },
        Op::GetPathI64Constants {
            collection,
            first_key,
            second_key,
            ..
        } => Op::GetPathI64Constants {
            destination: result,
            collection: value(*collection),
            first_key: *first_key,
            second_key: *second_key,
        },
        Op::IsNumber { value: item, .. } => Op::IsNumber {
            destination: result,
            value: value(*item),
        },
        Op::TaggedIsNumber { value: item, .. } => Op::TaggedIsNumber {
            destination: result,
            value: value(*item),
        },
        Op::Count { collection, .. } => Op::Count {
            destination: result,
            collection: value(*collection),
        },
        Op::TaggedCount { collection, .. } => Op::TaggedCount {
            destination: result,
            collection: value(*collection),
        },
        Op::Nth {
            collection, index, ..
        } => Op::Nth {
            destination: result,
            collection: value(*collection),
            index: value(*index),
        },
        Op::TaggedNth {
            collection, index, ..
        } => Op::TaggedNth {
            destination: result,
            collection: value(*collection),
            index: value(*index),
        },
        Op::TaggedUnboxI64 { source, .. } => unary!(TaggedUnboxI64, source),
        Op::CallStatic {
            function,
            arguments,
            ..
        } => Op::CallStatic {
            destination: result,
            function: *function,
            arguments: arguments.iter().copied().map(&mut value).collect(),
        },
    }
}

pub fn verify(program: &SsaProgram) -> Result<(), String> {
    if program.functions.is_empty() || usize::from(program.entry) >= program.functions.len() {
        return Err("whole-Wasm SSA has an invalid entry".into());
    }
    for (expected_function, function) in program.functions.iter().enumerate() {
        if usize::from(function.id) != expected_function || function.blocks.is_empty() {
            return Err(format!(
                "whole-Wasm SSA has invalid function {}",
                function.id
            ));
        }
        if function.representations.len() != function.value_count as usize {
            return Err(format!(
                "whole-Wasm SSA function {} has invalid representation facts",
                function.id
            ));
        }
        let mut definitions = BTreeSet::new();
        for (expected_block, block) in function.blocks.iter().enumerate() {
            if usize::from(block.id.0) != expected_block {
                return Err(format!(
                    "whole-Wasm SSA function {} has non-dense blocks",
                    function.id
                ));
            }
            let mut available = BTreeSet::new();
            for parameter in &block.parameters {
                define(function, &mut definitions, &mut available, *parameter)?;
            }
            for operation in &block.operations {
                for operand in operands(operation) {
                    require_available(function, &available, operand)?;
                }
                define(
                    function,
                    &mut definitions,
                    &mut available,
                    result(operation),
                )?;
                if let Op::CallStatic {
                    function: target,
                    arguments,
                    ..
                } = operation
                {
                    let Some(callee) = program.functions.get(usize::from(*target)) else {
                        return Err(format!("whole-Wasm SSA call has invalid target {target}"));
                    };
                    if arguments.len() != usize::from(callee.arity) {
                        return Err(format!("whole-Wasm SSA call to {target} has invalid arity"));
                    }
                }
            }
            match &block.terminator {
                SsaTerminator::Goto(edge) => verify_edge(function, &available, edge)?,
                SsaTerminator::BranchZero {
                    condition,
                    rep,
                    zero,
                    nonzero,
                } => {
                    if *rep == Rep::Unknown {
                        return Err(format!(
                            "whole-Wasm SSA function {} branches on an unknown representation",
                            function.id
                        ));
                    }
                    require_available(function, &available, *condition)?;
                    verify_edge(function, &available, zero)?;
                    verify_edge(function, &available, nonzero)?;
                }
                SsaTerminator::Return(value) => require_available(function, &available, *value)?,
            }
        }
        if definitions.len() != function.value_count as usize {
            return Err(format!(
                "whole-Wasm SSA function {} has non-dense values",
                function.id
            ));
        }
        if function.blocks[0].parameters.len() < usize::from(function.arity)
            || function.blocks[0].parameters[..usize::from(function.arity)]
                .iter()
                .enumerate()
                .any(|(id, value)| value.0 != id as u32)
        {
            return Err(format!(
                "whole-Wasm SSA function {} has invalid parameters",
                function.id
            ));
        }
    }
    Ok(())
}

fn define(
    function: &SsaFunction,
    definitions: &mut BTreeSet<ValueId>,
    available: &mut BTreeSet<ValueId>,
    value: ValueId,
) -> Result<(), String> {
    if value.0 >= function.value_count || !definitions.insert(value) {
        return Err(format!(
            "whole-Wasm SSA function {} defines value {} more than once",
            function.id, value.0
        ));
    }
    available.insert(value);
    Ok(())
}

fn require_available(
    function: &SsaFunction,
    available: &BTreeSet<ValueId>,
    value: ValueId,
) -> Result<(), String> {
    if !available.contains(&value) {
        return Err(format!(
            "whole-Wasm SSA function {} uses unavailable value {}",
            function.id, value.0
        ));
    }
    Ok(())
}

fn verify_edge(
    function: &SsaFunction,
    available: &BTreeSet<ValueId>,
    edge: &SsaEdge,
) -> Result<(), String> {
    let Some(target) = function.blocks.get(usize::from(edge.target.0)) else {
        return Err(format!(
            "whole-Wasm SSA has invalid target {}",
            edge.target.0
        ));
    };
    if edge.arguments.len() != target.parameters.len() {
        return Err(format!(
            "whole-Wasm SSA edge to {} has invalid arity",
            edge.target.0
        ));
    }
    for argument in &edge.arguments {
        require_available(function, available, *argument)?;
    }
    Ok(())
}

pub(crate) fn result(operation: &SsaOp) -> ValueId {
    destination(operation)
}

pub(crate) fn operands(operation: &SsaOp) -> Vec<ValueId> {
    let mut values = Vec::new();
    match operation {
        Op::Constant { .. } | Op::ConstantHandle { .. } => {}
        Op::BoxI64 { source, .. }
        | Op::UnboxI64 { source, .. }
        | Op::Move { source, .. }
        | Op::TaggedUnboxI64 { source, .. } => values.push(*source),
        Op::Binary { left, right, .. } => values.extend([*left, *right]),
        Op::BinaryConstant { left, .. } => values.push(*left),
        Op::ArrayNew { values: items, .. } | Op::BuildVector { values: items, .. } => {
            values.extend(items)
        }
        Op::NativeVector { values: items, .. } => {
            values.extend(items.iter().map(|(value, _)| value))
        }
        Op::ArrayGetI64 { array, index, .. } => values.extend([array, index]),
        Op::ArrayGetI64Constant { array, .. } => values.push(*array),
        Op::ArraySetI64 {
            array,
            index,
            value,
            ..
        } => values.extend([array, index, value]),
        Op::ObjectNew { entries, .. } | Op::BuildMap { entries, .. } => {
            values.extend(entries.iter().flat_map(|(key, value)| [key, value]))
        }
        Op::ObjectGetI64 { object, key, .. } => values.extend([object, key]),
        Op::ObjectSetI64 {
            object, key, value, ..
        } => values.extend([object, key, value]),
        Op::BuildMapI64Pair { key, value, .. } => values.extend([key, value]),
        Op::Assoc {
            collection,
            key,
            value,
            ..
        } => values.extend([collection, key, value]),
        Op::AssocMapI64Pair {
            collection,
            outer_key,
            inner_key,
            value,
            ..
        } => values.extend([collection, outer_key, inner_key, value]),
        Op::Get {
            collection, key, ..
        }
        | Op::GetI64 {
            collection, key, ..
        } => values.extend([collection, key]),
        Op::GetPathI64Constants { collection, .. }
        | Op::Count { collection, .. }
        | Op::TaggedCount { collection, .. } => values.push(*collection),
        Op::IsNumber { value, .. } | Op::TaggedIsNumber { value, .. } => values.push(*value),
        Op::Nth {
            collection, index, ..
        }
        | Op::TaggedNth {
            collection, index, ..
        } => values.extend([collection, index]),
        Op::CallStatic { arguments, .. } => values.extend(arguments),
    }
    values
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::compile_source;

    #[test]
    fn lowering_is_deterministic_and_assigns_dense_values() {
        let bytecode =
            compile_source("(loop [i 0 acc 0] (if (< i 10) (recur (+ i 1) (+ acc i)) acc))")
                .unwrap();
        let first = lower_program(&bytecode).unwrap();
        assert_eq!(first, lower_program(&bytecode).unwrap());
        let function = &first.functions[usize::from(first.entry)];
        assert!(function.blocks.len() > 1);
        let definitions = function
            .blocks
            .iter()
            .flat_map(|block| {
                block
                    .parameters
                    .iter()
                    .copied()
                    .chain(block.operations.iter().map(result))
            })
            .map(|value| value.0)
            .collect::<BTreeSet<_>>();
        assert_eq!(definitions, (0..function.value_count).collect());
    }

    #[test]
    fn verifier_rejects_duplicate_definitions_and_bad_edge_arity() {
        let bytecode = compile_source("(loop [i 0] (if (< i 2) (recur (+ i 1)) i))").unwrap();
        let mut duplicate = lower_program(&bytecode).unwrap();
        let function = &mut duplicate.functions[0];
        function.blocks[1].parameters[0] = result(&function.blocks[0].operations[0]);
        assert!(verify(&duplicate).unwrap_err().contains("more than once"));

        let mut bad_edge = lower_program(&bytecode).unwrap();
        let terminator = &mut bad_edge.functions[0].blocks[0].terminator;
        let edge = match terminator {
            SsaTerminator::Goto(edge) => edge,
            SsaTerminator::BranchZero { zero, .. } => zero,
            SsaTerminator::Return(_) => panic!("loop entry must have an edge"),
        };
        edge.arguments.pop();
        assert!(verify(&bad_edge).unwrap_err().contains("invalid arity"));
    }
}
