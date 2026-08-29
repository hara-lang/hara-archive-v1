use super::*;
use crate::lang::data::{List as PList, OrderedSet};

#[path = "fiber/coroutine.rs"]
pub(crate) mod coroutine;
#[cfg(test)]
#[path = "fiber/coroutine_tests.rs"]
mod coroutine_tests;

/// Core special forms that must be routed through the synchronous `eval` path
/// because they need unevaluated arguments, structural handling, or namespace
/// side effects. Forms with dedicated CPS arms in `list` are listed here too
/// so that they do not accidentally reach `application`, but the dedicated arms
/// take precedence.
const SYNC_SPECIAL_FORMS: &[&str] = &[
    ".",
    "binding",
    "comment",
    "declare",
    "def",
    "defmutable",
    "defstruct",
    "defprotocol",
    "defmulti",
    "defmethod",
    "defmacro",
    "defn",
    "defn-",
    "do",
    "eval",
    "extend-type",
    "field",
    "fn",
    "if",
    "intern-var",
    "let",
    "letfn",
    "loop",
    "ns",
    "ns+",
    "ns-alias-state",
    "read-forms",
    "recur",
    "require",
    "resolve",
    "set!",
    "syntax-quote",
    "throw",
    "try",
    "var",
    "var/set",
];

/// Completion names retained for the evaluator-free core surface.
///
/// This is a user-interface inventory, not an evaluator dispatch table. Actual
/// callable resolution goes through the Foundation namespace or the native
/// callable catalog; only `SYNC_SPECIAL_FORMS` controls fiber fallback.
pub(crate) const COMPLETION_SYMBOLS: &[&str] = &[
    "=",
    "+",
    "-",
    "*",
    "/",
    "%",
    "mod",
    "<",
    ">",
    "<=",
    ">=",
    ".",
    "abs",
    "acos",
    "acosh",
    "alter-var-root",
    "any?",
    "array",
    "atom",
    "asin",
    "asinh",
    "assoc",
    "assoc-in",
    "atan",
    "atan2",
    "atanh",
    "binding",
    "bit-and",
    "bit-or",
    "bit-xor",
    "bit-not",
    "bit-shift-left",
    "bit-shift-right",
    "bytes",
    "bytes/copy",
    "bytes/count",
    "bytes/get",
    "bytes/set",
    "bytes/s8",
    "bytes/slice",
    "bytes/u8",
    "cas!",
    "ceil",
    "char?",
    "comp",
    "comp2",
    "comp3",
    "complement",
    "concat",
    "conj",
    "cons",
    "constantly",
    "cos",
    "cosh",
    "ns-current",
    "cycle",
    "dec",
    "declare",
    "def",
    "defmacro",
    "defmethod",
    "defmulti",
    "defn",
    "defn-",
    "do",
    "drop",
    "drop-while",
    "double?",
    "empty",
    "empty?",
    "eval",
    "eval-in-ns",
    "even?",
    "every?",
    "exp",
    "false?",
    "file/read",
    "file/join",
    "file/resolve",
    "file/write",
    "file/exists?",
    "file/stat",
    "file/entries",
    "file/list",
    "file/walk",
    "file/mkdir",
    "file/delete",
    "file/copy",
    "file/move",
    "file/temp-file",
    "file/temp-directory",
    "filter",
    "field",
    "first",
    "floor",
    "fn",
    "hash",
    "identity",
    "if",
    "inc",
    "instance?",
    "intern-var",
    "interleave",
    "interpose",
    "iter",
    "iter-close",
    "iter-concat",
    "iter-cycle",
    "iter-drop",
    "iter-drop-while",
    "iter-every?",
    "iter-any?",
    "iter-finite?",
    "iter-next?",
    "iter-interleave",
    "iter-interpose",
    "iter-iterate",
    "iter-keep",
    "iter-map",
    "iter-mapcat",
    "iter-materialize",
    "iter-next",
    "iter-partition-pair",
    "iter-partition",
    "iter-partition-all",
    "iter-range",
    "iter-repeatedly",
    "iter-constantly",
    "iter-filter",
    "iter-take",
    "iter-take-while",
    "iter-zip",
    "iter?",
    "iterate",
    "keep",
    "key",
    "keys",
    "keyword",
    "keyword?",
    "last",
    "let",
    "letfn",
    "list",
    "list?",
    "load-string",
    "long?",
    "bigint?",
    "integer?",
    "loop",
    "map",
    "map?",
    "mapcat",
    "neg?",
    "name",
    "namespace",
    "nil?",
    "number?",
    "ns",
    "ns-alias-state",
    "ns-loaded?",
    "ns-state",
    "ns-create",
    "ns-find",
    "ns-info",
    "ns-list",
    "ns-aliases",
    "ns-name",
    "ns-publics",
    "ns-vars",
    "nth",
    "not",
    "peek",
    "not-empty",
    "object",
    "odd?",
    "p",
    "pair",
    "partition-pair",
    "partition",
    "partition-all",
    "pointer",
    "pos?",
    "pow",
    "quot",
    "pr-str",
    "capture",
    "Printer/capture",
    "println",
    "promise",
    "promise/run",
    "promise?",
    "promise/all",
    "promise/cancel",
    "promise/delay",
    "promise/from",
    "promise/new",
    "range",
    "read-forms",
    "read-string",
    "recur",
    "repeat",
    "repeatedly",
    "require",
    "resolve",
    "rem",
    "reset!",
    "rest",
    "reverse",
    "second",
    "seq",
    "seq?",
    "set!",
    "set?",
    "string?",
    "symbol?",
    "swap!",
    "sin",
    "sinh",
    "socket/close",
    "socket/connect",
    "socket/send",
    "str",
    "str/decode-utf8",
    "str/encode-utf8",
    "str/length",
    "str/blank?",
    "str/includes?",
    "str/starts-with?",
    "str/ends-with?",
    "str/char-at",
    "str/slice",
    "str/index-of",
    "str/last-index-of",
    "str/split",
    "str/split-lines",
    "str/join",
    "str/repeat",
    "str/replace",
    "str/replace-first",
    "str/trim-left",
    "str/trim-right",
    "str/upper",
    "str/lower",
    "str/capitalize",
    "str/decapitalize",
    "str/pad-left",
    "str/pad-right",
    "str/reverse",
    "sqrt",
    "symbol",
    "take",
    "take-while",
    "tan",
    "tanh",
    "throw",
    "true?",
    "try",
    "tup",
    "update",
    "update-in",
    "val",
    "vals",
    "var",
    "var-sym",
    "var/set",
    "vector",
    "vector?",
    "fn?",
    "function?",
    "hash-map",
    "hash-set",
    "zero?",
    "zip",
    "__map-transform",
    "__iterator-transform",
];

pub(crate) fn completion_symbols() -> &'static [&'static str] {
    COMPLETION_SYMBOLS
}

