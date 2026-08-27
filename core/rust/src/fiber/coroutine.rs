use super::*;

#[path = "coroutine/observation.rs"]
mod observation;
#[path = "coroutine/semantic.rs"]
pub(super) mod semantic;
#[path = "coroutine/snapshot.rs"]
mod snapshot;
// These are the private coroutine subsystem's shared API. Some build targets
// exercise only a subset, but keeping the re-export central avoids parallel
// snapshot type paths in observation and semantic instrumentation.
#[allow(unused_imports)]
pub use snapshot::{
    EvalBindingSnapshot, EvalErrorSnapshot, EvalFocusSnapshot, EvalFrameSnapshot,
    EvalObservationLimits, EvalObservationSnapshot, EvalObservationStatus, EvalObservedBoundary,
    EvalObservedBoundaryKind, EvalPendingSnapshot, EvalPositionSnapshot, EvalSemanticCallSnapshot,
    EvalSemanticEffectSnapshot, EvalSemanticErrorSnapshot, EvalSemanticSnapshot,
    EvalSourceSpanSnapshot, EvalValueSnapshot, INTERPRETER_LIVE_BOUNDARY_SCHEMA,
    INTERPRETER_LIVE_SNAPSHOT_SCHEMA,
};

pub fn pack_values(values: Vec<Value>) -> Result<Value, String> {
    match values.len() {
        0 => Ok(Value::Nil),
        1 => Ok(values.into_iter().next().unwrap()),
        _ => vector_literal(values),
    }
}

pub fn run_coroutine(step: Step, coroutine: Rc<Coroutine>, k: Cont) -> Step {
    match step {
        Step::Done(Ok(v)) => {
            *coroutine.state.borrow_mut() = CoroutineState::Dead;
            k(Ok(v))
        }
        Step::Done(Err(e)) => {
            *coroutine.state.borrow_mut() = CoroutineState::Dead;
            k(Err(e))
        }
        Step::Yield(value, resume) => {
            let trace = trace_stack_snapshot();
            *coroutine.state.borrow_mut() = CoroutineState::Suspended(Box::new(move |value| {
                let step = with_trace_stack(&trace, || resume(value));
                with_trace_stack_step(trace, step)
            }));
            k(Ok(value))
        }
        Step::Continue(next) => {
            let trace = trace_stack_snapshot();
            Step::Continue(Box::new(move || {
                let step = with_trace_stack(&trace, next);
                run_coroutine(step, coroutine, k)
            }))
        }
        Step::Wait(promise, resume) => {
            let trace = trace_stack_snapshot();
            Step::Wait(
                promise,
                Box::new(move |state| {
                    let step = with_trace_stack(&trace, || resume(state));
                    run_coroutine(step, coroutine, k)
                }),
            )
        }
    }
}

pub fn coroutine_resume(coroutine: Rc<Coroutine>, args: Vec<Value>, k: Cont) -> Step {
    let mut state = coroutine.state.borrow_mut();
    match std::mem::replace(&mut *state, CoroutineState::Running) {
        CoroutineState::New(body) => {
            drop(state);
            match body {
                Value::Function(f) => {
                    let step = call(f, args, Box::new(move |r| Step::Done(r)));
                    run_coroutine(step, coroutine, k)
                }
                _ => k(Err("coroutine/create expects a function".into())),
            }
        }
        CoroutineState::Suspended(resume) => {
            drop(state);
            match pack_values(args) {
                Ok(packed) => run_coroutine(resume(packed), coroutine, k),
                Err(e) => k(Err(e)),
            }
        }
        CoroutineState::Running => k(Err(
            "coroutine/resume: cannot resume a running coroutine".into()
        )),
        CoroutineState::Dead => k(Err(
            "coroutine/resume: cannot resume a dead coroutine".into()
        )),
    }
}

pub(crate) fn resume_sync(
    coroutine: Rc<Coroutine>,
    arguments: Vec<Value>,
) -> Result<Value, String> {
    let mut step = coroutine_resume(coroutine, arguments, Box::new(Step::Done));
    loop {
        match step {
            Step::Done(result) => return result,
            Step::Continue(next) => step = next(),
            Step::Wait(promise, resume) => {
                let state = promise.wait_state();
                if matches!(state, PromiseState::Pending) {
                    return Err(
                        "coroutine/resume cannot synchronously await a pending promise".into(),
                    );
                }
                step = resume(state);
            }
            Step::Yield(_, _) => {
                return Err("coroutine/yield escaped its coroutine boundary".into());
            }
        }
    }
}

pub fn resume_form(v: Vec<Form>, env: Rc<RefCell<HashMap<String, Value>>>, k: Cont) -> Step {
    if v.len() < 2 {
        return k(Err("coroutine/resume expects a coroutine".into()));
    }
    let arg_forms = v[2..].to_vec();
    let coroutine_form = v[1].clone();
    one(
        coroutine_form,
        env.clone(),
        Box::new(move |r| match r {
            Ok(Value::Coroutine(coroutine)) => values_cps(
                Rc::new(arg_forms),
                0,
                Vec::new(),
                env,
                Box::new(move |r| match r {
                    Ok(args) => coroutine_resume(coroutine, args, k),
                    Err(e) => k(Err(e)),
                }),
            ),
            Ok(_) => k(Err("coroutine/resume expects a coroutine".into())),
            Err(e) => k(Err(e)),
        }),
    )
}

pub fn resume_protocol_form(
    v: Vec<Form>,
    env: Rc<RefCell<HashMap<String, Value>>>,
    k: Cont,
) -> Step {
    if v.len() < 2 {
        return k(Err(
            "protocol/arity: ICoroutine/resume expects a receiver".into()
        ));
    }
    let arg_forms = v[2..].to_vec();
    one(
        v[1].clone(),
        env.clone(),
        Box::new(move |receiver| match receiver {
            Ok(Value::Coroutine(coroutine)) => values_cps(
                Rc::new(arg_forms),
                0,
                Vec::new(),
                env,
                Box::new(move |arguments| match arguments {
                    Ok(arguments) => coroutine_resume(coroutine, arguments, k),
                    Err(error) => k(Err(error)),
                }),
            ),
            Ok(receiver) => values_cps(
                Rc::new(arg_forms),
                0,
                Vec::new(),
                env,
                Box::new(move |arguments| match arguments {
                    Ok(mut arguments) => {
                        arguments.insert(0, receiver);
                        k(crate::core::protocol_call(
                            "std.protocol.icoroutine.ICoroutine",
                            "resume",
                            &arguments,
                        ))
                    }
                    Err(error) => k(Err(error)),
                }),
            ),
            Err(error) => k(Err(error)),
        }),
    )
}
