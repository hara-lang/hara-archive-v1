fn portable_type_keyword(value: &Value) -> Result<Keyword, String> {
    let builtin = match value {
        Value::Nil => "Nil",
        Value::Number(_) => "Long",
        Value::Float(_) => "Float",
        Value::BigInteger(_) if crate::numeric::is_long_value(value) => "Long",
        Value::BigInteger(_) => "BigInteger",
        Value::Character(_) => "Character",
        Value::Regex(_) => "RegExp",
        Value::Tagged(value) if is_uuid_tagged(value) => "UUID",
        Value::Tagged(value) if is_reduced_value(&Value::Tagged(value.clone())) => "Reduced",
        Value::Tagged(_) => "TaggedLiteral",
        Value::Bool(_) => "Boolean",
        Value::String(_) => "String",
        Value::Keyword(_) => "Keyword",
        Value::Symbol(_) => "Symbol",
        Value::Pointer(_) => "Pointer",
        Value::Function(_) => "Function",
        Value::Bytes(_) => "Bytes",
        Value::ByteBuffer(_) => "ByteBuffer",
        Value::Array(_) => "Array",
        Value::Object(_) => "Object",
        Value::Promise(_) => "Promise",
        Value::Atom(_) => "Atom",
        Value::Recur(_) => "Recur",
        Value::List(_) => "List",
        Value::Cons(_) => "Cons",
        Value::Queue(_) => "Queue",
        Value::Deque(_) => "Deque",
        Value::Tuple(_) => "Vector",
        Value::Vector(_) => "Vector",
        Value::MapEntry(_) => "MapEntry",
        Value::MutableCollection(_) => "MutableCollection",
        Value::Seq(_) => "Seq",
        Value::Map(_) => "HashMap",
        Value::OrderedMap(_) => "OrderedMap",
        Value::SortedMap(_) => "SortedMap",
        Value::Trie(_) => "Trie",
        Value::PriorityMap(_) => "PriorityMap",
        Value::Set(_) => "HashSet",
        Value::OrderedSet(_) => "OrderedSet",
        Value::SortedSet(_) => "SortedSet",
        Value::Iterator(_) => "Iterator",
        Value::Var(_) => "Var",
        Value::Namespace(_) => "Namespace",
        Value::Extension(_) => "Extension",
        Value::StructType(_) => "StructType",
        Value::Struct(value) => return Ok(Keyword::from(value.ty.name.replace('/', "."))),
        Value::MutableType(_) => "MutableType",
        Value::Mutable(value) => return Ok(Keyword::from(value.ty.name.replace('/', "."))),
        Value::Protocol(_) => "Protocol",
        Value::NativeType(_) => "NativeType",
        Value::Schema(_) => "SchemaType",
        Value::Coroutine(_) => "Coroutine",
        Value::Stream(_) => "Stream",
        Value::Result(_) => "Result",
        Value::ExceptionInfo(_) => "Exception",
    };
    Ok(Keyword::from(format!("std.native.{builtin}")))
}

fn native_type_instance(native: &NativeType, value: &Value) -> Result<bool, String> {
    Ok(portable_type_keyword(value)?.as_str() == native.name)
}

pub fn receiver_category(value: &Value) -> &'static str {
    match value {
        Value::Nil => "nil",
        Value::Number(_) | Value::Float(_) | Value::BigInteger(_) => "number",
        Value::Character(_) => "character",
        Value::Regex(_) => "pattern",
        Value::Tagged(_) => "tagged",
        Value::Bool(_) => "boolean",
        Value::String(_) => "string",
        Value::Keyword(_) => "keyword",
        Value::Symbol(_) => "symbol",
        Value::Pointer(_) => "pointer",
        Value::Function(_) => "function",
        Value::Bytes(_) | Value::ByteBuffer(_) => "bytes",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
        Value::Promise(_) => "promise",
        Value::Atom(_) => "atom",
        Value::Recur(_) => "recur",
        Value::List(_) => "list",
        Value::Cons(_) => "cons",
        Value::Queue(_) => "queue",
        Value::Deque(_) => "deque",
        Value::Tuple(_) => "vector",
        Value::Vector(_) => "vector",
        Value::MapEntry(_) => "map-entry",
        Value::MutableCollection(_) => "mutable",
        Value::Seq(_) => "seq",
        Value::Map(_)
        | Value::OrderedMap(_)
        | Value::SortedMap(_)
        | Value::Trie(_)
        | Value::PriorityMap(_) => "map",
        Value::Set(_) | Value::OrderedSet(_) | Value::SortedSet(_) => "set",
        Value::Iterator(_) => "iterator",
        Value::Var(_) => "var",
        Value::Namespace(_) => "namespace",
        Value::Extension(_) => "extension",
        Value::StructType(_) => "struct-type",
        Value::Struct(_) => "struct",
        Value::MutableType(_) => "mutable-type",
        Value::Mutable(_) => "mutable",
        Value::Protocol(_) => "protocol",
        Value::NativeType(_) => "native-type",
        Value::Schema(_) => "schema",
        Value::Coroutine(_) => "coroutine",
        Value::Stream(_) => "stream",
        Value::Result(_) => "result",
        Value::ExceptionInfo(_) => "exception",
    }
}

