use super::*;

#[test]
fn protocol_resume_path_uses_the_coroutine_evaluator() {
    let fiber = EvalFiber::start(
        "(let [c (std.foundation.coroutine/create (fn [] 42))] \
           (std.protocol.icoroutine.ICoroutine/resume c))",
        HashMap::new(),
    )
    .unwrap();
    assert_eq!(fiber.state(), EvalFiberState::Completed(Value::Number(42)));
}

fn keyword(name: &str) -> Value {
    Value::Keyword(crate::lang::data::Keyword::from(name))
}

fn status_of(env: &HashMap<String, Value>, name: &str) -> Value {
    match env.get(name) {
        Some(Value::Var(var)) => match var.deref_value() {
            Value::Coroutine(coroutine) => coroutine_status(&coroutine),
            other => panic!("expected coroutine var, got {other:?}"),
        },
        Some(Value::Coroutine(coroutine)) => coroutine_status(&coroutine),
        other => panic!("expected coroutine binding for {name}, got {other:?}"),
    }
}

#[test]
fn create_makes_suspended_coroutine() {
    let mut f = EvalFiber::start(
        "(std.foundation.coroutine/coroutine? (std.foundation.coroutine/create (fn [x] x)))",
        HashMap::new(),
    )
    .unwrap();
    assert_eq!(f.state(), EvalFiberState::Completed(Value::Bool(true)));

    let mut f = EvalFiber::start(
        "(std.foundation.coroutine/status (std.foundation.coroutine/create (fn [x] x)))",
        HashMap::new(),
    )
    .unwrap();
    assert_eq!(f.state(), EvalFiberState::Completed(keyword("suspended")));
}

#[test]
fn resume_runs_body_to_completion() {
    let mut f = EvalFiber::start(
        "(do (def c (std.foundation.coroutine/create (fn [x] (* x 2)))) \
         (std.foundation.coroutine/resume c 21) \
         (std.foundation.coroutine/status c))",
        HashMap::new(),
    )
    .unwrap();
    assert_eq!(f.state(), EvalFiberState::Completed(keyword("dead")));
}

#[test]
fn resume_on_dead_throws() {
    let mut f = EvalFiber::start(
        "(do (def c (std.foundation.coroutine/create (fn [] 1))) \
         (std.foundation.coroutine/resume c) \
         (std.foundation.coroutine/resume c))",
        HashMap::new(),
    )
    .unwrap();
    assert!(matches!(f.state(), EvalFiberState::Failed(e) if e.contains("dead")));
}

#[test]
fn body_error_rethrows_at_resume_and_kills_coroutine() {
    let mut f = EvalFiber::start(
        "(do (def c (std.foundation.coroutine/create (fn [] (/ 1 0)))) \
         (std.foundation.coroutine/resume c))",
        HashMap::new(),
    )
    .unwrap();
    assert!(matches!(f.state(), EvalFiberState::Failed(_)));
    assert_eq!(status_of(&f.environment(), "c"), keyword("dead"));
}

#[test]
fn yield_exchanges_values_both_ways() {
    let mut f = EvalFiber::start(
        "(do (def c (std.foundation.coroutine/create \
         (fn [start] \
           (let [a (std.foundation.coroutine/yield (* start start))] \
             (let [b (std.foundation.coroutine/yield :second)] \
               [a b]))))) \
         [(std.foundation.coroutine/resume c 10) \
          (std.foundation.coroutine/resume c :got-a) \
          (std.foundation.coroutine/resume c :got-b) \
          (std.foundation.coroutine/status c)])",
        HashMap::new(),
    )
    .unwrap();
    assert_eq!(
        f.state(),
        EvalFiberState::Completed(Value::Vector(
            vec![
                Value::Number(100),
                keyword("second"),
                Value::Vector(vec![keyword("got-a"), keyword("got-b")].into()),
                keyword("dead"),
            ]
            .into()
        ))
    );
}