pub(crate) type Cont = Box<dyn FnOnce(Result<Value, String>) -> Step>;
pub type Resume = Box<dyn FnOnce(PromiseState) -> Step>;
pub enum Step {
    Done(Result<Value, String>),
    Wait(Promise, Resume),
    Yield(Value, Box<dyn FnOnce(Value) -> Step>),
    /// Defers the next synchronous continuation to the fiber driver.  Without
    /// this trampoline, a document with many top-level forms keeps one Rust
    /// stack frame per completed form (and can exhaust the smaller WASM stack).
    Continue(Box<dyn FnOnce() -> Step>),
}

fn with_trace_stack_step(trace: Vec<String>, step: Step) -> Step {
    match step {
        Step::Continue(next) => Step::Continue(Box::new(move || {
            let step = with_trace_stack(&trace, next);
            with_trace_stack_step(trace, step)
        })),
        Step::Wait(promise, resume) => Step::Wait(
            promise,
            Box::new(move |state| {
                let step = with_trace_stack(&trace, || resume(state));
                with_trace_stack_step(trace, step)
            }),
        ),
        Step::Yield(value, resume) => Step::Yield(
            value,
            Box::new(move |value| {
                let step = with_trace_stack(&trace, || resume(value));
                with_trace_stack_step(trace, step)
            }),
        ),
        Step::Done(result) => Step::Done(result),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum EvalFiberState {
    Running,
    Suspended,
    Completed(Value),
    Failed(String),
    Cancelled,
}

pub struct EvalFiber {
    env: Rc<RefCell<HashMap<String, Value>>>,
    namespace_registry: NamespaceRegistry<Value>,
    pending: Option<Promise>,
    resume: Option<Resume>,
    state: EvalFiberState,
}
impl EvalFiber {
    pub fn start(source: &str, env: HashMap<String, Value>) -> Result<Self, String> {
        let forms = crate::kernel::read_forms(source).map_err(|error| error.to_string())?;
        Self::start_forms(
            forms
                .iter()
                .map(crate::core::attach_exception_sites)
                .collect(),
            env,
        )
    }
    pub fn start_forms(forms: Vec<Form>, env: HashMap<String, Value>) -> Result<Self, String> {
        let (namespace_registry, environment) = execution_context(env);
        let env = Rc::new(RefCell::new(environment));
        let step = with_namespace_registry(&namespace_registry, || {
            forms_cps(
                Rc::new(forms),
                0,
                Value::Nil,
                env.clone(),
                Box::new(Step::Done),
            )
        });
        let mut fiber = Self {
            env,
            namespace_registry,
            pending: None,
            resume: None,
            state: EvalFiberState::Running,
        };
        fiber.accept(step);
        Ok(fiber)
    }
    pub fn state(&self) -> EvalFiberState {
        self.state.clone()
    }
    pub fn pending(&self) -> Option<Promise> {
        self.pending.clone()
    }
    pub fn environment(&self) -> HashMap<String, Value> {
        self.env.borrow().clone()
    }
    pub fn resume(&mut self, state: PromiseState) -> EvalFiberState {
        if !matches!(self.state, EvalFiberState::Suspended) {
            return self.state();
        }
        let Some(resume) = self.resume.take() else {
            self.state = EvalFiberState::Failed("fiber continuation missing".into());
            return self.state();
        };
        self.pending = None;
        self.state = EvalFiberState::Running;
        let step = with_namespace_registry(&self.namespace_registry, || resume(state));
        self.accept(step);
        self.state()
    }
    pub fn cancel(&mut self) -> bool {
        if matches!(
            self.state,
            EvalFiberState::Completed(_) | EvalFiberState::Failed(_) | EvalFiberState::Cancelled
        ) {
            return false;
        }
        if let Some(pending) = self.pending.take() {
            pending.notify_cancel();
        }
        self.resume = None;
        self.state = EvalFiberState::Cancelled;
        true
    }
    pub fn drive_sync(&mut self) -> Result<Value, String> {
        loop {
            match self.state() {
                EvalFiberState::Completed(v) => return Ok(v),
                EvalFiberState::Failed(e) => return Err(e),
                EvalFiberState::Cancelled => return Err("eval cancelled".into()),
                EvalFiberState::Running => return Err("fiber is running".into()),
                EvalFiberState::Suspended => {
                    let Some(pending) = self.pending() else {
                        return Err("fiber suspended without promise".into());
                    };
                    match pending.wait_state() {
                        PromiseState::Fulfilled(v) => {
                            self.resume(PromiseState::Fulfilled(v));
                        }
                        PromiseState::Rejected(e) => {
                            self.resume(PromiseState::Rejected(e));
                        }
                        PromiseState::Pending => {
                            #[cfg(not(target_arch = "wasm32"))]
                            self.resume(pending.wait_state());
                            #[cfg(target_arch = "wasm32")]
                            return Err(
                                "deref cannot block on a pending promise outside an HTA fiber"
                                    .into(),
                            );
                        }
                    }
                }
            }
        }
    }
    fn accept(&mut self, mut step: Step) {
        loop {
            match step {
                Step::Continue(next) => {
                    step = with_namespace_registry(&self.namespace_registry, next)
                }
                Step::Done(Ok(v)) => {
                    self.state = EvalFiberState::Completed(v);
                    return;
                }
                Step::Done(Err(e)) => {
                    self.state = EvalFiberState::Failed(e);
                    return;
                }
                Step::Wait(p, r) => {
                    self.pending = Some(p);
                    self.resume = Some(r);
                    self.state = EvalFiberState::Suspended;
                    return;
                }
                Step::Yield(_, _) => {
                    self.state = EvalFiberState::Failed(
                        "coroutine/yield used outside of a coroutine".into(),
                    );
                    return;
                }
            }
        }
    }
}

fn forms_cps(
    forms: Rc<Vec<Form>>,
    i: usize,
    last: Value,
    env: Rc<RefCell<HashMap<String, Value>>>,
    k: Cont,
) -> Step {
    if i == forms.len() || matches!(last, Value::Recur(_)) {
        return k(Ok(last));
    }
    let next = forms.clone();
    let e = env.clone();
    let form = forms[i].clone();
    let boundary_form = form.clone();
    let boundary_env = env.clone();
    one(
        form,
        env,
        Box::new(move |r| match r {
            Ok(v) => {
                coroutine::semantic::record_boundary(
                    coroutine::semantic::EvalSemanticRule::FormReturn,
                    &boundary_form,
                    &v,
                    &boundary_env,
                );
                Step::Continue(Box::new(move || forms_cps(next, i + 1, v, e, k)))
            }
            Err(x) => {
                coroutine::semantic::record_error(
                    coroutine::semantic::EvalSemanticRule::ErrorRaise,
                    &boundary_form,
                    &x,
                    false,
                    &boundary_env,
                );
                k(Err(x))
            }
        }),
    )
}
fn values_cps(
    forms: Rc<Vec<Form>>,
    i: usize,
    values: Vec<Value>,
    env: Rc<RefCell<HashMap<String, Value>>>,
    k: Box<dyn FnOnce(Result<Vec<Value>, String>) -> Step>,
) -> Step {
    if i == forms.len() {
        return k(Ok(values));
    }
    let next = forms.clone();
    let e = env.clone();
    let form = forms[i].clone();
    let boundary_form = form.clone();
    let boundary_env = env.clone();
    one(
        form,
        env,
        Box::new(move |r| match r {
            Ok(v) => {
                coroutine::semantic::record_boundary(
                    coroutine::semantic::EvalSemanticRule::ValueReturn,
                    &boundary_form,
                    &v,
                    &boundary_env,
                );
                let mut values = values;
                values.push(v);
                Step::Continue(Box::new(move || values_cps(next, i + 1, values, e, k)))
            }
            Err(x) => {
                coroutine::semantic::record_error(
                    coroutine::semantic::EvalSemanticRule::ErrorRaise,
                    &boundary_form,
                    &x,
                    false,
                    &boundary_env,
                );
                k(Err(x))
            }
        }),
    )
}
fn one(form: Form, env: Rc<RefCell<HashMap<String, Value>>>, k: Cont) -> Step {
    if let Err(error) = super::check_evaluation_interrupt() {
        return k(Err(error));
    }
    match form {
        Form::Map(entries) => {
            let flat = Rc::new(entries.into_iter().flat_map(|(a, b)| [a, b]).collect());
            values_cps(
                flat,
                0,
                Vec::new(),
                env,
                Box::new(move |r| {
                    k(r.map(|v| {
                        Value::Map(
                            v.chunks_exact(2)
                                .map(|p| (p[0].clone(), p[1].clone()))
                                .collect::<PMap<Value, Value>>(),
                        )
                    }))
                }),
            )
        }
        Form::Set(v) => values_cps(
            Rc::new(v),
            0,
            Vec::new(),
            env,
            Box::new(move |r| {
                k(r.map(|v| {
                    Value::OrderedSet(Box::new(
                        unique_values(v).into_iter().collect::<OrderedSet<Value>>(),
                    ))
                }))
            }),
        ),
        Form::Vector(v) => values_cps(
            Rc::new(v),
            0,
            Vec::new(),
            env,
            Box::new(move |r| k(r.and_then(vector_literal))),
        ),
        Form::List(v) if v.is_empty() => k(Ok(Value::List(PList::new()))),
        Form::List(v) if v.len() == 2 && matches!(&v[0],Form::Symbol(n)if n=="quote") => {
            k(literal_value(&v[1]))
        }
        Form::List(v) => list(v, env, k),
        simple => sync(simple, env, k),
    }
}
fn sync(form: Form, env: Rc<RefCell<HashMap<String, Value>>>, k: Cont) -> Step {
    let result = {
        let mut borrowed = env.borrow_mut();
        eval(&form, &mut borrowed)
    };
    k(result)
}
fn list(v: Vec<Form>, env: Rc<RefCell<HashMap<String, Value>>>, k: Cont) -> Step {
    let head = match &v[0] {
        Form::Symbol(n) => Some(n.as_str()),
        _ => None,
    };
    match head {
        Some("do") => forms_cps(Rc::new(v[1..].to_vec()), 0, Value::Nil, env, k),
        Some("if") => {
            if v.len() != 3 && v.len() != 4 {
                return k(Err("if expects 2 or 3 arguments".into()));
            }
            let vv = v.clone();
            let e = env.clone();
            one(
                v[1].clone(),
                env,
                Box::new(move |r| match r {
                    Ok(x) if x.truthy() => one(vv[2].clone(), e, k),
                    Ok(_) if vv.len() == 4 => one(vv[3].clone(), e, k),
                    Ok(_) => k(Ok(Value::Nil)),
                    Err(x) => k(Err(x)),
                }),
            )
        }
        Some("and") => and_cps(Rc::new(v[1..].to_vec()), 0, Value::Bool(true), env, k),
        Some("or") => or_cps(Rc::new(v[1..].to_vec()), 0, Value::Nil, env, k),
        Some("cond") => {
            if v.len() % 2 == 0 {
                return k(Err("cond expects test/expression pairs".into()));
            }
            cond_cps(Rc::new(v[1..].to_vec()), 0, env, k)
        }
        Some("let") => scoped(v, env, k, false),
        Some("loop") => scoped(v, env, k, true),
        Some("recur") => values_cps(
            Rc::new(v[1..].to_vec()),
            0,
            Vec::new(),
            env,
            Box::new(move |r| k(r.map(Value::Recur))),
        ),
        Some("try") => try_cps(v, env, k),
        Some("throw") => {
            if v.len() != 2 {
                return k(Err("throw expects one value".into()));
            }
            one(
                v[1].clone(),
                env,
                Box::new(move |r| match r {
                    Ok(x) if matches!(x, Value::ExceptionInfo(_)) => k(Err(thrown_error(x))),
                    Ok(_) => k(Err("throw expects an Exception value created by ex".into())),
                    Err(x) => k(Err(x)),
                }),
            )
        }
        Some("std.foundation.coroutine/resume") => coroutine::resume_form(v, env, k),
        Some("std.protocol.icoroutine.ICoroutine/resume") => {
            coroutine::resume_protocol_form(v, env, k)
        }
        Some("def") | Some("var/set") => bind_form(v, env, k),
        Some("set!") => set_form(v, env, k),
        Some("resolve") if matches!(env.borrow().get("resolve"), Some(value) if !matches!(value, Value::Var(_))) => {
            application(v, env, k)
        }
        Some(name) if SYNC_SPECIAL_FORMS.contains(&name) => sync(Form::List(v), env, k),
        _ => application(v, env, k),
    }
}

fn and_cps(
    forms: Rc<Vec<Form>>,
    index: usize,
    last: Value,
    env: Rc<RefCell<HashMap<String, Value>>>,
    k: Cont,
) -> Step {
    if index == forms.len() || !last.truthy() {
        return k(Ok(last));
    }
    let next = forms.clone();
    let e = env.clone();
    one(
        forms[index].clone(),
        env,
        Box::new(move |result| match result {
            Ok(value) => Step::Continue(Box::new(move || and_cps(next, index + 1, value, e, k))),
            Err(error) => k(Err(error)),
        }),
    )
}

fn or_cps(
    forms: Rc<Vec<Form>>,
    index: usize,
    last: Value,
    env: Rc<RefCell<HashMap<String, Value>>>,
    k: Cont,
) -> Step {
    if index == forms.len() {
        return k(Ok(last));
    }
    let next = forms.clone();
    let e = env.clone();
    one(
        forms[index].clone(),
        env,
        Box::new(move |result| match result {
            Ok(value) if value.truthy() => k(Ok(value)),
            Ok(value) => Step::Continue(Box::new(move || or_cps(next, index + 1, value, e, k))),
            Err(error) => k(Err(error)),
        }),
    )
}

fn cond_cps(
    clauses: Rc<Vec<Form>>,
    index: usize,
    env: Rc<RefCell<HashMap<String, Value>>>,
    k: Cont,
) -> Step {
    if index == clauses.len() {
        return k(Ok(Value::Nil));
    }
    let next = clauses.clone();
    let e = env.clone();
    one(
        clauses[index].clone(),
        env,
        Box::new(move |result| match result {
            Ok(value) if value.truthy() => one(next[index + 1].clone(), e, k),
            Ok(_) => Step::Continue(Box::new(move || cond_cps(next, index + 2, e, k))),
            Err(error) => k(Err(error)),
        }),
    )
}

type Previous = Vec<(String, Option<Value>)>;
fn bindings(forms: &[Form], op: &str) -> Result<Vec<Form>, String> {
    let v = match forms.get(1) {
        Some(Form::List(v)) | Some(Form::Vector(v)) => v.clone(),
        _ => return Err(format!("{op} expects bindings")),
    };
    if v.len() % 2 != 0 {
        return Err(format!("{op} bindings require name/value pairs"));
    }
    Ok(v)
}
fn bind_values(
    v: Rc<Vec<Form>>,
    i: usize,
    old: Previous,
    env: Rc<RefCell<HashMap<String, Value>>>,
    k: Box<dyn FnOnce(Result<Previous, String>, Rc<RefCell<HashMap<String, Value>>>) -> Step>,
) -> Step {
    if i == v.len() {
        return k(Ok(old), env);
    }
    let pattern = v[i].clone();
    let vv = v.clone();
    let e = env.clone();
    one(
        v[i + 1].clone(),
        env,
        Box::new(move |r| match r {
            Ok(x) => {
                let mut old = old;
                let before = e.borrow().clone();
                let mut names = Vec::new();
                let binding = {
                    let mut environment = e.borrow_mut();
                    crate::core::bind_pattern(&pattern, x, &mut environment, &mut names, None)
                };
                if let Err(error) = binding {
                    return k(Err(format!("destructuring failed: {error}")), e);
                }
                for name in names {
                    old.push((name.clone(), before.get(&name).cloned()));
                }
                Step::Continue(Box::new(move || bind_values(vv, i + 2, old, e, k)))
            }
            Err(x) => k(Err(x), e),
        }),
    )
}
fn restore(env: &mut HashMap<String, Value>, old: Previous) {
    for (n, v) in old.into_iter().rev() {
        if let Some(v) = v {
            env.insert(n, v);
        } else {
            env.remove(&n);
        }
    }
}
fn scoped(v: Vec<Form>, env: Rc<RefCell<HashMap<String, Value>>>, k: Cont, is_loop: bool) -> Step {
    if v.len() < 3 {
        return k(Err("binding form expects bindings and body".into()));
    }
    let b = match bindings(&v, if is_loop { "loop" } else { "let" }) {
        Ok(x) => x,
        Err(x) => return k(Err(x)),
    };
    let patterns = Rc::new(b.chunks(2).map(|pair| pair[0].clone()).collect());
    let body = if v.len() == 3 {
        v[2].clone()
    } else {
        Form::List(
            std::iter::once(Form::Symbol("do".into()))
                .chain(v[2..].iter().cloned())
                .collect(),
        )
    };
    bind_values(
        Rc::new(b),
        0,
        Vec::new(),
        env,
        Box::new(move |r, e| match r {
            Ok(old) if is_loop => loop_body(patterns, body, old, e, k),
            Ok(old) => {
                let re = e.clone();
                one(
                    body,
                    e,
                    Box::new(move |r| {
                        restore(&mut re.borrow_mut(), old);
                        k(r)
                    }),
                )
            }
            Err(x) => k(Err(x)),
        }),
    )
}
fn loop_body(
    patterns: Rc<Vec<Form>>,
    body: Form,
    old: Previous,
    env: Rc<RefCell<HashMap<String, Value>>>,
    k: Cont,
) -> Step {
    let pp = patterns.clone();
    let bb = body.clone();
    let oo = old.clone();
    let ee = env.clone();
    one(
        body,
        env,
        Box::new(move |r| match r {
            Ok(Value::Recur(v)) => {
                if v.len() != pp.len() {
                    restore(&mut ee.borrow_mut(), oo);
                    return k(Err("loop recur arity mismatch".into()));
                }
                for (pattern, value) in pp.iter().zip(v) {
                    let mut names = Vec::new();
                    if let Err(error) = crate::core::bind_pattern(
                        pattern,
                        value,
                        &mut ee.borrow_mut(),
                        &mut names,
                        None,
                    ) {
                        restore(&mut ee.borrow_mut(), oo);
                        return k(Err(format!("loop destructuring failed: {error}")));
                    }
                }
                Step::Continue(Box::new(move || loop_body(pp, bb, oo, ee, k)))
            }
            r => {
                restore(&mut ee.borrow_mut(), oo);
                k(r)
            }
        }),
    )
}

fn set_form(v: Vec<Form>, env: Rc<RefCell<HashMap<String, Value>>>, k: Cont) -> Step {
    let effect_form = Form::List(v.clone());
    if v.len() != 3 {
        return k(Err("set! expects a place and value".into()));
    }
    if matches!(&v[1], Form::Symbol(_) | Form::Metadata(_, _)) {
        return bind_form(v, env, k);
    }
    let Form::List(place) = &v[1] else {
        return k(Err("set! expects a name symbol or field place".into()));
    };
    if !matches!(place.first(), Some(Form::Symbol(operation)) if operation == "field") {
        return k(Err("set! expects a name symbol or field place".into()));
    }
    if place.len() != 3 {
        return k(Err("set! field place expects a receiver and field".into()));
    }
    let field = match &place[2] {
        Form::Keyword(field) | Form::Symbol(field) if !field.contains('/') => field.clone(),
        _ => {
            return k(Err(
                "set! field place expects an unqualified literal field".into()
            ))
        }
    };
    let receiver = place[1].clone();
    let replacement = v[2].clone();
    let replacement_env = env.clone();
    let effect_env = replacement_env.clone();
    one(
        receiver,
        env,
        Box::new(move |receiver_result| match receiver_result {
            Ok(receiver) => one(
                replacement,
                replacement_env,
                Box::new(move |replacement_result| match replacement_result {
                    Ok(replacement) => {
                        let after = replacement.clone();
                        match crate::core::mutable_field_set(&receiver, &field, replacement) {
                            Ok(result) => {
                                coroutine::semantic::record_effect(
                                    coroutine::semantic::EvalSemanticRule::FieldSet,
                                    &effect_form,
                                    field,
                                    None,
                                    after,
                                    &effect_env,
                                );
                                k(Ok(result))
                            }
                            Err(error) => k(Err(error)),
                        }
                    }
                    Err(error) => k(Err(error)),
                }),
            ),
            Err(error) => k(Err(error)),
        }),
    )
}

fn bind_form(v: Vec<Form>, env: Rc<RefCell<HashMap<String, Value>>>, k: Cont) -> Step {
    let effect_form = Form::List(v.clone());
    if v.len() != 3 {
        return k(Err("binding form expects symbol and value".into()));
    }
    let op = match &v[0] {
        Form::Symbol(n) => n.clone(),
        _ => unreachable!(),
    };
    let (name, metadata) = match &v[1] {
        Form::Symbol(n) => (n.clone(), None),
        Form::Metadata(meta, value) => match value.as_ref() {
            Form::Symbol(n) => match crate::core::metadata_from_form(meta) {
                Ok(metadata) => (n.clone(), Some(metadata)),
                Err(error) => return k(Err(error)),
            },
            _ => return k(Err(format!("{op} name must be a symbol"))),
        },
        _ => return k(Err(format!("{op} name must be a symbol"))),
    };
    let e = env.clone();
    one(
        v[2].clone(),
        env,
        Box::new(move |r| match r {
            Ok(x) => {
                let effect_after = x.clone();
                let mut effect_before = None;
                let effect_target;
                let mut env = e.borrow_mut();
                let result = if op == "def" {
                    let origin = crate::core::definition_origin();
                    let var = if let Some(Value::Var(var)) = env.get(&name) {
                        effect_before = Some(var.deref_value());
                        if crate::core::binding_is_local(var) {
                            var.reset_value(x.clone());
                            var.set_origin(origin);
                            if let Some(meta) = &metadata {
                                var.set_hara_metadata(Some(meta.clone()));
                            }
                            var.clone()
                        } else {
                            let var = crate::kernel::Var::new(
                                crate::core::local_var_name(&name),
                                x.clone(),
                            );
                            var.set_origin(origin);
                            if let Some(meta) = &metadata {
                                var.set_hara_metadata(Some(meta.clone()));
                            }
                            env.insert(name.clone(), Value::Var(var.clone()));
                            var
                        }
                    } else {
                        let var =
                            crate::kernel::Var::new(crate::core::local_var_name(&name), x.clone());
                        var.set_origin(origin);
                        if let Some(meta) = &metadata {
                            var.set_hara_metadata(Some(meta.clone()));
                        }
                        env.insert(name.clone(), Value::Var(var.clone()));
                        var
                    };
                    effect_target = var.display();
                    Value::Var(var)
                } else {
                    let Some(c) = binding_var(&mut env, &name) else {
                        return k(Err(format!("unbound var: {name}")));
                    };
                    effect_before = Some(c.deref_value());
                    effect_target = c.display();
                    c.reset_value(x.clone());
                    if let Some(meta) = metadata {
                        c.set_hara_metadata(Some(meta));
                    }
                    x
                };
                drop(env);
                coroutine::semantic::record_effect(
                    if op == "def" {
                        coroutine::semantic::EvalSemanticRule::VarDefine
                    } else {
                        coroutine::semantic::EvalSemanticRule::VarSet
                    },
                    &effect_form,
                    effect_target,
                    effect_before,
                    effect_after,
                    &e,
                );
                k(Ok(result))
            }
            Err(x) => k(Err(x)),
        }),
    )
}

fn try_cps(v: Vec<Form>, env: Rc<RefCell<HashMap<String, Value>>>, k: Cont) -> Step {
    let mut body = Vec::new();
    let mut catches = Vec::new();
    let mut finals = Vec::new();
    let mut clauses_started = false;
    for f in v.into_iter().skip(1) {
        match &f {
            Form::List(p) if !p.is_empty() && matches!(&p[0],Form::Symbol(n)if n=="catch") => {
                clauses_started = true;
                catches.push(p.clone())
            }
            Form::List(p) if !p.is_empty() && matches!(&p[0],Form::Symbol(n)if n=="finally") => {
                clauses_started = true;
                finals.extend_from_slice(&p[1..])
            }
            _ if !clauses_started => body.push(f),
            _ => return k(Err("try clauses must follow body".into())),
        }
    }
    let e = env.clone();
    forms_cps(
        Rc::new(body),
        0,
        Value::Nil,
        env,
        Box::new(move |r| finish_try(r, catches, finals, e, k)),
    )
}
fn finish_try(
    r: Result<Value, String>,
    catches: Vec<Vec<Form>>,
    finals: Vec<Form>,
    env: Rc<RefCell<HashMap<String, Value>>>,
    k: Cont,
) -> Step {
    match r {
        Err(x) => {
            let mut selected = None;
            let mut saw_unconditional = false;
            for parts in catches {
                if saw_unconditional {
                    return k(Err(
                        "unconditional catch must be the last catch clause".into()
                    ));
                }
                let parsed = match parse_catch_clause(&parts) {
                    Ok(parsed) => parsed,
                    Err(error) => return k(Err(error)),
                };
                saw_unconditional = parsed.0.is_none();
                if parsed
                    .0
                    .as_deref()
                    .is_none_or(|selector| crate::core::catch_matches(&x, selector))
                {
                    selected = Some((parts, parsed.1, parsed.2));
                    break;
                }
            }
            let Some((p, binding_index, body_index)) = selected else {
                return finally(Err(x), finals, env, k);
            };
            let catch_form = Form::List(p.clone());
            let n = match &p[binding_index] {
                Form::Symbol(n) => n.clone(),
                _ => return k(Err("catch name must be symbol".into())),
            };
            let old = env.borrow_mut().insert(n.clone(), caught_error(&x));
            coroutine::semantic::record_error(
                coroutine::semantic::EvalSemanticRule::ErrorCatch,
                &catch_form,
                &x,
                true,
                &env,
            );
            let e = env.clone();
            forms_cps(
                Rc::new(p[body_index..].to_vec()),
                0,
                Value::Nil,
                env,
                Box::new(move |r| {
                    restore(&mut e.borrow_mut(), vec![(n, old)]);
                    finally(r, finals, e, k)
                }),
            )
        }
        result => finally(result, finals, env, k),
    }
}

fn parse_catch_clause(parts: &[Form]) -> Result<(Option<String>, usize, usize), String> {
    match parts {
        [_, Form::Symbol(name), _] if name != "Exception" && name != "Throwable" => {
            Ok((None, 1, 2))
        }
        [_, Form::Symbol(name), body, ..]
            if name != "Exception"
                && name != "Throwable"
                && !matches!(body, Form::Symbol(_)) =>
        {
            Ok((None, 1, 2))
        }
        [_, Form::Symbol(class), Form::Symbol(_), _, ..] => {
            Ok((Some(class.clone()), 2, 3))
        }
        [_, Form::Keyword(code), Form::Symbol(_), _, ..] if code.contains('/') => {
            Ok((Some(format!(":{code}")), 2, 3))
        }
        [_, Form::Vector(codes), Form::Symbol(_), _, ..]
            if !codes.is_empty()
                && codes
                    .iter()
                    .all(|code| matches!(code, Form::Keyword(name) if name.contains('/'))) =>
        {
            let selectors = codes
                .iter()
                .map(|code| match code {
                    Form::Keyword(name) => format!(":{name}"),
                    _ => unreachable!(),
                })
                .collect::<Vec<_>>()
                .join(",");
            Ok((Some(format!("[{selectors}]")), 2, 3))
        }
        _ => Err("catch selector must be a namespaced keyword, a non-empty vector of namespaced keywords, or omitted".into()),
    }
}
fn finally(
    result: Result<Value, String>,
    v: Vec<Form>,
    env: Rc<RefCell<HashMap<String, Value>>>,
    k: Cont,
) -> Step {
    forms_cps(
        Rc::new(v),
        0,
        Value::Nil,
        env,
        Box::new(move |r| match r {
            Err(x) => k(Err(x)),
            Ok(_) => k(result),
        }),
    )
}

thread_local! {static TEMP:Cell<u64>=const{Cell::new(0)};}
fn temp() -> String {
    TEMP.with(|x| {
        let n = x.get();
        x.set(n + 1);
        format!("__fiber_{n}")
    })
}
fn application(v: Vec<Form>, env: Rc<RefCell<HashMap<String, Value>>>, k: Cont) -> Step {
    if let Some(Form::Symbol(name)) = v.first() {
        if crate::core::resolve_macro(name).is_some() {
            let result = {
                let mut environment = env.borrow_mut();
                eval(&Form::List(v), &mut environment)
            };
            return k(result);
        }
    }

    let head_symbol = match &v[0] {
        Form::Symbol(name) => Some(name.as_str()),
        _ => None,
    };
    if let Some(name) = head_symbol {
        if let Ok(registry) = namespace_registry() {
            let mut environment = env.borrow_mut();
            if let Err(error) =
                ensure_foundation_namespace_for_symbol(&registry, &mut environment, name)
            {
                return k(Err(error));
            }
        }
    }
    let bound = head_symbol.and_then(|name| binding_value(&env.borrow(), name));
    if let Some(Value::Function(function)) = bound {
        let call_form = Form::List(v.clone());
        let call_name = function
            .name
            .clone()
            .unwrap_or_else(|| "<anonymous>".into());
        let call_environment = env.clone();
        return values_cps(
            Rc::new(v[1..].to_vec()),
            0,
            Vec::new(),
            env,
            Box::new(move |arguments| match arguments {
                Ok(arguments) => {
                    coroutine::semantic::record_call(
                        &call_form,
                        call_name,
                        &arguments,
                        &call_environment,
                    );
                    call(function, arguments, k)
                }
                Err(error) => k(Err(error)),
            }),
        );
    }

    if head_symbol.is_some_and(|name| SYNC_SPECIAL_FORMS.contains(&name)) {
        return eval_special_form(v, env, k);
    }

    let forms = Rc::new(v[1..].to_vec());
    let arguments_environment = env.clone();
    let call_form = Form::List(v.clone());
    let function_call_form = call_form.clone();
    let function_call_environment = arguments_environment.clone();
    let value_call_environment = arguments_environment.clone();
    one(
        v[0].clone(),
        env,
        Box::new(move |result| match result {
            Ok(Value::Function(function)) => {
                let call_name = function
                    .name
                    .clone()
                    .unwrap_or_else(|| "<anonymous>".into());
                values_cps(
                    forms,
                    0,
                    Vec::new(),
                    arguments_environment,
                    Box::new(move |arguments| match arguments {
                        Ok(arguments) => {
                            coroutine::semantic::record_call(
                                &function_call_form,
                                call_name,
                                &arguments,
                                &function_call_environment,
                            );
                            call(function, arguments, k)
                        }
                        Err(error) => k(Err(error)),
                    }),
                )
            }
            Ok(value) => {
                let call_name = crate::core::portable_type_name(&value).to_owned();
                values_cps(
                    forms,
                    0,
                    Vec::new(),
                    arguments_environment,
                    Box::new(move |arguments| match arguments {
                        Ok(arguments) => {
                            coroutine::semantic::record_call(
                                &call_form,
                                call_name,
                                &arguments,
                                &value_call_environment,
                            );
                            k(crate::core::call_value(value, arguments))
                        }
                        Err(error) => k(Err(error)),
                    }),
                )
            }
            Err(error) => k(Err(error)),
        }),
    )
}

fn execution_context(
    environment: HashMap<String, Value>,
) -> (NamespaceRegistry<Value>, HashMap<String, Value>) {
    if let Ok(registry) = namespace_registry() {
        return (registry, environment);
    }
    let (registry, mut runtime_environment) = crate::Runtime::new().standalone_eval_context();
    runtime_environment.extend(environment);
    (registry, runtime_environment)
}

fn eval_special_form(v: Vec<Form>, env: Rc<RefCell<HashMap<String, Value>>>, k: Cont) -> Step {
    let call_form = Form::List(v.clone());
    let op = v[0].clone();
    let call_name = match &op {
        Form::Symbol(name) => name.clone(),
        _ => "<callable>".into(),
    };
    let e = env.clone();
    values_cps(
        Rc::new(v[1..].to_vec()),
        0,
        Vec::new(),
        env,
        Box::new(move |r| match r {
            Ok(values) => {
                coroutine::semantic::record_call(&call_form, call_name, &values, &e);
                let mut env = e.borrow_mut();
                let mut old = Vec::new();
                let mut list = vec![op];
                for x in values {
                    let n = temp();
                    let prior = env.insert(n.clone(), x);
                    old.push((n.clone(), prior));
                    list.push(Form::Symbol(n));
                }
                let r = eval(&Form::List(list), &mut env);
                restore(&mut env, old);
                drop(env);
                k(r)
            }
            Err(x) => k(Err(x)),
        }),
    )
}
fn call(f: Rc<Function>, args: Vec<Value>, k: Cont) -> Step {
    if !f.clauses.is_empty() {
        let Some(clause) = select_clause(&f.clauses, args.len()) else {
            let name = f.name.clone().unwrap_or_else(|| "<anonymous>".into());
            return k(Err(format!(
                "{name} has no arity accepting {} arguments",
                args.len()
            )));
        };
        return call(clause, args, k);
    }
    if let Some(fiber_native) = &f.fiber_native {
        return fiber_native(args, k);
    }
    if f.native.is_some() {
        return k(crate::core::call_function(&f, args));
    }
    if f.variadic.is_none() && f.params.len() != args.len() {
        if f.namespace.as_deref() == Some("std.foundation") && f.name.as_deref() == Some("type") {
            return k(Err("type expects one value".into()));
        }
        return k(Err(format!(
            "function expects {} arguments",
            f.params.len()
        )));
    }
    if args.len() < f.params.len() {
        return k(Err(format!(
            "function expects at least {} arguments",
            f.params.len()
        )));
    }
    let tracing = tracing_enabled();
    if tracing {
        TRACE_STACK.with(|stack| {
            stack.borrow_mut().push(trace_frame_label(
                f.name.clone().unwrap_or_else(|| "<anonymous>".into()),
                f.namespace.clone(),
                current_exception_site(),
            ))
        });
    }
    let caller_scoped_foundation = f.namespace.as_deref() == Some("std.foundation")
        && (f.is_macro
            || matches!(
                f.name.as_deref(),
                Some(
                    "macroexpand"
                        | "macroexpand-1"
                        | "ns-current"
                        | "env-snapshot"
                        | "ns-vars"
                        | "ns-list"
                        | "ns-info"
                        | "env-module"
                )
            ));
    let namespace_scope = namespace_registry().ok().and_then(|registry| {
        (!caller_scoped_foundation)
            .then_some(())
            .and_then(|_| f.namespace.as_ref())
            .map(|namespace| {
                let previous = registry.current().name().as_str().to_owned();
                registry.set_current(namespace);
                (registry, previous)
            })
    });
    let mut env = f.captured.borrow().clone();
    for (n, x) in f.params.iter().zip(args.iter()) {
        env.insert(n.clone(), x.clone());
    }
    let mut bound = Vec::new();
    for (pattern, value) in f.patterns.iter().zip(args.iter()) {
        if let Err(error) =
            crate::core::bind_pattern(pattern, value.clone(), &mut env, &mut bound, None)
        {
            if let Some((registry, previous)) = namespace_scope {
                registry.set_current(&previous);
            }
            let error = append_trace(error);
            if tracing {
                TRACE_STACK.with(|stack| {
                    stack.borrow_mut().pop();
                });
            }
            return k(Err(format!("function destructuring failed: {error}")));
        }
    }
    if let Some(n) = &f.variadic {
        let skip = f.params.len();
        let rest = Value::List(args.into_iter().skip(skip).collect());
        env.insert(n.clone(), rest.clone());
        if let Some(pattern) = &f.variadic_pattern {
            if let Err(error) = crate::core::bind_pattern(pattern, rest, &mut env, &mut bound, None)
            {
                if let Some((registry, previous)) = namespace_scope {
                    registry.set_current(&previous);
                }
                let error = append_trace(error);
                if tracing {
                    TRACE_STACK.with(|stack| {
                        stack.borrow_mut().pop();
                    });
                }
                return k(Err(format!("function destructuring failed: {error}")));
            }
        }
    }
    forms_cps(
        Rc::new(f.body.clone()),
        0,
        Value::Nil,
        Rc::new(RefCell::new(env)),
        Box::new(move |r| match r {
            Ok(Value::Recur(_)) => {
                if let Some((registry, previous)) = namespace_scope {
                    registry.set_current(&previous);
                }
                let result = append_trace("recur must be inside loop".into());
                if tracing {
                    TRACE_STACK.with(|stack| {
                        stack.borrow_mut().pop();
                    });
                }
                k(Err(result))
            }
            r => {
                if let Some((registry, previous)) = namespace_scope {
                    registry.set_current(&previous);
                }
                let r = r.map_err(append_trace);
                if tracing {
                    TRACE_STACK.with(|stack| {
                        stack.borrow_mut().pop();
                    });
                }
                k(r)
            }
        }),
    )
}

pub(crate) fn invoke_function_sync(
    function: Rc<Function>,
    arguments: Vec<Value>,
) -> Result<Value, String> {
    let (namespace_registry, environment) = execution_context(HashMap::new());
    let env = Rc::new(RefCell::new(environment));
    let mut fiber = EvalFiber {
        env,
        namespace_registry,
        pending: None,
        resume: None,
        state: EvalFiberState::Running,
    };
    let step = with_namespace_registry(&fiber.namespace_registry, || {
        call(function, arguments, Box::new(Step::Done))
    });
    fiber.accept(step);
    fiber.drive_sync()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn def_returns_the_qualified_var() {
        let registry = crate::kernel::NamespaceRegistry::new("user");
        crate::core::with_namespace_registry(&registry, || {
            let mut fiber = EvalFiber::start("(def player 1)", HashMap::new()).unwrap();
            let Value::Var(var) = fiber.drive_sync().unwrap() else {
                panic!("def must return a Var")
            };
            assert_eq!(var.display(), "#'user/player");
            assert_eq!(var.deref_value(), Value::Number(1));
        });
    }

    #[test]
    fn fallback_def_takes_ownership_from_a_bootstrap_library_var() {
        let registry = crate::kernel::NamespaceRegistry::new("user");
        crate::core::with_namespace_registry(&registry, || {
            let seed = registry.current().intern_with_origin(
                "optimized",
                Value::Number(7),
                crate::kernel::VarOrigin::RustLibrary,
            );
            let mut environment = HashMap::new();
            environment.insert("optimized".into(), Value::Var(seed.clone()));

            let result =
                crate::core::with_definition_origin(crate::kernel::VarOrigin::HalFallback, || {
                    let mut fiber = EvalFiber::start("(def optimized 9)", environment).unwrap();
                    fiber.drive_sync().unwrap()
                });
            let Value::Var(var) = result else {
                panic!("def must return a Var")
            };
            assert!(seed.same_identity(&var));
            assert_eq!(var.origin(), crate::kernel::VarOrigin::HalFallback);
            assert_eq!(var.deref_value(), Value::Number(9));
        });
    }

    #[test]
    fn named_value_constructors_are_visible_to_later_forms_in_the_same_do() {
        let cases = [
            (
                "(do (defstruct Point [x y]) \
                 (+ (get (Point 1 2) :x) \
                    (get (->Point 3 4) :x) \
                    (get (map->Point {:x 5}) :x)))",
                Value::Number(9),
            ),
            (
                "(do (defmutable Cursor [x y]) \
                 (+ (field (Cursor 1 2) :x) \
                    (field (->Cursor 3 4) :x) \
                    (field (map->Cursor {:x 5}) :x)))",
                Value::Number(9),
            ),
        ];
        for (source, expected) in cases {
            let mut fiber = EvalFiber::start(source, HashMap::new()).unwrap();
            assert_eq!(fiber.drive_sync(), Ok(expected));
        }
    }

    #[test]
    fn mutable_field_set_place_updates_and_returns_replacement() {
        let mut fiber = EvalFiber::start(
            "(do (defmutable Cursor [x y]) \
             (def cursor (Cursor 1 2)) \
             (if (= (set! (field cursor :x) 42) 42) \
               (field cursor :x) \
               -1))",
            HashMap::new(),
        )
        .unwrap();
        assert_eq!(fiber.drive_sync(), Ok(Value::Number(42)));
    }

    #[test]
    fn mutable_field_set_place_resumes_after_replacement_suspends() {
        let promise = Promise::new();
        let mut environment = HashMap::new();
        environment.insert("replacement".into(), Value::Promise(promise.clone()));
        let mut fiber = EvalFiber::start(
            "(do (defmutable Cursor [x]) \
             (def cursor (Cursor 1)) \
             (set! (field cursor :x) (deref replacement)) \
             (field cursor :x))",
            environment,
        )
        .unwrap();
        assert_eq!(fiber.state(), EvalFiberState::Suspended);
        promise.resolve(Value::Number(42));
        assert_eq!(
            fiber.resume(promise.state()),
            EvalFiberState::Completed(Value::Number(42))
        );
    }

    #[test]
    fn mutable_field_set_place_resumes_receiver_before_evaluating_replacement() {
        let ready = Promise::new();
        let mut environment = HashMap::new();
        environment.insert("ready".into(), Value::Promise(ready.clone()));
        let mut fiber = EvalFiber::start(
            "(do (def order []) \
             (defmutable Cursor [x]) \
             (def cursor (Cursor 1)) \
             (set! (field (do (deref ready) cursor) :x) \
                   (do (set! order (conj order :replacement)) 42)) \
             [order (field cursor :x)])",
            environment,
        )
        .unwrap();
        assert_eq!(fiber.state(), EvalFiberState::Suspended);
        ready.resolve(Value::Bool(true));
        assert_eq!(
            fiber.resume(ready.state()),
            EvalFiberState::Completed(Value::Vector(
                [
                    Value::Vector([Value::Keyword("replacement".into())].into_iter().collect()),
                    Value::Number(42),
                ]
                .into_iter()
                .collect(),
            ))
        );
    }

    #[test]
    fn anonymous_namespace_form_is_a_session_local_noop() {
        let registry = crate::kernel::NamespaceRegistry::new("user");
        crate::core::with_namespace_registry(&registry, || {
            let mut fiber = EvalFiber::start("(ns+)", HashMap::new()).unwrap();
            assert_eq!(fiber.drive_sync(), Ok(Value::Nil));
            assert_eq!(registry.current().name().as_str(), "user");
        });
    }

    #[test]
    fn resumes_nested() {
        let p = Promise::new();
        let mut e = HashMap::new();
        e.insert("p".into(), Value::Promise(p.clone()));
        let mut f = EvalFiber::start("(let [x 1] (+ x (deref p)))", e).unwrap();
        assert_eq!(f.state(), EvalFiberState::Suspended);
        p.resolve(Value::Number(41));
        assert_eq!(
            f.resume(p.state()),
            EvalFiberState::Completed(Value::Number(42))
        );
    }

    #[test]
    fn drive_sync_waits_for_a_deferred_promise() {
        let mut fiber =
            EvalFiber::start("(deref (promise/delay 1 (fn [] 42)))", HashMap::new()).unwrap();
        assert_eq!(fiber.drive_sync(), Ok(Value::Number(42)));
    }

    #[test]
    fn cancelling_a_suspended_fiber_notifies_its_pending_promise() {
        let promise = Promise::new();
        let cancelled = Rc::new(Cell::new(false));
        let observed = cancelled.clone();
        promise.set_cancel_hook(Rc::new(move || observed.set(true)));
        let mut environment = HashMap::new();
        environment.insert("p".into(), Value::Promise(promise));
        let mut fiber = EvalFiber::start("(deref p)", environment).unwrap();
        assert_eq!(fiber.state(), EvalFiberState::Suspended);
        assert!(fiber.cancel());
        assert!(cancelled.get());
        assert_eq!(fiber.state(), EvalFiberState::Cancelled);
    }
    #[test]
    fn resumes_function_finally() {
        let p = Promise::new();
        let mut e = HashMap::new();
        e.insert("p".into(), Value::Promise(p.clone()));
        let mut f = EvalFiber::start(
            "(do (def f (fn [x] (try (+ x (deref p)) (finally nil)))) (f 2))",
            e,
        )
        .unwrap();
        assert_eq!(f.state(), EvalFiberState::Suspended);
        p.resolve(Value::Number(40));
        assert_eq!(
            f.resume(p.state()),
            EvalFiberState::Completed(Value::Number(42))
        );
    }
    #[test]
    fn resumes_multi_arity_dispatch() {
        let p = Promise::new();
        let mut e = HashMap::new();
        e.insert("p".into(), Value::Promise(p.clone()));
        let mut f = EvalFiber::start(
            "(do (defn g ([x] (+ x 1)) ([x y] (+ x y (deref p)))) (g 1 2))",
            e,
        )
        .unwrap();
        assert_eq!(f.state(), EvalFiberState::Suspended);
        p.resolve(Value::Number(39));
        assert_eq!(
            f.resume(p.state()),
            EvalFiberState::Completed(Value::Number(42))
        );
        let mut f = EvalFiber::start(
            "(do (defn h ([x] (+ x 1)) ([x y] (+ x y))) (h 41))",
            HashMap::new(),
        )
        .unwrap();
        assert_eq!(f.state(), EvalFiberState::Completed(Value::Number(42)));
    }

    #[test]
    fn computed_function_head_can_suspend() {
        let promise = Promise::new();
        let mut environment = HashMap::new();
        environment.insert("p".into(), Value::Promise(promise.clone()));
        let mut fiber = EvalFiber::start(
            "(do (def entry [:task (fn [] (std.foundation.coroutine/await p))]) \
             ((nth entry 1)))",
            environment,
        )
        .unwrap();
        assert_eq!(fiber.state(), EvalFiberState::Suspended);
        promise.resolve(Value::Number(42));
        assert_eq!(
            fiber.resume(promise.state()),
            EvalFiberState::Completed(Value::Number(42))
        );
    }

    #[test]
    fn logical_forms_short_circuit_without_evaluating_later_branches() {
        let cases = [
            ("(cond true 42 :else (count :invalid))", Value::Number(42)),
            ("(and false (count :invalid))", Value::Bool(false)),
            ("(or 42 (count :invalid))", Value::Number(42)),
        ];
        for (source, expected) in cases {
            let fiber = EvalFiber::start(source, HashMap::new()).unwrap();
            assert_eq!(fiber.state(), EvalFiberState::Completed(expected));
        }
    }

    #[test]
    fn numeric_and_boolean_predicates_match_foundation_types() {
        let cases = [
            ("(long? 42)", Value::Bool(true)),
            ("(bigint? 9223372036854775808)", Value::Bool(true)),
            ("(integer? 9223372036854775808)", Value::Bool(true)),
            ("(integer? 1.0)", Value::Bool(false)),
            ("(double? 42.0)", Value::Bool(true)),
            ("(number? 42)", Value::Bool(true)),
            ("(boolean? false)", Value::Bool(true)),
            ("(boolean? nil)", Value::Bool(false)),
        ];
        for (source, expected) in cases {
            let fiber = EvalFiber::start(source, HashMap::new()).unwrap();
            assert_eq!(fiber.state(), EvalFiberState::Completed(expected));
        }
    }

    #[test]
    fn character_predicate_matches_foundation_types() {
        let cases = [
            ("(char? \\x)", Value::Bool(true)),
            ("(char? \"x\")", Value::Bool(false)),
        ];
        for (source, expected) in cases {
            let fiber = EvalFiber::start(source, HashMap::new()).unwrap();
            assert_eq!(fiber.state(), EvalFiberState::Completed(expected));
        }
    }

    #[test]
    fn loop_recur_trampolines_large_iteration_counts() {
        let mut fiber = EvalFiber::start(
            "(loop [i 0] (if (< i 50000) (recur (inc i)) i))",
            HashMap::new(),
        )
        .unwrap();
        assert_eq!(fiber.drive_sync(), Ok(Value::Number(50000)));
    }
}
