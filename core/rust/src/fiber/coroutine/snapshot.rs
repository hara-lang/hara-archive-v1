//! Portable, bounded projections of the live production evaluator fiber.
//!
//! These snapshots observe the retained CPS continuation introduced by the
//! live-fiber seam. They contain only owned scalar and string data: executable
//! values, promises, continuations, mutable cells, and host handles remain
//! owned by [`EvalFiber`].

use super::super::*;
use super::semantic;
use crate::kernel::{Position, Span, SpannedForm};
use crate::lang::data::{OrderedMap, Vector};

pub const INTERPRETER_LIVE_SNAPSHOT_SCHEMA: &str = "hal.interpreter-live-snapshot/0-alpha";
pub const INTERPRETER_LIVE_BOUNDARY_SCHEMA: &str = "hal.interpreter-live-boundary/0-alpha";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EvalObservationLimits {
    pub bindings: usize,
    pub display_chars: usize,
}

impl Default for EvalObservationLimits {
    fn default() -> Self {
        Self {
            bindings: 64,
            display_chars: 160,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvalObservationStatus {
    Running,
    Paused,
    Suspended,
    Returned,
    Failed,
    Cancelled,
}

impl EvalObservationStatus {
    pub const fn as_keyword(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Paused => "paused",
            Self::Suspended => "suspended",
            Self::Returned => "returned",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Returned | Self::Failed | Self::Cancelled)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvalObservedBoundaryKind {
    Semantic,
    Continue,
    Suspend,
    Resume,
    Return,
    Fail,
    Noop,
}

impl EvalObservedBoundaryKind {
    pub const fn as_keyword(self) -> &'static str {
        match self {
            Self::Semantic => "evaluation/semantic",
            Self::Continue => "evaluation/continue",
            Self::Suspend => "evaluation/suspend",
            Self::Resume => "evaluation/resume",
            Self::Return => "evaluation/return",
            Self::Fail => "evaluation/fail",
            Self::Noop => "evaluation/noop",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvalValueSnapshot {
    pub kind: &'static str,
    pub display: String,
    pub truncated: bool,
    pub redacted: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvalBindingSnapshot {
    pub name: String,
    pub value: EvalValueSnapshot,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvalErrorSnapshot {
    pub message: String,
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvalPendingSnapshot {
    pub state: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvalPositionSnapshot {
    pub offset: usize,
    pub line: usize,
    pub column: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvalSourceSpanSnapshot {
    pub start: EvalPositionSnapshot,
    pub end: EvalPositionSnapshot,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvalFocusSnapshot {
    pub form: String,
    pub form_truncated: bool,
    pub form_kind: &'static str,
    pub path: Option<Vec<usize>>,
    pub span: Option<EvalSourceSpanSnapshot>,
    pub source_candidates: usize,
    pub ambiguous: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvalFrameSnapshot {
    pub kind: &'static str,
    pub binding_count: usize,
    pub bindings: Vec<EvalBindingSnapshot>,
    pub bindings_omitted: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvalSemanticCallSnapshot {
    pub name: String,
    pub arity: usize,
    pub arguments: Vec<EvalValueSnapshot>,
    pub arguments_omitted: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvalSemanticEffectSnapshot {
    pub target: String,
    pub before: Option<EvalValueSnapshot>,
    pub after: EvalValueSnapshot,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvalSemanticErrorSnapshot {
    pub category: &'static str,
    pub message: String,
    pub truncated: bool,
    pub caught: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvalSemanticSnapshot {
    pub sequence: usize,
    pub rule: &'static str,
    pub focus: EvalFocusSnapshot,
    pub result: Option<EvalValueSnapshot>,
    pub call: Option<EvalSemanticCallSnapshot>,
    pub effect: Option<EvalSemanticEffectSnapshot>,
    pub error: Option<EvalSemanticErrorSnapshot>,
    pub frames: Vec<EvalFrameSnapshot>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvalObservationSnapshot {
    pub schema: &'static str,
    pub source_id: String,
    pub status: EvalObservationStatus,
    pub paused: bool,
    pub binding_count: usize,
    pub bindings: Vec<EvalBindingSnapshot>,
    pub bindings_omitted: usize,
    pub semantic_pending: usize,
    pub semantic: Option<EvalSemanticSnapshot>,
    pub pending: Option<EvalPendingSnapshot>,
    pub result: Option<EvalValueSnapshot>,
    pub error: Option<EvalErrorSnapshot>,
}

impl EvalObservationSnapshot {
    pub fn to_value(&self) -> Value {
        object([
            ("schema", string(self.schema)),
            ("sourceId", string(&self.source_id)),
            ("status", string(self.status.as_keyword())),
            ("paused", Value::Bool(self.paused)),
            ("bindingCount", integer(self.binding_count)),
            ("bindings", vector(self.bindings.iter().map(binding_value))),
            ("bindingsOmitted", integer(self.bindings_omitted)),
            ("semanticPending", integer(self.semantic_pending)),
            (
                "semantic",
                optional_value(self.semantic.as_ref().map(semantic_value)),
            ),
            (
                "pending",
                optional_value(self.pending.as_ref().map(pending_value)),
            ),
            (
                "result",
                optional_value(self.result.as_ref().map(value_snapshot_value)),
            ),
            (
                "error",
                optional_value(self.error.as_ref().map(error_value)),
            ),
        ])
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvalObservedBoundary {
    pub schema: &'static str,
    pub kind: EvalObservedBoundaryKind,
    pub before: EvalObservationSnapshot,
    pub after: EvalObservationSnapshot,
}

impl EvalObservedBoundary {
    pub fn to_value(&self) -> Value {
        object([
            ("schema", string(self.schema)),
            ("kind", string(self.kind.as_keyword())),
            ("before", self.before.to_value()),
            ("after", self.after.to_value()),
        ])
    }
}

impl EvalFiber {
    /// Returns a bounded JSON-safe document with default observation limits.
    pub fn snapshot_observed_value(&self, source_id: impl Into<String>) -> Value {
        self.snapshot_observed(source_id, EvalObservationLimits::default())
            .to_value()
    }

    /// Returns a bounded JSON-safe document without exposing runtime handles.
    pub fn snapshot_observed_value_with_limits(
        &self,
        source_id: impl Into<String>,
        binding_limit: usize,
        display_chars: usize,
    ) -> Value {
        self.snapshot_observed(
            source_id,
            EvalObservationLimits {
                bindings: binding_limit,
                display_chars,
            },
        )
        .to_value()
    }

    /// Executes one production continuation and returns before/after evidence.
    pub fn step_observed_value(&mut self, source_id: impl Into<String>) -> Value {
        self.step_observed_snapshot(source_id, EvalObservationLimits::default())
            .to_value()
    }

    /// Executes one production continuation with caller-selected evidence bounds.
    pub fn step_observed_value_with_limits(
        &mut self,
        source_id: impl Into<String>,
        binding_limit: usize,
        display_chars: usize,
    ) -> Value {
        self.step_observed_snapshot(
            source_id,
            EvalObservationLimits {
                bindings: binding_limit,
                display_chars,
            },
        )
        .to_value()
    }

    /// Applies one real promise settlement and returns before/after evidence.
    pub fn resume_observed_value(
        &mut self,
        state: PromiseState,
        source_id: impl Into<String>,
    ) -> Value {
        self.resume_observed_snapshot(state, source_id, EvalObservationLimits::default())
            .to_value()
    }

    /// Applies one promise settlement with caller-selected evidence bounds.
    pub fn resume_observed_value_with_limits(
        &mut self,
        state: PromiseState,
        source_id: impl Into<String>,
        binding_limit: usize,
        display_chars: usize,
    ) -> Value {
        self.resume_observed_snapshot(
            state,
            source_id,
            EvalObservationLimits {
                bindings: binding_limit,
                display_chars,
            },
        )
        .to_value()
    }

    /// Projects the current evaluator state without exposing executable values.
    pub(crate) fn snapshot_observed(
        &self,
        source_id: impl Into<String>,
        limits: EvalObservationLimits,
    ) -> EvalObservationSnapshot {
        let source_id = source_id.into();
        let status = observation_status(self);
        let (binding_count, bindings, bindings_omitted) = {
            let environment = self.env.borrow();
            binding_projection(&environment, limits)
        };
        let semantic_pending = semantic::pending_count(&self.env);
        let semantic = semantic_snapshot(self, limits);
        let pending = self.pending.as_ref().map(|promise| EvalPendingSnapshot {
            state: promise_state_keyword(&promise.state()),
        });
        let result = match &self.state {
            EvalFiberState::Completed(value) => Some(value_snapshot(value, limits.display_chars)),
            _ => None,
        };
        let error = match &self.state {
            EvalFiberState::Failed(message) => {
                let (message, truncated) = bounded_text(message, limits.display_chars);
                Some(EvalErrorSnapshot { message, truncated })
            }
            _ => None,
        };

        EvalObservationSnapshot {
            schema: INTERPRETER_LIVE_SNAPSHOT_SCHEMA,
            source_id,
            status,
            paused: self.observed_paused(),
            binding_count,
            bindings,
            bindings_omitted,
            semantic_pending,
            semantic,
            pending,
            result,
            error,
        }
    }

    /// Executes one live evaluator boundary and returns bounded before/after state.
    pub(crate) fn step_observed_snapshot(
        &mut self,
        source_id: impl Into<String>,
        limits: EvalObservationLimits,
    ) -> EvalObservedBoundary {
        let source_id = source_id.into();
        let before = self.snapshot_observed(source_id.clone(), limits);
        self.step_observed();
        let after = self.snapshot_observed(source_id, limits);
        EvalObservedBoundary {
            schema: INTERPRETER_LIVE_BOUNDARY_SCHEMA,
            kind: boundary_kind(&before, &after, false),
            before,
            after,
        }
    }

    /// Applies one promise settlement and returns the resulting live boundary.
    pub(crate) fn resume_observed_snapshot(
        &mut self,
        state: PromiseState,
        source_id: impl Into<String>,
        limits: EvalObservationLimits,
    ) -> EvalObservedBoundary {
        let source_id = source_id.into();
        let before = self.snapshot_observed(source_id.clone(), limits);
        self.resume_observed(state);
        let after = self.snapshot_observed(source_id, limits);
        EvalObservedBoundary {
            schema: INTERPRETER_LIVE_BOUNDARY_SCHEMA,
            kind: boundary_kind(&before, &after, true),
            before,
            after,
        }
    }
}

fn observation_status(fiber: &EvalFiber) -> EvalObservationStatus {
    match &fiber.state {
        EvalFiberState::Running if fiber.observed_paused() => EvalObservationStatus::Paused,
        EvalFiberState::Running => EvalObservationStatus::Running,
        EvalFiberState::Suspended => EvalObservationStatus::Suspended,
        EvalFiberState::Completed(_) => EvalObservationStatus::Returned,
        EvalFiberState::Failed(_) => EvalObservationStatus::Failed,
        EvalFiberState::Cancelled => EvalObservationStatus::Cancelled,
    }
}

fn boundary_kind(
    before: &EvalObservationSnapshot,
    after: &EvalObservationSnapshot,
    resumed: bool,
) -> EvalObservedBoundaryKind {
    let before_sequence = before.semantic.as_ref().map(|semantic| semantic.sequence);
    let after_sequence = after.semantic.as_ref().map(|semantic| semantic.sequence);
    let semantic_advanced = before_sequence != after_sequence;
    if semantic_advanced && before.status == after.status {
        return EvalObservedBoundaryKind::Semantic;
    }
    match after.status {
        EvalObservationStatus::Suspended => EvalObservedBoundaryKind::Suspend,
        EvalObservationStatus::Returned => EvalObservedBoundaryKind::Return,
        EvalObservationStatus::Failed => EvalObservedBoundaryKind::Fail,
        EvalObservationStatus::Cancelled => EvalObservedBoundaryKind::Noop,
        EvalObservationStatus::Running | EvalObservationStatus::Paused if resumed => {
            EvalObservedBoundaryKind::Resume
        }
        EvalObservationStatus::Running | EvalObservationStatus::Paused => {
            if before.status.is_terminal() {
                EvalObservedBoundaryKind::Noop
            } else {
                EvalObservedBoundaryKind::Continue
            }
        }
    }
}

fn binding_projection(
    environment: &HashMap<String, Value>,
    limits: EvalObservationLimits,
) -> (usize, Vec<EvalBindingSnapshot>, usize) {
    let mut bindings = environment
        .iter()
        .map(|(name, value)| EvalBindingSnapshot {
            name: name.clone(),
            value: value_snapshot(value, limits.display_chars),
        })
        .collect::<Vec<_>>();
    bindings.sort_by(|left, right| left.name.cmp(&right.name));
    let binding_count = bindings.len();
    bindings.truncate(limits.bindings);
    let bindings_omitted = binding_count.saturating_sub(bindings.len());
    (binding_count, bindings, bindings_omitted)
}

fn frame_snapshot(
    kind: &'static str,
    environment: &HashMap<String, Value>,
    limits: EvalObservationLimits,
) -> EvalFrameSnapshot {
    let (binding_count, bindings, bindings_omitted) = binding_projection(environment, limits);
    EvalFrameSnapshot {
        kind,
        binding_count,
        bindings,
        bindings_omitted,
    }
}

fn semantic_snapshot(
    fiber: &EvalFiber,
    limits: EvalObservationLimits,
) -> Option<EvalSemanticSnapshot> {
    let boundary = semantic::current_boundary(&fiber.env)?;
    let source_forms = semantic::source_forms(&fiber.env);
    let focus = focus_snapshot(
        &boundary.form,
        source_forms.as_deref().map(Vec::as_slice),
        limits.display_chars,
    );
    let current = frame_snapshot("current", &boundary.environment, limits);
    let session = {
        let environment = fiber.env.borrow();
        frame_snapshot("session", &environment, limits)
    };
    let (result, call, effect, error) = match &boundary.payload {
        semantic::EvalSemanticPayload::Result(value) => (
            Some(value_snapshot(value, limits.display_chars)),
            None,
            None,
            None,
        ),
        semantic::EvalSemanticPayload::Call { name, arguments } => {
            let arity = arguments.len();
            let retained = arguments
                .iter()
                .take(limits.bindings)
                .map(|value| value_snapshot(value, limits.display_chars))
                .collect::<Vec<_>>();
            (
                None,
                Some(EvalSemanticCallSnapshot {
                    name: name.clone(),
                    arity,
                    arguments_omitted: arity.saturating_sub(retained.len()),
                    arguments: retained,
                }),
                None,
                None,
            )
        }
        semantic::EvalSemanticPayload::Effect {
            target,
            before,
            after,
        } => (
            None,
            None,
            Some(EvalSemanticEffectSnapshot {
                target: target.clone(),
                before: before
                    .as_ref()
                    .map(|value| value_snapshot(value, limits.display_chars)),
                after: value_snapshot(after, limits.display_chars),
            }),
            None,
        ),
        semantic::EvalSemanticPayload::Error { message, caught } => {
            let (message, truncated) = bounded_text(message, limits.display_chars);
            (
                None,
                None,
                None,
                Some(EvalSemanticErrorSnapshot {
                    category: normalized_error_category(&message),
                    message,
                    truncated,
                    caught: *caught,
                }),
            )
        }
    };
    Some(EvalSemanticSnapshot {
        sequence: boundary.sequence,
        rule: boundary.rule.as_keyword(),
        focus,
        result,
        call,
        effect,
        error,
        frames: vec![current, session],
    })
}

fn normalized_error_category(message: &str) -> &'static str {
    let message = message.to_ascii_lowercase();
    if message.contains("division by zero")
        || message.contains("divide by zero")
        || message.contains("/ by zero")
    {
        "division by zero"
    } else if message.contains("expects numbers")
        || message.contains("expects two numbers")
        || message.contains("expected a number")
        || message.contains("expected numeric")
    {
        "expects numbers"
    } else if message.contains("unbound symbol") || message.contains("unbound var") {
        "unbound symbol"
    } else if message.contains("recur") {
        "recur"
    } else if message.contains("unsupported") {
        "unsupported form"
    } else {
        "runtime"
    }
}

#[derive(Clone)]
struct SourceMatch {
    path: Vec<usize>,
    span: Span,
}

fn focus_snapshot(
    form: &Form,
    source_forms: Option<&[SpannedForm]>,
    display_chars: usize,
) -> EvalFocusSnapshot {
    let matches = source_forms
        .map(|forms| source_matches(forms, form))
        .unwrap_or_default();
    let source_candidates = matches.len();
    let unique = source_candidates == 1;
    let (path, span) = if unique {
        let matched = matches.into_iter().next().expect("one source match");
        (Some(matched.path), Some(span_snapshot(&matched.span)))
    } else {
        (None, None)
    };
    let form_kind = form_kind(form);
    let (form, form_truncated) = bounded_text(&form.to_string(), display_chars);
    EvalFocusSnapshot {
        form,
        form_truncated,
        form_kind,
        path,
        span,
        source_candidates,
        ambiguous: source_candidates > 1,
    }
}

fn form_kind(form: &Form) -> &'static str {
    match form {
        Form::Symbol(_) => "symbol",
        Form::List(values) => match values.first() {
            Some(Form::Symbol(name)) if SYNC_SPECIAL_FORMS.contains(&name.as_str()) => {
                "special-form"
            }
            _ => "call",
        },
        Form::Map(_) | Form::Set(_) | Form::Vector(_) => "collection",
        Form::Metadata(_, _) => "metadata",
        Form::Tagged(_, _) => "tagged",
        _ => "literal",
    }
}

fn source_matches(forms: &[SpannedForm], target: &Form) -> Vec<SourceMatch> {
    let mut output = Vec::new();
    collect_source_matches(forms, target, &[], &mut output);
    output
}

fn collect_source_matches(
    forms: &[SpannedForm],
    target: &Form,
    prefix: &[usize],
    output: &mut Vec<SourceMatch>,
) {
    for (index, form) in forms.iter().enumerate() {
        let mut path = prefix.to_vec();
        path.push(index);
        if &form.form == target {
            output.push(SourceMatch {
                path: path.clone(),
                span: form.span.clone(),
            });
        }
        collect_source_matches(&form.children, target, &path, output);
    }
}

fn span_snapshot(span: &Span) -> EvalSourceSpanSnapshot {
    EvalSourceSpanSnapshot {
        start: position_snapshot(span.start),
        end: position_snapshot(span.end),
    }
}

fn position_snapshot(position: Position) -> EvalPositionSnapshot {
    EvalPositionSnapshot {
        offset: position.offset,
        line: position.line,
        column: position.column,
    }
}

fn value_snapshot(value: &Value, display_chars: usize) -> EvalValueSnapshot {
    let kind = value_kind(value);
    let (display, redacted) = safe_display(value);
    let (display, truncated) = bounded_text(&display, display_chars);
    EvalValueSnapshot {
        kind,
        display,
        truncated,
        redacted,
    }
}

fn value_kind(value: &Value) -> &'static str {
    match value {
        Value::Number(_) => "long",
        Value::BigInteger(_) if crate::numeric::is_long_value(value) => "long",
        Value::BigInteger(_) => "bigint",
        Value::Float(_) => "float",
        Value::Character(_) => "character",
        Value::Bool(_) => "boolean",
        Value::String(_) => "string",
        Value::Keyword(_) => "keyword",
        Value::Symbol(_) => "symbol",
        Value::Bytes(_) => "bytes",
        Value::Promise(_) => "promise",
        Value::Function(_) => "function",
        Value::Var(_) => "var",
        Value::Extension(_) => "extension",
        Value::Coroutine(_) => "coroutine",
        Value::Iterator(_) => "iterator",
        Value::Nil => "nil",
        _ => "value",
    }
}

fn safe_display(value: &Value) -> (String, bool) {
    match value {
        Value::Promise(promise) => (
            format!("<promise {}>", promise_state_keyword(&promise.state())),
            true,
        ),
        Value::Function(_) => ("<function>".into(), true),
        Value::Coroutine(_) => ("<coroutine>".into(), true),
        Value::Iterator(_) => ("<iterator>".into(), true),
        Value::Extension(extension) => (
            format!("<extension {}/{}>", extension.provider, extension.type_name),
            true,
        ),
        Value::ByteBuffer(_) => ("<byte-buffer>".into(), true),
        Value::Array(_) => ("<array>".into(), true),
        Value::Object(_) => ("<object>".into(), true),
        Value::MutableCollection(_) => ("<mutable-collection>".into(), true),
        Value::Mutable(_) => ("<mutable>".into(), true),
        _ => (value.display(), false),
    }
}

fn promise_state_keyword(state: &PromiseState) -> &'static str {
    match state {
        PromiseState::Pending => "pending",
        PromiseState::Fulfilled(_) => "fulfilled",
        PromiseState::Rejected(_) => "rejected",
    }
}

fn bounded_text(value: &str, limit: usize) -> (String, bool) {
    let mut characters = value.chars();
    let mut retained = characters.by_ref().take(limit).collect::<String>();
    let truncated = characters.next().is_some();
    if truncated {
        retained.push('…');
    }
    (retained, truncated)
}

fn semantic_value(semantic: &EvalSemanticSnapshot) -> Value {
    object([
        ("sequence", integer(semantic.sequence)),
        ("rule", string(semantic.rule)),
        ("focus", focus_value(&semantic.focus)),
        (
            "result",
            optional_value(semantic.result.as_ref().map(value_snapshot_value)),
        ),
        (
            "call",
            optional_value(semantic.call.as_ref().map(semantic_call_value)),
        ),
        (
            "effect",
            optional_value(semantic.effect.as_ref().map(semantic_effect_value)),
        ),
        (
            "error",
            optional_value(semantic.error.as_ref().map(semantic_error_value)),
        ),
        ("frames", vector(semantic.frames.iter().map(frame_value))),
    ])
}

fn semantic_call_value(call: &EvalSemanticCallSnapshot) -> Value {
    object([
        ("name", string(&call.name)),
        ("arity", integer(call.arity)),
        (
            "arguments",
            vector(call.arguments.iter().map(value_snapshot_value)),
        ),
        ("argumentsOmitted", integer(call.arguments_omitted)),
    ])
}

fn semantic_effect_value(effect: &EvalSemanticEffectSnapshot) -> Value {
    object([
        ("target", string(&effect.target)),
        (
            "before",
            optional_value(effect.before.as_ref().map(value_snapshot_value)),
        ),
        ("after", value_snapshot_value(&effect.after)),
    ])
}

fn semantic_error_value(error: &EvalSemanticErrorSnapshot) -> Value {
    object([
        ("category", string(error.category)),
        ("message", string(&error.message)),
        ("truncated", Value::Bool(error.truncated)),
        ("caught", Value::Bool(error.caught)),
    ])
}

fn focus_value(focus: &EvalFocusSnapshot) -> Value {
    object([
        ("form", string(&focus.form)),
        ("formTruncated", Value::Bool(focus.form_truncated)),
        ("formKind", string(focus.form_kind)),
        (
            "path",
            optional_value(
                focus
                    .path
                    .as_ref()
                    .map(|path| vector(path.iter().copied().map(integer))),
            ),
        ),
        (
            "span",
            optional_value(focus.span.as_ref().map(source_span_value)),
        ),
        ("sourceCandidates", integer(focus.source_candidates)),
        ("ambiguous", Value::Bool(focus.ambiguous)),
    ])
}

fn source_span_value(span: &EvalSourceSpanSnapshot) -> Value {
    object([
        ("start", position_value(&span.start)),
        ("end", position_value(&span.end)),
    ])
}

fn position_value(position: &EvalPositionSnapshot) -> Value {
    object([
        ("offset", integer(position.offset)),
        ("line", integer(position.line)),
        ("column", integer(position.column)),
    ])
}

fn frame_value(frame: &EvalFrameSnapshot) -> Value {
    object([
        ("kind", string(frame.kind)),
        ("bindingCount", integer(frame.binding_count)),
        ("bindings", vector(frame.bindings.iter().map(binding_value))),
        ("bindingsOmitted", integer(frame.bindings_omitted)),
    ])
}

fn binding_value(binding: &EvalBindingSnapshot) -> Value {
    object([
        ("name", string(&binding.name)),
        ("value", value_snapshot_value(&binding.value)),
    ])
}

fn value_snapshot_value(value: &EvalValueSnapshot) -> Value {
    object([
        ("kind", string(value.kind)),
        ("display", string(&value.display)),
        ("truncated", Value::Bool(value.truncated)),
        ("redacted", Value::Bool(value.redacted)),
    ])
}

fn pending_value(pending: &EvalPendingSnapshot) -> Value {
    object([("state", string(pending.state))])
}

fn error_value(error: &EvalErrorSnapshot) -> Value {
    object([
        ("message", string(&error.message)),
        ("truncated", Value::Bool(error.truncated)),
    ])
}

fn object<const N: usize>(fields: [(&str, Value); N]) -> Value {
    Value::OrderedMap(Box::new(OrderedMap::from_iter(
        fields
            .into_iter()
            .map(|(key, value)| (Value::String(key.into()), value)),
    )))
}

fn vector(values: impl IntoIterator<Item = Value>) -> Value {
    Value::Vector(Vector::from_iter(values))
}

fn string(value: impl Into<String>) -> Value {
    Value::String(value.into())
}

fn integer(value: usize) -> Value {
    Value::Number(i64::try_from(value).unwrap_or(i64::MAX))
}

fn optional_value(value: Option<Value>) -> Value {
    value.unwrap_or(Value::Nil)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshots_sort_bound_and_redact_environment_bindings() {
        let mut environment = HashMap::new();
        environment.insert("zeta".into(), Value::Number(3));
        environment.insert("alpha".into(), Value::String("abcdefgh".into()));
        environment.insert(
            "extension".into(),
            Value::Extension(ExtensionValue {
                provider: "demo".into(),
                type_name: "socket".into(),
                handle: 999,
            }),
        );
        let fiber = EvalFiber::start_observed("nil", environment).unwrap();
        let snapshot = fiber.snapshot_observed(
            "fixture/snapshot.hal",
            EvalObservationLimits {
                bindings: 2,
                display_chars: 4,
            },
        );

        assert_eq!(snapshot.status, EvalObservationStatus::Paused);
        assert_eq!(snapshot.binding_count, 3);
        assert_eq!(snapshot.bindings_omitted, 1);
        assert_eq!(snapshot.bindings[0].name, "alpha");
        assert_eq!(snapshot.bindings[1].name, "extension");
        assert!(snapshot.bindings[0].value.truncated);
        assert!(snapshot.bindings[1].value.redacted);
        assert!(!snapshot.bindings[1].value.display.contains("999"));
        let json = crate::json::write(&snapshot.to_value()).unwrap();
        assert!(json.contains("hal.interpreter-live-snapshot/0-alpha"));
        assert!(!json.contains("999"));
    }

    #[test]
    fn live_boundaries_project_before_after_state_and_terminal_result() {
        let limits = EvalObservationLimits::default();
        let mut fiber = EvalFiber::start_observed("(+ 19 23)", HashMap::new()).unwrap();
        let first = fiber.step_observed_snapshot("fixture/add.hal", limits);
        assert_eq!(first.kind, EvalObservedBoundaryKind::Semantic);
        assert_eq!(first.before.status, EvalObservationStatus::Paused);
        assert_eq!(first.after.status, EvalObservationStatus::Paused);

        let mut returned = fiber.step_observed_snapshot("fixture/add.hal", limits);
        while returned.after.status == EvalObservationStatus::Paused {
            returned = fiber.step_observed_snapshot("fixture/add.hal", limits);
        }
        assert_eq!(returned.kind, EvalObservedBoundaryKind::Return);
        assert_eq!(returned.after.status, EvalObservationStatus::Returned);
        assert_eq!(
            returned
                .after
                .result
                .as_ref()
                .map(|value| value.display.as_str()),
            Some("42")
        );
        let json = crate::json::write(&returned.to_value()).unwrap();
        assert!(json.contains("evaluation/return"));
        assert!(json.contains("\"display\":\"42\""));
    }

    #[test]
    fn promise_boundaries_expose_state_without_identity_or_automatic_drain() {
        let promise = Promise::new();
        let mut environment = HashMap::new();
        environment.insert("pending-value".into(), Value::Promise(promise.clone()));
        let limits = EvalObservationLimits::default();
        let mut fiber =
            EvalFiber::start_observed("(Coroutine/await pending-value)", environment).unwrap();

        while matches!(fiber.state(), EvalFiberState::Running) {
            fiber.step_observed_snapshot("fixture/await.hal", limits);
        }
        let suspended = fiber.snapshot_observed("fixture/await.hal", limits);
        assert_eq!(suspended.status, EvalObservationStatus::Suspended);
        assert_eq!(
            suspended.pending.as_ref().map(|pending| pending.state),
            Some("pending")
        );

        promise.resolve(Value::Number(42));
        let resumed = fiber.resume_observed_snapshot(promise.state(), "fixture/await.hal", limits);
        assert_eq!(resumed.kind, EvalObservedBoundaryKind::Resume);
        assert_eq!(resumed.after.status, EvalObservationStatus::Paused);
        assert!(resumed.after.pending.is_none());
        let json = crate::json::write(&resumed.to_value()).unwrap();
        assert!(!json.contains("identity"));
    }

    fn collect_semantics(source: &str) -> Vec<EvalSemanticSnapshot> {
        let mut fiber = EvalFiber::start_observed(source, HashMap::new()).unwrap();
        let mut output = Vec::new();
        let mut sequence = 0;
        loop {
            let snapshot =
                fiber.snapshot_observed("fixture/semantic.hal", EvalObservationLimits::default());
            if !matches!(fiber.state(), EvalFiberState::Running) && snapshot.semantic_pending == 0 {
                break;
            }
            let boundary = fiber
                .step_observed_snapshot("fixture/semantic.hal", EvalObservationLimits::default());
            if let Some(semantic) = boundary.after.semantic {
                if semantic.sequence > sequence {
                    sequence = semantic.sequence;
                    output.push(semantic);
                }
            }
            assert!(sequence < 128, "semantic evaluation did not terminate");
        }
        output
    }

    #[test]
    fn nested_calls_retain_actual_result_form_path_and_span() {
        let semantics = collect_semantics("(+ 1 (* 2 3))");
        let multiply = semantics
            .iter()
            .find(|semantic| {
                semantic.focus.form == "(* 2 3)"
                    && semantic
                        .result
                        .as_ref()
                        .is_some_and(|result| result.display == "6")
            })
            .expect("inner multiply return boundary");
        assert_eq!(
            multiply
                .result
                .as_ref()
                .map(|result| result.display.as_str()),
            Some("6")
        );
        assert_eq!(multiply.focus.form_kind, "call");
        assert_eq!(multiply.focus.path.as_deref(), Some(&[0, 2][..]));
        assert_eq!(multiply.focus.source_candidates, 1);
        assert_eq!(
            multiply
                .focus
                .span
                .as_ref()
                .map(|span| (span.start.offset, span.end.offset)),
            Some((5, 12))
        );

        let outer = semantics
            .iter()
            .find(|semantic| {
                semantic.focus.form == "(+ 1 (* 2 3))"
                    && semantic
                        .result
                        .as_ref()
                        .is_some_and(|result| result.display == "7")
            })
            .expect("outer addition boundary");
        assert_eq!(outer.focus.path.as_deref(), Some(&[0][..]));
    }

    #[test]
    fn lexical_boundary_captures_binding_before_scope_restoration() {
        let semantics = collect_semantics("(let [x 41] (+ x 1))");
        let resolved = semantics
            .iter()
            .find(|semantic| {
                semantic.focus.form == "x"
                    && semantic
                        .result
                        .as_ref()
                        .is_some_and(|result| result.display == "41")
            })
            .expect("resolved lexical symbol boundary");
        let current = resolved
            .frames
            .iter()
            .find(|frame| frame.kind == "current")
            .expect("current lexical frame");
        let x = current
            .bindings
            .iter()
            .find(|binding| binding.name == "x")
            .expect("captured x binding");
        assert_eq!(x.value.display, "41");
    }

    #[test]
    fn duplicate_source_forms_are_explicitly_ambiguous() {
        let semantics = collect_semantics("(+ 1 1)");
        let literal = semantics
            .iter()
            .find(|semantic| semantic.focus.form == "1")
            .expect("literal boundary");
        assert_eq!(literal.focus.source_candidates, 2);
        assert!(literal.focus.ambiguous);
        assert!(literal.focus.path.is_none());
        assert!(literal.focus.span.is_none());
    }

    #[test]
    fn call_entry_is_published_before_the_matching_return() {
        let semantics = collect_semantics("(+ 1 (* 2 3))");
        let enter = semantics
            .iter()
            .position(|semantic| semantic.rule == "call/enter" && semantic.focus.form == "(* 2 3)")
            .expect("inner call entry");
        let returned = semantics
            .iter()
            .position(|semantic| {
                semantic.rule == "value/return"
                    && semantic.focus.form == "(* 2 3)"
                    && semantic
                        .result
                        .as_ref()
                        .is_some_and(|result| result.display == "6")
            })
            .expect("inner call return");
        assert!(enter < returned);
        let call = semantics[enter].call.as_ref().expect("call payload");
        assert_eq!(call.arity, 2);
        assert_eq!(
            call.arguments
                .iter()
                .map(|argument| argument.display.as_str())
                .collect::<Vec<_>>(),
            vec!["2", "3"]
        );
    }

    #[test]
    fn var_mutations_are_explicit_ordered_effects() {
        let semantics = collect_semantics("(do (def counter 1) (set! counter 42) counter)");
        let define = semantics
            .iter()
            .find(|semantic| semantic.rule == "effect/var-define")
            .expect("definition effect");
        let define_effect = define.effect.as_ref().expect("definition payload");
        assert_eq!(define_effect.after.display, "1");

        let set = semantics
            .iter()
            .find(|semantic| semantic.rule == "effect/var-set")
            .expect("set effect");
        let set_effect = set.effect.as_ref().expect("set payload");
        assert_eq!(
            set_effect
                .before
                .as_ref()
                .map(|value| value.display.as_str()),
            Some("1")
        );
        assert_eq!(set_effect.after.display, "42");
        assert!(define.sequence < set.sequence);
    }

    #[test]
    fn raised_errors_and_selected_catches_are_explicit() {
        let semantics = collect_semantics("(try (/ 1 0) (catch Exception error 42))");
        let raised = semantics
            .iter()
            .find(|semantic| semantic.rule == "error/raise")
            .expect("raise event");
        let raised_error = raised.error.as_ref().expect("raise payload");
        assert_eq!(raised_error.category, "division by zero");
        assert!(!raised_error.caught);
        assert_eq!(raised.focus.form, "(/ 1 0)");

        let caught = semantics
            .iter()
            .find(|semantic| semantic.rule == "error/catch")
            .expect("catch event");
        assert!(caught.error.as_ref().is_some_and(|error| error.caught));
        assert!(raised.sequence < caught.sequence);
    }
}
