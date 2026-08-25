//! Opt-in live stepping for the production CPS evaluator.
//!
//! The ordinary [`EvalFiber::start`] path still drains every trampoline
//! continuation immediately. `start_observed` stores that same continuation
//! inside the existing fiber and executes at most one `Step::Continue`
//! boundary per `step_observed` call. Promise suspension keeps the real
//! promise and resume closure; no journal replay or alternate evaluator is
//! involved.

use super::super::*;
use super::semantic;
use crate::instrumentation::{
    EventKind, EventLocation, PortableProjection, ProducerEvent, ProjectionLimits,
};
use crate::kernel::{read_forms, SpannedForm};
use std::collections::BTreeMap;

impl EvalFiber {
    /// Creates a live evaluator paused before the first production CPS step.
    pub fn start_observed(source: &str, env: HashMap<String, Value>) -> Result<Self, String> {
        let spanned = read_forms(source).map_err(|error| error.to_string())?;
        let forms = spanned.iter().map(|form| form.form.clone()).collect();
        Self::start_forms_observed_internal(forms, Some(Rc::new(spanned)), env)
    }

    /// Creates a live evaluator paused before evaluating forms without source spans.
    pub fn start_forms_observed(
        forms: Vec<Form>,
        env: HashMap<String, Value>,
    ) -> Result<Self, String> {
        Self::start_forms_observed_internal(forms, None, env)
    }

    fn start_forms_observed_internal(
        forms: Vec<Form>,
        source_forms: Option<Rc<Vec<SpannedForm>>>,
        env: HashMap<String, Value>,
    ) -> Result<Self, String> {
        let (namespace_registry, environment) = execution_context(env);
        let env = Rc::new(RefCell::new(environment));
        // The compatibility observer retains its historical full projection.
        // Shared instrumentation changes these flags before every safepoint.
        semantic::register_context(&env, source_forms, true, true);
        let execution_env = env.clone();
        let forms = Rc::new(forms);
        let resume: Resume =
            Box::new(move |_| forms_cps(forms, 0, Value::Nil, execution_env, Box::new(Step::Done)));
        Ok(Self {
            env,
            namespace_registry,
            pending: None,
            resume: Some(resume),
            state: EvalFiberState::Running,
        })
    }

    /// Configures semantic capture for the next real CPS safepoint. Disabling
    /// capture drops pending evidence but never changes the retained continuation.
    pub(crate) fn configure_instrumentation_capture(
        &self,
        capture_events: bool,
        capture_environment: bool,
    ) {
        semantic::configure_capture(&self.env, capture_events, capture_environment);
    }

    pub(crate) fn instrumentation_environment_clone_count(&self) -> u64 {
        semantic::environment_clone_count(&self.env)
    }

    /// Returns the cheap identity/data for the currently published semantic
    /// boundary without materializing source, frames, locals, or value displays.
    pub(crate) fn instrumentation_event(&self) -> Option<(usize, ProducerEvent)> {
        let boundary = semantic::current_boundary(&self.env)?;
        let kind = match boundary.rule {
            semantic::EvalSemanticRule::FormReturn | semantic::EvalSemanticRule::ValueReturn => {
                EventKind::SemanticBoundary
            }
            semantic::EvalSemanticRule::CallEnter => EventKind::CallEnter,
            semantic::EvalSemanticRule::CallReturn => EventKind::CallReturn,
            semantic::EvalSemanticRule::VarDefine | semantic::EvalSemanticRule::VarSet => {
                EventKind::VarSet
            }
            semantic::EvalSemanticRule::FieldSet => EventKind::FieldSet,
            semantic::EvalSemanticRule::ErrorRaise | semantic::EvalSemanticRule::ErrorCatch => {
                EventKind::ExceptionRaise
            }
        };
        let mut event = ProducerEvent::live(kind).with_data("rule", boundary.rule.as_keyword());
        match &boundary.payload {
            semantic::EvalSemanticPayload::Result(value) => {
                event = event.with_data("result/type", crate::core::portable_type_name(value));
                if boundary.rule == semantic::EvalSemanticRule::CallReturn {
                    if let Some(function) = &boundary.function {
                        event = event.with_data("function", function);
                    }
                }
            }
            semantic::EvalSemanticPayload::Call { name, arguments } => {
                event = event
                    .with_data("function", name)
                    .with_data("arguments/count", arguments.len().to_string());
            }
            semantic::EvalSemanticPayload::Effect {
                target,
                before,
                after,
            } => {
                event = event
                    .with_data("target", target)
                    .with_data("before/present", before.is_some().to_string())
                    .with_data("after/type", crate::core::portable_type_name(after));
            }
            semantic::EvalSemanticPayload::Error { message, caught } => {
                event = event
                    .with_data("caught", caught.to_string())
                    .with_data("message", bounded_text(message, 1_024));
            }
        }
        Some((boundary.sequence, event))
    }