#[test]
fn multi_arg_resume_delivers_vector_to_yield() {
    let mut f = EvalFiber::start(
        "(do (def c (std.foundation.coroutine/create \
         (fn [] (let [v (std.foundation.coroutine/yield :first)] v)))) \
         [(std.foundation.coroutine/resume c) \
          (std.foundation.coroutine/resume c 9 8) \
          (std.foundation.coroutine/status c)])",
        HashMap::new(),
    )
    .unwrap();
    assert_eq!(
        f.state(),
        EvalFiberState::Completed(Value::Vector(
            vec![
                keyword("first"),
                Value::Vector(vec![Value::Number(9), Value::Number(8)].into()),
                keyword("dead"),
            ]
            .into()
        ))
    );
}

#[test]
fn close_on_never_resumed_coroutine() {
    let mut f = EvalFiber::start(
        "(do (def c (std.foundation.coroutine/create (fn [] :never-runs))) \
         (std.foundation.coroutine/coroutine? (std.foundation.coroutine/close c)) \
         (std.foundation.coroutine/status c))",
        HashMap::new(),
    )
    .unwrap();
    assert_eq!(f.state(), EvalFiberState::Completed(keyword("dead")));
}

#[test]
fn yield_requires_one_argument() {
    let mut f = EvalFiber::start("(std.foundation.coroutine/yield 1 2 3)", HashMap::new()).unwrap();
    assert_eq!(
        f.state(),
        EvalFiberState::Failed("function expects 1 arguments".into())
    );
}

#[test]
fn yield_works_from_nested_helper() {
    let mut f = EvalFiber::start(
        "(do (defn helper-n [x] (std.foundation.coroutine/yield (* x 10))) \
         (def c (std.foundation.coroutine/create (fn [] (helper-n 3) :end))) \
         [(std.foundation.coroutine/resume c) \
          (std.foundation.coroutine/resume c) \
          (std.foundation.coroutine/status c)])",
        HashMap::new(),
    )
    .unwrap();
    assert_eq!(
        f.state(),
        EvalFiberState::Completed(Value::Vector(
            vec![Value::Number(30), keyword("end"), keyword("dead"),].into()
        ))
    );
}

#[test]
fn yield_outside_coroutine_throws() {
    let mut f = EvalFiber::start("(std.foundation.coroutine/yield 1)", HashMap::new()).unwrap();
    assert!(matches!(f.state(), EvalFiberState::Failed(e) if e.contains("outside")));
}

#[test]
fn reentrant_resume_throws() {
    let mut f = EvalFiber::start(
        "(do (def c nil) \
         (set! c (std.foundation.coroutine/create (fn [] (std.foundation.coroutine/resume c)))) \
         (std.foundation.coroutine/resume c))",
        HashMap::new(),
    )
    .unwrap();
    assert!(matches!(f.state(), EvalFiberState::Failed(e) if e.contains("running")));
}

#[test]
fn nested_coroutines_resume_each_other() {
    let mut f = EvalFiber::start(
        "(do (def c-inner (std.foundation.coroutine/create \
         (fn [] (std.foundation.coroutine/yield :inner-yield) :inner-end))) \
         (def c-outer (std.foundation.coroutine/create \
         (fn [] \
           (std.foundation.coroutine/yield (std.foundation.coroutine/resume c-inner)) \
           (std.foundation.coroutine/yield (std.foundation.coroutine/resume c-inner :x)) \
           :outer-end))) \
         [(std.foundation.coroutine/resume c-outer) \
          (std.foundation.coroutine/resume c-outer) \
          (std.foundation.coroutine/resume c-outer) \
          (std.foundation.coroutine/status c-outer)])",
        HashMap::new(),
    )
    .unwrap();
    assert_eq!(
        f.state(),
        EvalFiberState::Completed(Value::Vector(
            vec![
                keyword("inner-yield"),
                keyword("inner-end"),
                keyword("outer-end"),
                keyword("dead"),
            ]
            .into()
        ))
    );
}

