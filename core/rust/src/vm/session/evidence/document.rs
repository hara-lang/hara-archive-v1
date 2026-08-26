use crate::core::Value;
use crate::lang::data::{OrderedMap, Vector};
use crate::vm::machine::observation::{
    CallFrameSnapshot, HandlerSnapshot, InstructionOperand, InstructionSnapshot, MachineSnapshot,
    SourcePositionSnapshot, ValueSnapshot,
};

use super::{
    CompactEventRecord, SessionMetrics, TraceStepRecord, BYTECODE_EVENTS_SCHEMA,
    BYTECODE_METRICS_SCHEMA, BYTECODE_TRACE_SCHEMA, MAX_SAFE_INTEGER,
};

pub(super) fn metrics_document(
    session_id: &str,
    trace_id: &str,
    sequence: u64,
    status: &str,
    metrics: &SessionMetrics,
) -> Value {
    let opcode_counts = object(
        metrics
            .opcode_counts
            .iter()
            .map(|(opcode, count)| (opcode.clone(), integer(*count))),
    );
    object([
        ("schema", string(BYTECODE_METRICS_SCHEMA)),
        ("sessionId", string(session_id)),
        ("traceId", string(trace_id)),
        ("sequence", integer(sequence)),
        ("status", string(status)),
        ("instructions", integer(metrics.instructions)),
        ("opcodeCounts", opcode_counts),
        ("calls", integer(metrics.calls)),
        ("returns", integer(metrics.returns)),
        ("unwinds", integer(metrics.unwinds)),
        ("suspensions", integer(metrics.suspensions)),
        ("resumptions", integer(metrics.resumptions)),
        ("terminalReturns", integer(metrics.terminal_returns)),
        ("failures", integer(metrics.failures)),
        (
            "maxStackDepth",
            integer(usize_to_u64(metrics.max_stack_depth)),
        ),
        (
            "maxCallDepth",
            integer(usize_to_u64(metrics.max_call_depth)),
        ),
    ])
}

pub(super) fn events_document<'a>(
    session_id: &str,
    trace_id: &str,
    sequence: u64,
    status: &str,
    events: impl IntoIterator<Item = &'a CompactEventRecord>,
    dropped: u64,
) -> Value {
    object([
        ("schema", string(BYTECODE_EVENTS_SCHEMA)),
        ("sessionId", string(session_id)),
        ("traceId", string(trace_id)),
        ("sequence", integer(sequence)),
        ("status", string(status)),
        (
            "events",
            vector(events.into_iter().map(compact_event_value)),
        ),
        ("dropped", integer(dropped)),
    ])
}

pub(super) fn trace_document<'a>(
    session_id: &str,
    trace_id: &str,
    source_id: &str,
    sequence: u64,
    status: &str,
    steps: impl IntoIterator<Item = &'a TraceStepRecord>,
    dropped: u64,
) -> Value {
    object([
        ("schema", string(BYTECODE_TRACE_SCHEMA)),
        ("id", string(trace_id)),
        ("sessionId", string(session_id)),
        ("sourceId", string(source_id)),
        ("sequence", integer(sequence)),
        ("status", string(status)),
        (
            "steps",
            vector(
                steps
                    .into_iter()
                    .map(|step| trace_step_value(step, source_id)),
            ),
        ),
        ("dropped", integer(dropped)),
    ])
}

pub(super) fn snapshot_value(snapshot: &MachineSnapshot, source_id: &str) -> Value {
    object([
        ("program", program_value(snapshot)),
        ("status", string(snapshot.status.as_keyword())),
        ("function", usize_value(snapshot.function)),
        (
            "functionName",
            optional_string(snapshot.function_name.as_deref()),
        ),
        ("functionArity", usize_value(snapshot.function_arity)),
        ("functionVariadic", Value::Bool(snapshot.function_variadic)),
        ("functionCaptures", usize_value(snapshot.function_captures)),
        ("ip", usize_value(snapshot.ip)),
        (
            "instruction",
            optional_value(
                snapshot
                    .instruction
                    .as_ref()
                    .map(instruction_snapshot_value),
            ),
        ),
        (
            "source",
            optional_value(
                snapshot
                    .source
                    .as_ref()
                    .map(|source| source_position_value(source, source_id)),
            ),
        ),
        ("stackBase", usize_value(snapshot.stack_base)),
        (
            "stack",
            vector(snapshot.stack.iter().map(value_snapshot_value)),
        ),
        ("stackOmitted", usize_value(snapshot.stack_omitted)),
        (
            "locals",
            vector(snapshot.locals.iter().map(value_snapshot_value)),
        ),
        ("localsOmitted", usize_value(snapshot.locals_omitted)),
        ("calls", vector(snapshot.calls.iter().map(call_frame_value))),
        ("callsOmitted", usize_value(snapshot.calls_omitted)),
        (
            "handlers",
            vector(snapshot.handlers.iter().map(handler_value)),
        ),
        ("handlersOmitted", usize_value(snapshot.handlers_omitted)),
        (
            "result",
            optional_value(snapshot.result.as_ref().map(value_snapshot_value)),
        ),
        ("error", optional_string(snapshot.error.as_deref())),
    ])
}