fn coroutine_status(coroutine: &Coroutine) -> Value {
    let state = coroutine.state.borrow();
    Value::Keyword(Keyword::from(match &*state {
        CoroutineState::New(_) | CoroutineState::Suspended(_) => "suspended",
        CoroutineState::Running => "running",
        CoroutineState::Dead => "dead",
    }))
}

fn coroutine_close(coroutine: &Coroutine) -> Result<(), String> {
    let mut state = coroutine.state.borrow_mut();
    match &*state {
        CoroutineState::Dead => Ok(()),
        CoroutineState::Running => Err("coroutine/close: cannot close a running coroutine".into()),
        _ => {
            *state = CoroutineState::Dead;
            Ok(())
        }
    }
}

fn stream_close(stream: &RuntimeStream) -> Result<(), String> {
    if stream.closed.replace(true) {
        return Ok(());
    }
    match &stream.source {
        RuntimeStreamSource::Coroutine { coroutine, .. } => coroutine_close(coroutine),
        RuntimeStreamSource::Guest { close, .. } => {
            if let Some(close) = close {
                call_function(close, Vec::new())?;
            }
            Ok(())
        }
        RuntimeStreamSource::Host { close, .. } => close(),
    }
}

fn stream_next(stream: &RuntimeStream) -> Value {
    let promise = Promise::new();
    if stream.closed.get() {
        promise.resolve(Value::Nil);
        return Value::Promise(promise);
    }
    if stream.pending.replace(true) {
        promise.reject("stream/pending-pull: only one Stream/next may be pending");
        return Value::Promise(promise);
    }
    match &stream.source {
        RuntimeStreamSource::Coroutine {
            coroutine,
            initial_arguments,
        } => {
            let arguments = initial_arguments.borrow_mut().take().unwrap_or_default();
            let coroutine = coroutine.clone();
            let state = Rc::new((stream.pending.clone(), stream.closed.clone()));
            let step = fiber::coroutine::coroutine_resume(
                coroutine.clone(),
                arguments,
                Box::new(Step::Done),
            );
            drive_stream_step(step, coroutine, state, promise.clone());
        }
        RuntimeStreamSource::Guest { next, .. } => {
            let source = match call_function(next, Vec::new()) {
                Ok(value) => promise_from(value),
                Err(error) => {
                    stream.pending.set(false);
                    promise.reject(error);
                    return Value::Promise(promise);
                }
            };
            let pending = stream.pending.clone();
            let closed = stream.closed.clone();
            let output = promise.clone();
            source.on_settle(Rc::new(move |settled| {
                pending.set(false);
                match settled {
                    PromiseState::Fulfilled(value) => {
                        if matches!(value, Value::Nil) {
                            closed.set(true);
                        }
                        output.resolve(value);
                    }
                    PromiseState::Rejected(error) => {
                        closed.set(true);
                        output.reject_rejection(error);
                    }
                    PromiseState::Pending => {}
                };
            }));
            let source_poll = source.clone();
            promise.set_poller(Rc::new(move || {
                source_poll.state();
            }));
            let source_wait = source.clone();
            promise.set_waiter(Rc::new(move || {
                source_wait.wait_state();
            }));
        }
        RuntimeStreamSource::Host { next, .. } => match next() {
            Ok(source) => {
                let pending = stream.pending.clone();
                let closed = stream.closed.clone();
                let output = promise.clone();
                source.on_settle(Rc::new(move |settled| {
                    pending.set(false);
                    match settled {
                        PromiseState::Fulfilled(value) => {
                            if matches!(value, Value::Nil) {
                                closed.set(true);
                            }
                            output.resolve(value);
                        }
                        PromiseState::Rejected(error) => {
                            closed.set(true);
                            output.reject_rejection(error);
                        }
                        PromiseState::Pending => {}
                    };
                }));
                let source_poll = source.clone();
                promise.set_poller(Rc::new(move || {
                    source_poll.state();
                }));
                let source_wait = source.clone();
                promise.set_waiter(Rc::new(move || {
                    source_wait.wait_state();
                }));
            }
            Err(error) => {
                stream.pending.set(false);
                promise.reject(error);
            }
        },
    }
    Value::Promise(promise)
}