#[test]
fn generator_pipeline_produces_lazily() {
    let mut f = EvalFiber::start(
        "(do (def c (std.foundation.coroutine/create \
         (fn [n] (loop [i 0] \
           (if (< i n) \
             (do (std.foundation.coroutine/yield (* i i)) (recur (inc i))) \
             :done))))) \
         [(std.foundation.coroutine/resume c 3) \
          (std.foundation.coroutine/resume c) \
          (std.foundation.coroutine/resume c) \
          (std.foundation.coroutine/resume c) \
          (std.foundation.coroutine/status c)])",
        HashMap::new(),
    )
    .unwrap();
    assert_eq!(
        f.state(),
        EvalFiberState::Completed(Value::Vector(
            vec![
                Value::Number(0),
                Value::Number(1),
                Value::Number(4),
                keyword("done"),
                keyword("dead"),
            ]
            .into()
        ))
    );
}

#[test]
fn await_returns_settled_promise_value() {
    let mut f = EvalFiber::start(
        "(do (def c (std.foundation.coroutine/create \
         (fn [] (std.foundation.coroutine/await (promise/delay 50 (fn [] :delayed-value)))))) \
         (std.foundation.coroutine/resume c))",
        HashMap::new(),
    )
    .unwrap();
    assert_eq!(f.state(), EvalFiberState::Suspended);

    let promise = f.pending().unwrap();
    promise.resolve(keyword("delayed-value"));
    assert_eq!(
        f.resume(promise.state()),
        EvalFiberState::Completed(keyword("delayed-value"))
    );
    assert_eq!(status_of(&f.environment(), "c"), keyword("dead"));
}

#[test]
fn await_rethrows_promise_rejection() {
    let mut f = EvalFiber::start(
        "(do (def c (std.foundation.coroutine/create \
         (fn [] (std.foundation.coroutine/await (promise (fn [] (/ 1 0))))))) \
         (std.foundation.coroutine/resume c))",
        HashMap::new(),
    )
    .unwrap();
    assert!(
        matches!(f.state(), EvalFiberState::Failed(e) if e.contains("Promise rejected") || e.contains("division") || e.contains("zero"))
    );
    assert_eq!(status_of(&f.environment(), "c"), keyword("dead"));
}

#[test]
fn yield_passes_promise_object_without_awaiting() {
    let mut f = EvalFiber::start(
        "(do (def p (promise/new (fn [resolve reject] nil))) \
         (def c (std.foundation.coroutine/create \
           (fn [] (std.foundation.coroutine/yield p)))) \
         [(std.foundation.coroutine/resume c) \
          (std.foundation.coroutine/status c)])",
        HashMap::new(),
    )
    .unwrap();
    let state = f.state();
    assert!(
        matches!(&state, EvalFiberState::Completed(Value::Tuple(t)) if t.len() == 2),
        "expected completed 2-tuple, got {:?}",
        state
    );
    if let EvalFiberState::Completed(Value::Tuple(t)) = state {
        assert!(
            matches!(t.get(0), Some(Value::Promise(_))),
            "expected yielded value to be the promise object, got {:?}",
            t.get(0)
        );
        assert_eq!(t.get(1), Some(&keyword("suspended")));
    }
}

#[test]
fn yield_awaits_promise_when_composed_with_await() {
    let mut f = EvalFiber::start(
        "(do (def p (promise/new (fn [resolve reject] nil))) \
         (def c (std.foundation.coroutine/create \
           (fn [] (std.foundation.coroutine/yield (std.foundation.coroutine/await p))))) \
         (std.foundation.coroutine/resume c))",
        HashMap::new(),
    )
    .unwrap();
    assert_eq!(f.state(), EvalFiberState::Suspended);

    let promise = f.pending().unwrap();
    promise.resolve(keyword("resolved-value"));
    assert_eq!(
        f.resume(promise.state()),
        EvalFiberState::Completed(keyword("resolved-value"))
    );
    assert_eq!(status_of(&f.environment(), "c"), keyword("suspended"));
}