    pub(crate) fn instrumentation_source_location(&self, source_id: &str) -> Option<EventLocation> {
        let boundary = semantic::current_boundary(&self.env)?;
        Some(EventLocation {
            source_id: Some(source_id.into()),
            function: boundary.function,
            ..EventLocation::default()
        })
    }

    pub(crate) fn instrumentation_current_frame(
        &self,
        limits: ProjectionLimits,
    ) -> Option<PortableProjection> {
        let boundary = semantic::current_boundary(&self.env)?;
        Some(environment_projection(
            "interpreter/current-frame",
            &boundary.environment,
            limits,
        ))
    }

    pub(crate) fn instrumentation_frames(
        &self,
        limits: ProjectionLimits,
    ) -> Option<PortableProjection> {
        let boundary = semantic::current_boundary(&self.env)?;
        let session = self.env.borrow();
        let mut projection = PortableProjection::new("interpreter/frames")
            .with_field("current/bindings", boundary.environment.len().to_string())
            .with_field("session/bindings", session.len().to_string());
        let current = environment_projection("current", &boundary.environment, limits);
        let session = environment_projection("session", &session, limits);
        for (name, value) in current.fields {
            projection.fields.insert(format!("current/{name}"), value);
        }
        for (name, value) in session.fields {
            projection.fields.insert(format!("session/{name}"), value);
        }
        Some(projection)
    }

    pub(crate) fn instrumentation_locals(
        &self,
        limits: ProjectionLimits,
    ) -> Option<PortableProjection> {
        let environment = self.env.borrow();
        Some(environment_projection(
            "interpreter/locals",
            &environment,
            limits,
        ))
    }

    pub(crate) fn instrumentation_value_preview(
        &self,
        limits: ProjectionLimits,
    ) -> Option<PortableProjection> {
        let boundary = semantic::current_boundary(&self.env)?;
        let display_chars = limits.max_bytes.min(16_384);
        let projection = match &boundary.payload {
            semantic::EvalSemanticPayload::Result(value) => {
                let kind = if boundary.rule == semantic::EvalSemanticRule::CallReturn {
                    "interpreter/call-return-preview"
                } else {
                    "interpreter/value-preview"
                };
                let mut projection = PortableProjection::new(kind)
                    .with_field("kind", crate::core::portable_type_name(value))
                    .with_field("display", bounded_text(&value.display(), display_chars));
                if let Some(function) = &boundary.function {
                    projection
                        .fields
                        .insert("function".into(), function.clone());
                }
                projection
            }
            semantic::EvalSemanticPayload::Call { name, arguments } => {
                let mut projection = PortableProjection::new("interpreter/call-preview")
                    .with_field("function", name)
                    .with_field("arguments/count", arguments.len().to_string());
                for (index, argument) in arguments.iter().take(limits.max_items).enumerate() {
                    projection.fields.insert(
                        format!("argument/{index}"),
                        bounded_text(&argument.display(), display_chars),
                    );
                }
                projection
            }
            semantic::EvalSemanticPayload::Effect {
                target,
                before,
                after,
            } => {
                let mut projection = PortableProjection::new("interpreter/effect-preview")
                    .with_field("target", target)
                    .with_field("after", bounded_text(&after.display(), display_chars));
                if let Some(before) = before {
                    projection.fields.insert(
                        "before".into(),
                        bounded_text(&before.display(), display_chars),
                    );
                }
                projection
            }
            semantic::EvalSemanticPayload::Error { message, caught } => {
                PortableProjection::new("interpreter/error-preview")
                    .with_field("caught", caught.to_string())
                    .with_field("message", bounded_text(message, display_chars))
            }
        };
        Some(projection)
    }