fn trace_step_value(step: &TraceStepRecord, source_id: &str) -> Value {
    object([
        ("id", string(step.id.as_str())),
        ("sequence", integer(step.sequence)),
        ("kind", string(step.kind.as_keyword())),
        ("status", string(step.status.as_keyword())),
        ("before", snapshot_value(&step.before, source_id)),
        ("after", snapshot_value(&step.after, source_id)),
        (
            "instruction",
            optional_value(step.instruction.as_ref().map(instruction_snapshot_value)),
        ),
        (
            "source",
            optional_value(
                step.source
                    .as_ref()
                    .map(|source| source_position_value(source, source_id)),
            ),
        ),
        ("error", optional_string(step.error.as_deref())),
    ])
}

fn compact_event_value(event: &CompactEventRecord) -> Value {
    match event {
        CompactEventRecord::Instruction {
            id,
            sequence,
            function,
            ip,
            opcode,
            stack_depth,
            call_depth,
        } => object([
            ("id", string(id.as_str())),
            ("sequence", integer(*sequence)),
            ("kind", string("instruction")),
            ("function", usize_value(*function)),
            ("ip", usize_value(*ip)),
            ("opcode", string(opcode.as_str())),
            ("stackDepth", usize_value(*stack_depth)),
            ("callDepth", usize_value(*call_depth)),
        ]),
        CompactEventRecord::Transition {
            id,
            sequence,
            transition,
            from_function,
            from_ip,
            to_function,
            to_ip,
            stack_depth,
            call_depth,
        } => object([
            ("id", string(id.as_str())),
            ("sequence", integer(*sequence)),
            ("kind", string("transition")),
            ("transition", string(transition)),
            ("fromFunction", usize_value(*from_function)),
            ("fromIp", usize_value(*from_ip)),
            ("toFunction", usize_value(*to_function)),
            ("toIp", usize_value(*to_ip)),
            ("stackDepth", usize_value(*stack_depth)),
            ("callDepth", usize_value(*call_depth)),
        ]),
        CompactEventRecord::Terminal {
            id,
            sequence,
            terminal,
            function,
            ip,
            stack_depth,
            call_depth,
        } => object([
            ("id", string(id.as_str())),
            ("sequence", integer(*sequence)),
            ("kind", string("terminal")),
            ("terminal", string(terminal)),
            ("function", usize_value(*function)),
            ("ip", usize_value(*ip)),
            ("stackDepth", usize_value(*stack_depth)),
            ("callDepth", usize_value(*call_depth)),
        ]),
    }
}

fn program_value(snapshot: &MachineSnapshot) -> Value {
    object([
        ("entry", usize_value(snapshot.program.entry)),
        ("constants", usize_value(snapshot.program.constants)),
        ("functions", usize_value(snapshot.program.functions)),
    ])
}

fn instruction_snapshot_value(instruction: &InstructionSnapshot) -> Value {
    object([
        ("opcode", string(instruction.opcode)),
        (
            "operands",
            vector(instruction.operands.iter().map(instruction_operand_value)),
        ),
        ("display", string(instruction.display.as_str())),
    ])
}

fn instruction_operand_value(operand: &InstructionOperand) -> Value {
    match operand {
        InstructionOperand::Unsigned(value) => integer(*value),
        InstructionOperand::Text(value) => string(value.as_str()),
    }
}

fn source_position_value(source: &SourcePositionSnapshot, source_id: &str) -> Value {
    object([
        ("sourceId", string(source_id)),
        ("offset", usize_value(source.offset)),
        ("line", usize_value(source.line)),
        ("column", usize_value(source.column)),
    ])
}

fn value_snapshot_value(value: &ValueSnapshot) -> Value {
    object([
        ("kind", string(value.kind)),
        ("display", string(value.display.as_str())),
        ("truncated", Value::Bool(value.truncated)),
    ])
}

fn call_frame_value(frame: &CallFrameSnapshot) -> Value {
    object([
        ("function", usize_value(frame.function)),
        ("name", optional_string(frame.name.as_deref())),
        ("callIp", usize_value(frame.call_ip)),
        ("stackBase", usize_value(frame.stack_base)),
    ])
}

fn handler_value(handler: &HandlerSnapshot) -> Value {
    object([
        ("start", usize_value(handler.start)),
        ("end", usize_value(handler.end)),
        ("depth", usize_value(handler.depth)),
        (
            "catches",
            vector(handler.catches.iter().map(|catch| string(catch.as_str()))),
        ),
        ("finally", optional_usize(handler.finally)),
    ])
}

fn object<I, K>(fields: I) -> Value
where
    I: IntoIterator<Item = (K, Value)>,
    K: Into<String>,
{
    Value::OrderedMap(Box::new(OrderedMap::from_iter(
        fields
            .into_iter()
            .map(|(key, value)| (Value::String(key.into()), value)),
    )))
}

fn vector<I>(values: I) -> Value
where
    I: IntoIterator<Item = Value>,
{
    Value::Vector(Vector::from_iter(values))
}

fn string(value: impl Into<String>) -> Value {
    Value::String(value.into())
}

fn integer(value: u64) -> Value {
    Value::Number(value.min(MAX_SAFE_INTEGER) as i64)
}

fn usize_value(value: usize) -> Value {
    integer(usize_to_u64(value))
}

fn usize_to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

fn optional_string(value: Option<&str>) -> Value {
    value.map_or(Value::Nil, |value| string(value))
}

fn optional_usize(value: Option<usize>) -> Value {
    value.map_or(Value::Nil, usize_value)
}

fn optional_value(value: Option<Value>) -> Value {
    value.unwrap_or(Value::Nil)
}
