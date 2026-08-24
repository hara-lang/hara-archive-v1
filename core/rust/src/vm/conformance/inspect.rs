use crate::core::Value;
use crate::journal::{Journal, JournalEventKind};
use crate::kernel::halc_trace::{HalcArtifactTrace, HalcTraceValue};
use crate::vm::CompileErrorKind;

use super::{ExpectedOutcome, StageOutcome, TeachingAnnotation};

pub(super) fn teaching_annotations(
    journal: &Journal,
    halc: &HalcArtifactTrace,
    bytecode_trace: &Value,
) -> Vec<TeachingAnnotation> {
    let mut annotations = Vec::new();
    for event in journal.events.iter().take(10) {
        annotations.push(TeachingAnnotation {
            concept: journal_concept(&event.kind).into(),
            stage: "interpreter".into(),
            sequence: usize::try_from(event.sequence).unwrap_or(usize::MAX),
            detail: event
                .function
                .clone()
                .unwrap_or_else(|| journal_kind(&event.kind).into()),
        });
    }
    for event in halc.events.iter().take(10) {
        annotations.push(TeachingAnnotation {
            concept: "compilation/stage".into(),
            stage: "halc".into(),
            sequence: usize::try_from(event.sequence).unwrap_or(usize::MAX),
            detail: event.stage.into(),
        });
    }
    for step in trace_steps(bytecode_trace).into_iter().take(10) {
        annotations.push(TeachingAnnotation {
            concept: "execution/instruction".into(),
            stage: "bytecode".into(),
            sequence: map_usize(&step, "sequence").unwrap_or(0),
            detail: map_string(&step, "kind").unwrap_or_else(|| "step".into()),
        });
    }
    annotations
}

pub fn normalize_error_category(message: &str) -> &'static str {
    let buckets: &[(&[&str], &str)] = &[
        (&["division by zero"], "division by zero"),
        (&["non-finite number"], "non-finite number"),
        (&["integer overflow"], "integer overflow"),
        (&["expects numbers"], "expects numbers"),
        (&["unbound symbol"], "unbound symbol"),
        (&["recur"], "recur"),
        (&["unsupported"], "unsupported form"),
        (&["Invalid number", "EOF while reading"], "reader"),
    ];
    for (markers, category) in buckets {
        if markers.iter().any(|marker| message.contains(marker)) {
            return category;
        }
    }
    "unclassified"
}

pub(super) fn compile_error_category(kind: CompileErrorKind) -> &'static str {
    match kind {
        CompileErrorKind::Parse => "reader",
        CompileErrorKind::UnsupportedForm => "unsupported form",
        CompileErrorKind::UnboundSymbol => "unbound symbol",
        CompileErrorKind::Arity => "arity",
        CompileErrorKind::Recur => "recur",
        CompileErrorKind::InvalidEffect => "invalid effect",
        CompileErrorKind::Limit => "limit",
        CompileErrorKind::Internal => "internal",
    }
}

pub(super) fn expected_text(expected: &ExpectedOutcome) -> String {
    match expected {
        ExpectedOutcome::Display(value) => format!("display:{value}"),
        ExpectedOutcome::ErrorCategory(value) => format!("error-category:{value}"),
        ExpectedOutcome::CompileError(value) => format!("compile-error:{value}"),
    }
}

pub(super) fn outcome_text(outcome: &StageOutcome) -> String {
    format!(
        "status={},display={},category={}",
        outcome.status,
        outcome.display.as_deref().unwrap_or("none"),
        outcome.category.as_deref().unwrap_or("none")
    )
}

pub(super) fn normalize_compile_marker(value: &str) -> &str {
    if value.contains("unbound") {
        "unbound-symbol"
    } else if value.contains("recur") {
        "recur"
    } else if value.contains("unsupported") || value.contains("not supported") {
        "unsupported-form"
    } else {
        value
    }
}

pub(super) fn trace_evidence_string(trace: &HalcArtifactTrace, key: &str) -> Option<String> {
    trace
        .result
        .as_ref()
        .and_then(|values| evidence_string(values, key))
        .or_else(|| {
            trace
                .events
                .iter()
                .find_map(|event| evidence_string(&event.evidence, key))
        })
}

pub(super) fn evidence_string(
    evidence: &std::collections::BTreeMap<String, HalcTraceValue>,
    key: &str,
) -> Option<String> {
    match evidence.get(key) {
        Some(HalcTraceValue::String(value)) => Some(value.clone()),
        _ => None,
    }
}

pub(super) fn evidence_bool(
    evidence: &std::collections::BTreeMap<String, HalcTraceValue>,
    key: &str,
) -> Option<bool> {
    match evidence.get(key) {
        Some(HalcTraceValue::Boolean(value)) => Some(*value),
        _ => None,
    }
}

fn map_entry(value: &Value, key: &str) -> Option<Value> {
    crate::core::map_entries(value)?
        .into_iter()
        .find_map(|(candidate, value)| {
            matches!(candidate, Value::String(ref name) if name == key).then_some(value)
        })
}

fn map_string(value: &Value, key: &str) -> Option<String> {
    match map_entry(value, key) {
        Some(Value::String(value)) => Some(value),
        _ => None,
    }
}

fn map_usize(value: &Value, key: &str) -> Option<usize> {
    match map_entry(value, key) {
        Some(Value::Number(value)) if value >= 0 => usize::try_from(value).ok(),
        _ => None,
    }
}

pub(super) fn trace_string(value: &Value, key: &str) -> Option<String> {
    map_string(value, key)
}

pub(super) fn trace_usize(value: &Value, key: &str) -> Option<usize> {
    map_usize(value, key)
}

pub(super) fn trace_steps(value: &Value) -> Vec<Value> {
    match map_entry(value, "steps") {
        Some(Value::Vector(values)) => values.iter().cloned().collect(),
        _ => Vec::new(),
    }
}

pub(super) fn trace_sequences_contiguous(value: &Value) -> bool {
    let steps = trace_steps(value);
    let sequences = steps
        .iter()
        .filter_map(|step| map_usize(step, "sequence"))
        .collect::<Vec<_>>();
    sequences.len() == steps.len()
        && sequences
            .windows(2)
            .all(|window| window[1] == window[0].saturating_add(1))
}

fn journal_concept(kind: &JournalEventKind) -> &'static str {
    match kind {
        JournalEventKind::EvaluationStart => "evaluation/order",
        JournalEventKind::MacroExpand => "macro/expansion",
        JournalEventKind::OperationEnter => "operation/enter",
        JournalEventKind::OperationReturn => "operation/return",
        JournalEventKind::Error => "error/propagation",
        JournalEventKind::JournalTruncated => "trace/bounded",
    }
}

fn journal_kind(kind: &JournalEventKind) -> &'static str {
    match kind {
        JournalEventKind::EvaluationStart => "evaluation/start",
        JournalEventKind::MacroExpand => "macro/expand",
        JournalEventKind::OperationEnter => "operation/enter",
        JournalEventKind::OperationReturn => "operation/return",
        JournalEventKind::Error => "evaluation/error",
        JournalEventKind::JournalTruncated => "journal/truncated",
    }
}