    pub(crate) fn instrumentation_snapshot(
        &self,
        limits: ProjectionLimits,
    ) -> Option<PortableProjection> {
        let mut projection = PortableProjection::new("interpreter/snapshot")
            .with_field("state", instrumentation_state_keyword(&self.state))
            .with_field(
                "semantic/pending",
                semantic::pending_count(&self.env).to_string(),
            )
            .with_field(
                "environment/clones",
                self.instrumentation_environment_clone_count().to_string(),
            );
        if let Some(promise) = &self.pending {
            projection.fields.insert(
                "promise/state".into(),
                promise_state_keyword(&promise.state()).into(),
            );
        }
        if let Some(locals) = self.instrumentation_locals(limits) {
            for (name, value) in locals.fields {
                projection.fields.insert(format!("locals/{name}"), value);
            }
        }
        Some(projection)
    }

    /// Returns true while an observed fiber owns a retained CPS continuation.
    pub fn observed_paused(&self) -> bool {
        matches!(self.state, EvalFiberState::Running)
            && self.pending.is_none()
            && self.resume.is_some()
    }

    /// Returns the number of semantic boundaries retained for host publication.
    pub fn observed_pending_boundaries(&self) -> usize {
        semantic::pending_count(&self.env)
    }

    /// Executes at most one retained production continuation boundary.
    pub fn step_observed(&mut self) -> EvalFiberState {
        if semantic::advance_pending(&self.env) {
            return self.state();
        }
        if !matches!(self.state, EvalFiberState::Running) {
            return self.state();
        }
        let Some(resume) = self.resume.take() else {
            self.state = EvalFiberState::Failed("observed evaluator continuation missing".into());
            return self.state();
        };
        let step = with_namespace_registry(&self.namespace_registry, || {
            semantic::with_active_context(&self.env, || resume(PromiseState::Pending))
        });
        self.accept_observed(step);
        semantic::advance_pending(&self.env);
        self.state()
    }

    /// Runs up to `boundary_limit` evaluator or queued semantic boundaries.
    pub fn run_observed(&mut self, boundary_limit: usize) -> EvalFiberState {
        for _ in 0..boundary_limit {
            if !matches!(self.state, EvalFiberState::Running)
                && semantic::pending_count(&self.env) == 0
            {
                break;
            }
            self.step_observed();
        }
        self.state()
    }

    /// Applies one real promise settlement without draining later boundaries.
    pub fn resume_observed(&mut self, state: PromiseState) -> EvalFiberState {
        if !matches!(self.state, EvalFiberState::Suspended) {
            return self.state();
        }
        let Some(resume) = self.resume.take() else {
            self.state = EvalFiberState::Failed("fiber continuation missing".into());
            return self.state();
        };
        self.pending = None;
        self.state = EvalFiberState::Running;
        let step = with_namespace_registry(&self.namespace_registry, || {
            semantic::with_active_context(&self.env, || resume(state))
        });
        self.accept_observed(step);
        semantic::advance_pending(&self.env);
        self.state()
    }

    fn accept_observed(&mut self, step: Step) {
        match step {
            Step::Continue(next) => {
                self.resume = Some(Box::new(move |_| next()));
                self.pending = None;
                self.state = EvalFiberState::Running;
            }
            Step::Done(Ok(value)) => {
                self.resume = None;
                self.pending = None;
                self.state = EvalFiberState::Completed(value);
            }
            Step::Done(Err(error)) => {
                self.resume = None;
                self.pending = None;
                self.state = EvalFiberState::Failed(error);
            }
            Step::Wait(promise, resume) => {
                self.pending = Some(promise);
                self.resume = Some(resume);
                self.state = EvalFiberState::Suspended;
            }
            Step::Yield(_, _) => {
                self.resume = None;
                self.pending = None;
                self.state =
                    EvalFiberState::Failed("coroutine/yield used outside of a coroutine".into());
            }
        }
    }
}

impl Drop for EvalFiber {
    fn drop(&mut self) {
        semantic::remove_context(&self.env);
    }
}