pub(crate) fn host_stream(
    next: Rc<dyn Fn() -> Result<Promise, String>>,
    close: Rc<dyn Fn() -> Result<(), String>>,
) -> Value {
    Value::Stream(Rc::new(RuntimeStream::host(next, close)))
}

/// Pulls one item from a native Stream without exposing its representation to an embedder.
pub fn stream_next_value(value: &Value) -> Result<Promise, String> {
    let Value::Stream(stream) = value else {
        return Err("stream/next expects a Stream".into());
    };
    let Value::Promise(promise) = stream_next(stream) else {
        unreachable!("native Stream/next always returns a Promise")
    };
    Ok(promise)
}

/// Closes a native Stream owned by the current runtime worker.
pub fn stream_close_value(value: &Value) -> Result<(), String> {
    let Value::Stream(stream) = value else {
        return Err("stream/close expects a Stream".into());
    };
    stream_close(stream)
}

pub fn stream_value(value: &Value) -> bool {
    matches!(value, Value::Stream(_))
}

fn drive_stream_step(
    mut step: Step,
    coroutine: Rc<Coroutine>,
    state: Rc<(Rc<Cell<bool>>, Rc<Cell<bool>>)>,
    output: Promise,
) {
    loop {
        match step {
            Step::Done(result) => {
                state.0.set(false);
                match result {
                    Ok(_) if matches!(*coroutine.state.borrow(), CoroutineState::Dead) => {
                        state.1.set(true);
                        output.resolve(Value::Nil);
                    }
                    Ok(Value::Nil) => {
                        state.1.set(true);
                        let _ = coroutine_close(&coroutine);
                        output.reject("stream/nil-item: a stream coroutine may not yield nil");
                    }
                    Ok(value) => {
                        output.resolve(value);
                    }
                    Err(error) => {
                        state.1.set(true);
                        output.reject(error);
                    }
                }
                return;
            }
            Step::Continue(next) => step = next(),
            Step::Wait(promise, resume) => {
                let resume = Rc::new(RefCell::new(Some(resume)));
                let coroutine_next = coroutine.clone();
                let state_next = state.clone();
                let output_next = output.clone();
                promise.on_settle(Rc::new(move |settled| {
                    if let Some(resume) = resume.borrow_mut().take() {
                        drive_stream_step(
                            resume(settled),
                            coroutine_next.clone(),
                            state_next.clone(),
                            output_next.clone(),
                        );
                    }
                }));
                return;
            }
            Step::Yield(_, _) => {
                state.0.set(false);
                state.1.set(true);
                output.reject("stream/internal: yield escaped its coroutine boundary");
                return;
            }
        }
    }
}

fn native_stream_values(operation: &str, values: Vec<Value>) -> Result<Value, String> {
    let method = operation
        .strip_prefix("std.native.Stream/")
        .unwrap_or(operation);
    match method {
        "create" => {
            if !(1..=2).contains(&values.len()) {
                return Err("Stream/create expects next and optional close functions".into());
            }
            let Value::Function(next) = &values[0] else {
                return Err("Stream/create expects a next function".into());
            };
            let close = match values.get(1) {
                None | Some(Value::Nil) => None,
                Some(Value::Function(close)) => Some(close.clone()),
                Some(_) => return Err("Stream/create expects a close function or nil".into()),
            };
            Ok(Value::Stream(Rc::new(RuntimeStream::guest(next.clone(), close))))
        }
        "generate" => {
            if values.is_empty() {
                return Err("Stream/generate expects a function".into());
            }
            let body = values[0].clone();
            if !matches!(body, Value::Function(_)) {
                return Err("Stream/generate expects a function".into());
            }
            let arguments = values[1..].to_vec();
            Ok(Value::Stream(Rc::new(RuntimeStream::new(body, arguments))))
        }
        "next" => {
            if values.len() != 1 {
                return Err("Stream/next expects one stream".into());
            }
            match &values[0] {
                Value::Stream(stream) => Ok(stream_next(stream)),
                _ => Err("Stream/next expects a stream".into()),
            }
        }
        _ => Err(format!("unknown std.native.Stream operation: {method}")),
    }
}

fn parse_forms(source: &str) -> Result<Vec<Form>, String> {
    crate::kernel::parse_forms(source)
}

pub fn read_edn(source: &str) -> Result<Value, String> {
    let forms = parse_forms(source).map_err(|error| format!("edn/read: {error}"))?;
    if forms.len() != 1 {
        return Err("edn/read expects exactly one value".into());
    }
    form_to_value(&forms[0]).map_err(|error| format!("edn/read: {error}"))
}
