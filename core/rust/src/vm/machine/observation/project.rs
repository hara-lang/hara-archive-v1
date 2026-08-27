use super::*;

pub(super) fn slot_head(
    values: &[VmSlot],
    limit: usize,
    display_chars: usize,
) -> (Vec<ValueSnapshot>, usize) {
    let retained = values.len().min(limit);
    (
        values[..retained]
            .iter()
            .map(|value| slot_snapshot(value, display_chars))
            .collect(),
        values.len().saturating_sub(retained),
    )
}

pub(super) fn slot_tail(
    values: &[VmSlot],
    limit: usize,
    display_chars: usize,
) -> (Vec<ValueSnapshot>, usize) {
    let start = values.len().saturating_sub(limit);
    (
        values[start..]
            .iter()
            .map(|value| slot_snapshot(value, display_chars))
            .collect(),
        start,
    )
}

pub(super) fn slot_snapshot(value: &VmSlot, display_chars: usize) -> ValueSnapshot {
    match value {
        VmSlot::Number(value) => bounded_value("number", value.to_string(), display_chars),
        VmSlot::Bool(value) => bounded_value("boolean", value.to_string(), display_chars),
        VmSlot::Nil => bounded_value("nil", "nil".to_string(), display_chars),
        VmSlot::Value(value) => value_snapshot(value, display_chars),
        VmSlot::InlineClosure {
            prototype,
            identity,
        } => bounded_value(
            "closure",
            format!("<closure prototype={prototype} identity={identity}>"),
            display_chars,
        ),
        VmSlot::Closure(closure) => bounded_value(
            "closure",
            format!(
                "<closure prototype={} captures={}",
                closure.prototype,
                closure.captures.len()
            ) + ">",
            display_chars,
        ),
        VmSlot::MultiArity(dispatch) => bounded_value(
            "multi-arity",
            format!(
                "<multi-arity {} clauses={}",
                dispatch.name,
                dispatch.clauses.len()
            ) + ">",
            display_chars,
        ),
    }
}

pub(super) fn value_snapshot(value: &Value, display_chars: usize) -> ValueSnapshot {
    let kind = match value {
        Value::Number(_) => "number",
        Value::Bool(_) => "boolean",
        Value::Nil => "nil",
        Value::String(_) => "string",
        Value::Promise(_) => "promise",
        _ => "value",
    };
    bounded_value(kind, value.display(), display_chars)
}

pub(super) fn bounded_value(kind: &'static str, display: String, limit: usize) -> ValueSnapshot {
    let mut chars = display.chars();
    let mut bounded: String = chars.by_ref().take(limit).collect();
    let truncated = chars.next().is_some();
    if truncated {
        bounded.push('…');
    }
    ValueSnapshot {
        kind,
        display: bounded,
        truncated,
    }
}

pub(super) fn position_snapshot(position: Position) -> SourcePositionSnapshot {
    SourcePositionSnapshot {
        offset: position.offset,
        line: position.line,
        column: position.column,
    }
}

