//! Runtime implementation of the portable HAL Evaluation Journal contract.
//!
//! This module deliberately owns no evaluator state.  The evaluator will add
//! hooks in the next slice; keeping collection separate makes it possible to
//! prove that the schema and its limits do not affect normal evaluation.

use std::fmt;

pub const SCHEMA: &str = "hal.evaluation-journal/0-alpha";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct JournalId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EventId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OperationId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalLimits {
    pub max_events: usize,
    pub max_depth: usize,
    pub max_value_chars: usize,
}

impl Default for JournalLimits {
    fn default() -> Self {
        Self {
            max_events: 10_000,
            max_depth: 100,
            max_value_chars: 1_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValuePreview {
    pub type_name: String,
    pub display: String,
    pub truncated: bool,
}

impl ValuePreview {
    pub fn new(type_name: impl Into<String>, display: impl AsRef<str>, limit: usize) -> Self {
        let display = display.as_ref();
        let mut chars = display.chars();
        let bounded: String = chars.by_ref().take(limit).collect();
        let truncated = chars.next().is_some();
        let bounded = if truncated {
            display.chars().take(limit.saturating_sub(1)).collect()
        } else {
            bounded
        };
        Self {
            type_name: type_name.into(),
            display: if truncated {
                format!("{bounded}…")
            } else {
                bounded
            },
            truncated,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JournalEventKind {
    EvaluationStart,
    MacroExpand,
    OperationEnter,
    OperationReturn,
    Error,
    JournalTruncated,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalEvent {
    pub id: EventId,
    pub sequence: u64,
    pub kind: JournalEventKind,
    pub operation: Option<OperationId>,
    pub parent_operation: Option<OperationId>,
    pub depth: usize,
    pub function: Option<String>,
    pub values: Vec<ValuePreview>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JournalStatus {
    Ok,
    Error,
    Truncated,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Journal {
    pub schema: &'static str,
    pub journal_id: JournalId,
    pub status: JournalStatus,
    pub events: Vec<JournalEvent>,
    pub result: Option<ValuePreview>,
    pub error: Option<String>,
}

/// Bounded event collector used by development evaluator hooks.
#[derive(Debug)]
pub struct JournalCollector {
    journal: Journal,
    limits: JournalLimits,
    next_event: u64,
    next_operation: u64,
    truncated: bool,
}

impl JournalCollector {
    pub fn new(journal_id: JournalId, limits: JournalLimits) -> Self {
        Self {
            journal: Journal {
                schema: SCHEMA,
                journal_id,
                status: JournalStatus::Ok,
                events: Vec::new(),
                result: None,
                error: None,
            },
            limits,
            next_event: 1,
            next_operation: 1,
            truncated: false,
        }
    }

    pub fn next_operation_id(&mut self) -> OperationId {
        let id = OperationId(self.next_operation);
        self.next_operation += 1;
        id
    }

    pub fn preview_value(
        &self,
        type_name: impl Into<String>,
        display: impl AsRef<str>,
    ) -> ValuePreview {
        ValuePreview::new(type_name, display, self.limits.max_value_chars)
    }

    pub fn record(&mut self, mut event: JournalEvent) {
        if self.truncated {
            return;
        }
        if event.depth > self.limits.max_depth
            || self.journal.events.len() >= self.limits.max_events
        {
            self.truncated = true;
            self.journal.status = JournalStatus::Truncated;
            self.push_truncation_event();
            return;
        }
        event.id = EventId(self.next_event);
        event.sequence = self.next_event;
        self.next_event += 1;
        self.journal.events.push(event);
    }

    pub fn finish(mut self, result: ValuePreview) -> Journal {
        self.journal.result = Some(result);
        self.journal
    }

    pub fn fail(mut self, error: impl Into<String>) -> Journal {
        let error = error.into();
        self.record(JournalEvent::error(error.clone()));
        self.journal.status = if self.truncated {
            JournalStatus::Truncated
        } else {
            JournalStatus::Error
        };
        self.journal.error = Some(error);
        self.journal
    }

    fn push_truncation_event(&mut self) {
        // Reserve no extra capacity: the diagnostic replaces further detail.
        let id = EventId(self.next_event);
        self.next_event += 1;
        self.journal.events.push(JournalEvent {
            id,
            sequence: id.0,
            kind: JournalEventKind::JournalTruncated,
            operation: None,
            parent_operation: None,
            depth: 0,
            function: None,
            values: Vec::new(),
            message: Some("journal limit reached; evaluation continued".into()),
        });
    }
}

impl JournalEvent {
    pub fn new(kind: JournalEventKind) -> Self {
        Self {
            id: EventId(0),
            sequence: 0,
            kind,
            operation: None,
            parent_operation: None,
            depth: 0,
            function: None,
            values: Vec::new(),
            message: None,
        }
    }

    pub fn error(error: impl Into<String>) -> Self {
        let mut event = Self::new(JournalEventKind::Error);
        event.message = Some(error.into());
        event
    }
}

impl fmt::Display for JournalId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "journal-{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_conformance_corpus_is_readable_by_the_rust_runtime() {
        let source = std::fs::read_to_string(crate::spec_registry::require(
            "00-unsorted/diagnostics/draft/conformance/evaluation-journal.edn",
        ))
        .expect("evaluation journal corpus is readable");
        let forms = crate::kernel::parse_forms(source).unwrap();
        assert_eq!(forms.len(), 1);
        assert!(source.contains(":function/nested"));
        assert!(source.contains(":disabled/equivalence"));
    }

    #[test]
    fn collector_assigns_parent_linkable_operation_and_event_ids() {
        let mut collector = JournalCollector::new(JournalId(17), JournalLimits::default());
        let parent = collector.next_operation_id();
        let child = collector.next_operation_id();
        let mut enter = JournalEvent::new(JournalEventKind::OperationEnter);
        enter.operation = Some(parent);
        enter.function = Some("app.main/run".into());
        collector.record(enter);
        let mut nested = JournalEvent::new(JournalEventKind::OperationEnter);
        nested.operation = Some(child);
        nested.parent_operation = Some(parent);
        nested.depth = 1;
        nested.function = Some("app.math/calculate".into());
        collector.record(nested);

        let trace = collector.finish(ValuePreview::new("number", "12", 10));
        assert_eq!(trace.schema, SCHEMA);
        assert_eq!(trace.events[0].id, EventId(1));
        assert_eq!(trace.events[1].parent_operation, Some(parent));
        assert_eq!(trace.result.unwrap().display, "12");
    }

    #[test]
    fn collector_truncates_recording_without_losing_final_result() {
        let mut collector = JournalCollector::new(
            JournalId(1),
            JournalLimits {
                max_events: 1,
                ..JournalLimits::default()
            },
        );
        collector.record(JournalEvent::new(JournalEventKind::EvaluationStart));
        collector.record(JournalEvent::new(JournalEventKind::OperationEnter));
        let trace = collector.finish(ValuePreview::new("number", "12", 10));

        assert_eq!(trace.status, JournalStatus::Truncated);
        assert!(matches!(
            trace.events.last().unwrap().kind,
            JournalEventKind::JournalTruncated
        ));
        assert_eq!(trace.result.unwrap().display, "12");
    }

    #[test]
    fn value_previews_bound_unicode_without_invalid_utf8() {
        let value = ValuePreview::new("string", "λabcdef", 2);
        assert_eq!(value.display, "λ…");
        assert!(value.truncated);
    }
}