fn environment_projection(
    kind: &str,
    environment: &HashMap<String, Value>,
    limits: ProjectionLimits,
) -> PortableProjection {
    let mut entries = environment.iter().collect::<Vec<_>>();
    entries.sort_by(|left, right| left.0.cmp(right.0));
    let retained = entries.len().min(limits.max_items);
    let display_chars = limits.max_bytes.min(16_384);
    let mut fields = BTreeMap::new();
    for (name, value) in entries.into_iter().take(retained) {
        fields.insert(
            format!("binding/{name}"),
            bounded_text(&value.display(), display_chars),
        );
    }
    fields.insert("bindings/count".into(), environment.len().to_string());
    fields.insert(
        "bindings/omitted".into(),
        environment.len().saturating_sub(retained).to_string(),
    );
    PortableProjection {
        kind: kind.into(),
        fields,
    }
}

fn bounded_text(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.into();
    }
    let mut output = value.chars().take(limit).collect::<String>();
    output.push('…');
    output
}

fn instrumentation_state_keyword(state: &EvalFiberState) -> &'static str {
    match state {
        EvalFiberState::Running => "running",
        EvalFiberState::Suspended => "suspended",
        EvalFiberState::Completed(_) => "returned",
        EvalFiberState::Failed(_) => "failed",
        EvalFiberState::Cancelled => "cancelled",
    }
}

fn promise_state_keyword(state: &PromiseState) -> &'static str {
    match state {
        PromiseState::Pending => "pending",
        PromiseState::Fulfilled(_) => "fulfilled",
        PromiseState::Rejected(_) => "rejected",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_fiber_starts_paused_and_executes_one_trampoline_at_a_time() {
        let mut fiber =
            EvalFiber::start_observed("(do 1 2 (+ 1 (* 2 3)))", HashMap::new()).unwrap();
        assert_eq!(fiber.state(), EvalFiberState::Running);
        assert!(fiber.observed_paused());

        let first = fiber.run_observed(1);
        assert_eq!(first, EvalFiberState::Running);
        assert!(fiber.observed_paused());

        let mut boundaries = 1;
        while matches!(fiber.state(), EvalFiberState::Running) {
            fiber.step_observed();
            boundaries += 1;
            assert!(boundaries < 64, "observed evaluation did not terminate");
        }
        assert!(boundaries > 2);
        assert_eq!(fiber.state(), EvalFiberState::Completed(Value::Number(7)));
    }

    #[test]
    fn promise_suspension_retains_the_real_promise_and_resume_continuation() {
        let promise = Promise::new();
        let mut env = HashMap::new();
        env.insert("pending-value".into(), Value::Promise(promise.clone()));
        let mut fiber = EvalFiber::start_observed("(Coroutine/await pending-value)", env).unwrap();

        fiber.run_observed(16);
        assert_eq!(fiber.state(), EvalFiberState::Suspended);
        let retained = fiber.pending().expect("retained promise");
        assert!(retained.same_identity(&promise));

        assert!(promise.resolve(Value::Number(42)));
        let resumed = fiber.resume_observed(promise.state());
        assert_eq!(resumed, EvalFiberState::Running);
        assert!(fiber.observed_paused());

        let completed = fiber.run_observed(16);
        assert_eq!(completed, EvalFiberState::Completed(Value::Number(42)));
    }

    #[test]
    fn cancellation_discards_a_paused_live_continuation() {
        let mut fiber = EvalFiber::start_observed("(do 1 2 3)", HashMap::new()).unwrap();
        assert!(fiber.cancel());
        assert_eq!(fiber.state(), EvalFiberState::Cancelled);
        assert!(!fiber.observed_paused());
        assert_eq!(fiber.step_observed(), EvalFiberState::Cancelled);
        assert!(!fiber.cancel());
    }

    #[test]
    fn ordinary_eval_fiber_remains_full_speed() {
        let fiber = EvalFiber::start("(+ 19 23)", HashMap::new()).unwrap();
        assert_eq!(fiber.state(), EvalFiberState::Completed(Value::Number(42)));
    }

    #[test]
    fn disabled_instrumentation_capture_avoids_environment_clones() {
        let mut fiber = EvalFiber::start_observed("(+ 19 23)", HashMap::new()).unwrap();
        fiber.configure_instrumentation_capture(false, false);
        fiber.run_observed(32);
        assert_eq!(fiber.instrumentation_environment_clone_count(), 0);
        assert_eq!(fiber.state(), EvalFiberState::Completed(Value::Number(42)));
    }
}