pub(super) fn instruction_snapshot(instruction: &Instruction) -> InstructionSnapshot {
    use InstructionOperand::Unsigned;

    let (opcode, operands) = match instruction {
        Instruction::Constant(index) => ("constant", vec![Unsigned(*index as u64)]),
        Instruction::Nil => ("nil", vec![]),
        Instruction::True => ("true", vec![]),
        Instruction::False => ("false", vec![]),
        Instruction::LoadLocal(slot) => ("load-local", vec![Unsigned(*slot as u64)]),
        Instruction::StoreLocal(slot) => ("store-local", vec![Unsigned(*slot as u64)]),
        Instruction::Pop => ("pop", vec![]),
        Instruction::Dup => ("dup", vec![]),
        Instruction::IntrinsicCall { target, argc } => (
            "intrinsic-call",
            vec![Unsigned(*target as u64), Unsigned(*argc as u64)],
        ),
        Instruction::Jump(target) => ("jump", vec![Unsigned(*target as u64)]),
        Instruction::JumpIfFalse(target) => ("jump-if-false", vec![Unsigned(*target as u64)]),
        Instruction::Closure {
            prototype,
            captures,
        } => (
            "closure",
            vec![Unsigned(*prototype as u64), Unsigned(*captures as u64)],
        ),
        Instruction::Call { argc } => ("call", vec![Unsigned(*argc as u64)]),
        Instruction::CallStatic { prototype, argc } => (
            "call-static",
            vec![Unsigned(*prototype as u64), Unsigned(*argc as u64)],
        ),
        Instruction::Throw => ("throw", vec![]),
        Instruction::Rethrow => ("rethrow", vec![]),
        Instruction::GetGlobal(index) => ("get-global", vec![Unsigned(*index as u64)]),
        Instruction::DefGlobal { name, metadata } => {
            let mut operands = vec![Unsigned(*name as u64)];
            if let Some(metadata) = metadata {
                operands.push(Unsigned(*metadata as u64));
            }
            ("def-global", operands)
        }
        Instruction::SetGlobal(index) => ("set-global", vec![Unsigned(*index as u64)]),
        Instruction::VarGlobal(index) => ("var-global", vec![Unsigned(*index as u64)]),
        Instruction::DeclareGlobal(index) => ("declare-global", vec![Unsigned(*index as u64)]),
        Instruction::DefStruct { name, fields } => (
            "def-struct",
            vec![Unsigned(*name as u64), Unsigned(*fields as u64)],
        ),
        Instruction::DefMutable { name, fields } => (
            "def-mutable",
            vec![Unsigned(*name as u64), Unsigned(*fields as u64)],
        ),
        Instruction::MutableFieldGet(index) => ("mutable-field-get", vec![Unsigned(*index as u64)]),
        Instruction::MutableFieldSet(index) => ("mutable-field-set", vec![Unsigned(*index as u64)]),
        Instruction::InstanceOf => ("instance-of", vec![]),
        Instruction::MakeMultiArity { name, count } => (
            "make-multi-arity",
            vec![Unsigned(*name as u64), Unsigned(*count as u64)],
        ),
        Instruction::BuildVector(count) => ("build-vector", vec![Unsigned(*count as u64)]),
        Instruction::BuildMap(pairs) => ("build-map", vec![Unsigned(*pairs as u64)]),
        Instruction::BuildSet(count) => ("build-set", vec![Unsigned(*count as u64)]),
        Instruction::BuildList(count) => ("build-list", vec![Unsigned(*count as u64)]),
        Instruction::ConcatList(count) => ("concat-list", vec![Unsigned(*count as u64)]),
        Instruction::ToVector => ("to-vector", vec![]),
        Instruction::DefMacro { name, metadata } => {
            let mut operands = vec![Unsigned(*name as u64)];
            if let Some(metadata) = metadata {
                operands.push(Unsigned(*metadata as u64));
            }
            ("def-macro", operands)
        }
        Instruction::DefProtocol(index) => ("def-protocol", vec![Unsigned(*index as u64)]),
        Instruction::ExtendType(index) => ("extend-type", vec![Unsigned(*index as u64)]),
        Instruction::DefMulti(index) => ("def-multi", vec![Unsigned(*index as u64)]),
        Instruction::DefMethod(index) => ("def-method", vec![Unsigned(*index as u64)]),
        Instruction::IntrinsicValue(target) => ("intrinsic-value", vec![Unsigned(*target as u64)]),
        Instruction::BuiltinValue(index) => ("builtin-value", vec![Unsigned(*index as u64)]),
        Instruction::NamespaceValue(index) => ("namespace-value", vec![Unsigned(*index as u64)]),
        Instruction::NamespaceOperation(index) => {
            ("namespace-operation", vec![Unsigned(*index as u64)])
        }
        Instruction::DynamicBind(index) => ("dynamic-bind", vec![Unsigned(*index as u64)]),
        Instruction::DynamicUnbind(index) => ("dynamic-unbind", vec![Unsigned(*index as u64)]),
        Instruction::Await => ("await", vec![]),
        Instruction::HostCall => ("host-call", vec![]),
        Instruction::DotCall { method, argc } => (
            "dot-call",
            vec![Unsigned(*method as u64), Unsigned(*argc as u64)],
        ),
        Instruction::ProtocolCall { target, argc } => (
            "protocol-call",
            vec![Unsigned(*target as u64), Unsigned(*argc as u64)],
        ),
        Instruction::Yield => ("yield", vec![]),
        Instruction::Return => ("return", vec![]),
    };

    InstructionSnapshot {
        opcode,
        operands,
        display: instruction.to_string(),
    }
}
