fn os_values(operation: &str, values: Vec<Value>) -> Result<Value, String> {
    let operation = operation.strip_prefix("os/").unwrap_or(operation);
    let operation = operation
        .strip_prefix("std.native.OS/")
        .unwrap_or(operation);
    let process_operation = operation.strip_prefix("std.native.Process/");
    let operation = match process_operation.unwrap_or(operation) {
        "alive?" if process_operation.is_some() => "process-alive?",
        "write" if process_operation.is_some() => "process-write",
        "close-input" if process_operation.is_some() => "process-close-input",
        "stdout" if process_operation.is_some() => "process-stdout",
        "stderr" if process_operation.is_some() => "process-stderr",
        "stdout-stream" if process_operation.is_some() => "process-stdout-stream",
        "stderr-stream" if process_operation.is_some() => "process-stderr-stream",
        "wait" if process_operation.is_some() => "process-wait",
        "kill" if process_operation.is_some() => "process-kill",
        value => value,
    };
    match operation {
        "time-ms" => {
            if !values.is_empty() {
                return Err("os/time-ms expects no arguments".into());
            }
            return Ok(Value::Number(crate::clock::time_ms()));
        }
        "time-ns" => {
            if !values.is_empty() {
                return Err("os/time-ns expects no arguments".into());
            }
            return Ok(Value::Number(crate::clock::time_ns()));
        }
        "platform" => {
            if !values.is_empty() {
                return Err("os/platform expects no arguments".into());
            }
            let platform = if cfg!(target_os = "linux") {
                "linux"
            } else if cfg!(target_os = "macos") {
                "macos"
            } else if cfg!(target_os = "windows") {
                "windows"
            } else {
                "unknown"
            };
            return Ok(Value::Keyword(platform.into()));
        }
        "arch" => {
            if !values.is_empty() {
                return Err("os/arch expects no arguments".into());
            }
            let arch = match std::env::consts::ARCH {
                "x86_64" => "x86-64",
                value => value,
            };
            return Ok(Value::Keyword(arch.into()));
        }
        "cwd" => {
            if !values.is_empty() {
                return Err("os/cwd expects no arguments".into());
            }
            return std::env::current_dir()
                .map(|path| Value::String(path.to_string_lossy().into_owned()))
                .map_err(|error| format!("os/cwd failed: {error}"));
        }
        "env" => {
            if !values.is_empty() {
                return Err("os/env expects no arguments".into());
            }
            return Ok(Value::Map(PMap::from_iter(
                std::env::vars().map(|(key, value)| (Value::String(key), Value::String(value))),
            )));
        }
        "getenv" => {
            if values.len() != 1 {
                return Err("os/getenv expects a name".into());
            }
            let Value::String(name) = &values[0] else {
                return Err("os/getenv expects a string".into());
            };
            return Ok(std::env::var(name).map(Value::String).unwrap_or(Value::Nil));
        }
        "process?" => {
            if values.len() != 1 {
                return Err("os/process? expects one argument".into());
            }
            let value = values[0].clone();
            #[cfg(not(target_arch = "wasm32"))]
            return Ok(Value::Bool(crate::native_process::is_process(&value)));
            #[cfg(target_arch = "wasm32")]
            return Ok(Value::Bool(false));
        }
        _ => {}
    }
    require_process_access(&format!("os/{operation}"))?;
    #[cfg(target_arch = "wasm32")]
    return Err(format!("os/{operation} is unsupported on wasm"));
    #[cfg(not(target_arch = "wasm32"))]
    match operation {
        "spawn" => {
            if !(1..=2).contains(&values.len()) {
                return Err("os/spawn expects argv and optional options".into());
            }
            let argv = iterator_values(values[0].clone())?
                .into_iter()
                .map(|value| match value {
                    Value::String(value) => Ok(value),
                    _ => Err("os/spawn argv must contain strings".to_owned()),
                })
                .collect::<Result<Vec<_>, _>>()?;
            let mut cwd = None;
            let mut environment = Vec::new();
            if values.len() == 2 {
                let options = values[1].clone();
                for (key, value) in map_entries(&options)
                    .ok_or_else(|| "os/spawn options must be a map".to_owned())?
                {
                    match (key, value) {
                        (Value::Keyword(key), Value::String(value)) if key.as_str() == "cwd" => {
                            cwd = Some(value);
                        }
                        (Value::Keyword(key), value) if key.as_str() == "env" => {
                            for (name, value) in map_entries(&value)
                                .ok_or_else(|| "os/spawn :env must be a map".to_owned())?
                            {
                                let (Value::String(name), Value::String(value)) = (name, value)
                                else {
                                    return Err("os/spawn :env must contain string pairs".into());
                                };
                                environment.push((name, value));
                            }
                        }
                        _ => {}
                    }
                }
            }
            crate::native_process::spawn(&argv, cwd.as_deref(), &environment)
        }
        method @ ("process-alive?"
        | "process-close-input"
        | "process-stdout"
        | "process-stderr"
        | "process-wait"
        | "process-kill") => {
            if values.len() != 1 {
                return Err(format!("os/{method} expects a process"));
            }
            let process = values[0].clone();
            match method {
                "process-alive?" => crate::native_process::alive(&process).map(Value::Bool),
                "process-close-input" => {
                    crate::native_process::close_input(&process).map(|()| Value::Nil)
                }
                "process-stdout" => {
                    crate::native_process::promise(&process, "stdout").map(Value::Promise)
                }
                "process-stderr" => {
                    crate::native_process::promise(&process, "stderr").map(Value::Promise)
                }
                "process-wait" => {
                    crate::native_process::promise(&process, "wait").map(Value::Promise)
                }
                "process-kill" => crate::native_process::kill(&process).map(|()| process),
                _ => unreachable!(),
            }
        }
        method @ ("process-stdout-stream" | "process-stderr-stream") => {
            if values.len() != 1 {
                return Err(format!("os/{method} expects a process"));
            }
            let process = values[0].clone();
            let kind = if method == "process-stderr-stream" {
                "stderr"
            } else {
                "stdout"
            };
            let handle = crate::native_process::take_stream(&process, kind)?;
            Ok(host_stream(
                Rc::new(move || Ok(crate::native_process::stream_promise(handle, kind))),
                Rc::new(|| Ok(())),
            ))
        }
        "process-write" => {
            if values.len() != 2 {
                return Err("os/process-write expects a process and bytes".into());
            }
            let process = values[0].clone();
            let bytes = match &values[1] {
                Value::Bytes(value) => value.clone(),
                Value::ByteBuffer(value) => value.borrow().clone(),
                _ => return Err("os/process-write expects bytes".into()),
            };
            crate::native_process::write(&process, &bytes).map(|count| Value::Number(count as i64))
        }
        _ => Err(format!("unknown os operation: {operation}")),
    }
}

fn native_test_events() -> Value {
    Value::Vector(PVector::from_iter([
        Value::Keyword("test/run-started".into()),
        Value::Keyword("test/fact-started".into()),
        Value::Keyword("test/fact-completed".into()),
        Value::Keyword("test/run-completed".into()),
    ]))
}

fn native_test_runner(value: Value) -> Result<Value, String> {
    match value {
        Value::Keyword(runner) if matches!(runner.as_str(), "code.test" | "native") => {
            Ok(Value::Keyword(runner))
        }
        _ => Err("runtime test runner must be :code.test or :native".into()),
    }
}

fn native_test_active_runner() -> Result<Value, String> {
    ACTIVE_TEST_RUNNER
        .with(|runner| native_test_runner(Value::Keyword(runner.borrow().clone().into())))
}

fn native_test_config(runner: Value, options: Value) -> Result<Value, String> {
    if map_entries(&options).is_none() {
        return Err("std.native.Test/config options must be a map".into());
    }
    if map_value(&options, &Value::Keyword("runner".into())).is_some() {
        return Err("std.native.Test/config runner is owned by the runtime".into());
    }
    Ok(Value::Map(PMap::from_iter([
        (Value::Keyword("runner".into()), runner),
        (Value::Keyword("options".into()), options),
    ])))
}

fn native_test_context(name: Value, actual: Value, expected: Value, failures: Value) -> Value {
    Value::Map(PMap::from_iter([
        (
            Value::Keyword("test".into()),
            Value::Map(PMap::from_iter([
                (Value::Keyword("name".into()), name),
                (Value::Keyword("actual".into()), actual),
                (Value::Keyword("expected".into()), expected),
            ])),
        ),
        (Value::Keyword("failures".into()), failures),
    ]))
}

fn native_test_failure(actual: Value, expected: Value) -> Value {
    Value::Map(PMap::from_iter([
        (
            Value::Keyword("failure/code".into()),
            Value::Keyword("test/not-equal".into()),
        ),
        (
            Value::Keyword("failure/path".into()),
            Value::Vector(PVector::new()),
        ),
        (
            Value::Keyword("failure/in".into()),
            Value::Vector(PVector::new()),
        ),
        (Value::Keyword("failure/actual".into()), actual),
        (Value::Keyword("failure/expected".into()), expected),
        (
            Value::Keyword("failure/message".into()),
            Value::String("values are not equal".into()),
        ),
        (
            Value::Keyword("failure/context".into()),
            Value::Map(PMap::new()),
        ),
        (
            Value::Keyword("failure/children".into()),
            Value::Vector(PVector::new()),
        ),
    ]))
}

fn native_test_compare(actual: Value, expected: Value) -> Result<Value, String> {
    let pass = actual == expected;
    let failures = if pass {
        Value::Vector(PVector::new())
    } else {
        Value::Vector(PVector::from_iter([native_test_failure(
            actual.clone(),
            expected.clone(),
        )]))
    };
    Ok(Value::Result(Rc::new(ResultValue::success(
        Value::Bool(pass),
        native_test_context(Value::Nil, actual, expected, failures),
    )?)))
}

fn native_test_result(
    name: Value,
    actual: Value,
    expected: Value,
    comparison: Value,
) -> Result<Value, String> {
    let Value::Result(comparison) = comparison else {
        return Err("std.native.Test/result expects a comparison Result".into());
    };
    let failures = map_value(&comparison.context, &Value::Keyword("failures".into()))
        .cloned()
        .unwrap_or_else(|| Value::Vector(PVector::new()));
    Ok(Value::Result(Rc::new(comparison.with_context(
        native_test_context(name, actual, expected, failures),
    )?)))
}

fn native_test_error(name: Value, actual: Value, expected: Value, error: String) -> Value {
    Value::Result(Rc::new(
        ResultValue::error(
            caught_error(&error),
            native_test_context(name, actual, expected, Value::Vector(PVector::new())),
        )
        .expect("native Test error context is a map"),
    ))
}

fn native_test_checked_result(name: Value, metadata: Option<Value>, checked: Value) -> Value {
    let Value::Result(result) = checked else {
        return native_test_error(
            name,
            Value::Nil,
            Value::Nil,
            "Test/run check function must return a Result".into(),
        );
    };
    let mut context =
        PMap::from_iter(map_entries(&result.context).expect("Result context is a map"));
    let test = map_value(&result.context, &Value::Keyword("test".into()))
        .cloned()
        .unwrap_or_else(|| Value::Map(PMap::new()));
    let mut test = PMap::from_iter(map_entries(&test).unwrap_or_default());
    test = test.assoc_value(Value::Keyword("name".into()), name);
    context = context.assoc_value(Value::Keyword("test".into()), Value::Map(test));
    if let Some(metadata) = metadata {
        context = context.assoc_value(Value::Keyword("meta".into()), metadata);
    }
    Value::Result(Rc::new(
        result
            .with_context(Value::Map(context))
            .expect("native Test checked context is a map"),
    ))
}

fn native_test_lifecycle_error(phase: &str, error: String) -> Value {
    let name = Value::String(format!("test {phase}"));
    let phase = Value::Keyword(phase.into());
    let Value::Result(result) = native_test_error(name, Value::Nil, Value::Nil, error) else {
        unreachable!()
    };
    Value::Result(Rc::new(
        result
            .with_context(Value::Map(PMap::from_iter([(
                Value::Keyword("phase".into()),
                phase,
            )])))
            .expect("native Test lifecycle context is a map"),
    ))
}

fn native_test_lifecycle(lifecycle: &Value, phase: &str) -> Result<(), String> {
    let Some(function) = map_value(lifecycle, &Value::Keyword(phase.into())).cloned() else {
        return Ok(());
    };
    call_value(function, Vec::new())
        .and_then(native_test_await)
        .map(|_| ())
}

fn native_test_run(
    cases: Value,
    check_function: Option<Value>,
    lifecycle: Option<Value>,
) -> Result<Value, String> {
    let cases = match cases {
        Value::Vector(cases) => cases.iter().cloned().collect::<Vec<_>>(),
        Value::Tuple(cases) => cases.iter().cloned().collect::<Vec<_>>(),
        _ => return Err("std.native.Test/run expects a vector of test cases".into()),
    };
    let state = namespace_registry()?.find_or_create("std.native.Test.state");
    let results_symbol = crate::lang::data::Symbol::parse("results");
    let mut results = match state.resolve(&results_symbol).map(|var| var.deref_value()) {
        Some(Value::Vector(results)) => results.iter().cloned().collect::<Vec<_>>(),
        _ => Vec::new(),
    };
    let setup_ok = match &lifecycle {
        Some(lifecycle) => match native_test_lifecycle(lifecycle, "setup") {
            Ok(()) => true,
            Err(error) => {
                results.push(native_test_lifecycle_error("setup", error));
                false
            }
        },
        None => true,
    };
    if setup_ok {
        for (index, case) in cases.iter().enumerate() {
            let fallback_name = Value::String(format!("invalid case {}", index + 1));
            let Some(entries) = map_entries(case) else {
                results.push(native_test_error(
                    fallback_name,
                    Value::Nil,
                    Value::Nil,
                    "Test/run case must be a map".into(),
                ));
                continue;
            };
            let _ = entries;
            let name = map_value(case, &Value::Keyword("name".into()))
                .cloned()
                .unwrap_or(fallback_name);
            let expected = map_value(case, &Value::Keyword("expected".into())).cloned();
            let test = map_value(case, &Value::Keyword("test".into())).cloned();
            let metadata = map_value(case, &Value::Keyword("meta".into())).cloned();
            let result = match (test, expected) {
                (Some(test), Some(expected)) => match &check_function {
                    Some(check) => match call_value(check.clone(), vec![test, expected])
                        .and_then(native_test_await)
                    {
                        Ok(checked) => native_test_checked_result(name, metadata, checked),
                        Err(error) => {
                            let failed =
                                native_test_error(name.clone(), Value::Nil, Value::Nil, error);
                            native_test_checked_result(name, metadata, failed)
                        }
                    },
                    None => match call_value(test, Vec::new()).and_then(native_test_await) {
                        Ok(actual) => {
                            let comparison = native_test_compare(actual.clone(), expected.clone())?;
                            native_test_result(name, actual, expected, comparison)?
                        }
                        Err(error) => native_test_error(name, Value::Nil, expected, error),
                    },
                },
                (None, expected) => native_test_error(
                    name,
                    Value::Nil,
                    expected.unwrap_or(Value::Nil),
                    "Test/run case requires :test".into(),
                ),
                (Some(_), None) => native_test_error(
                    name,
                    Value::Nil,
                    Value::Nil,
                    "Test/run case requires :expected".into(),
                ),
            };
            results.push(result);
        }
    }
    if let Some(lifecycle) = &lifecycle {
        if let Err(error) = native_test_lifecycle(lifecycle, "teardown") {
            results.push(native_test_lifecycle_error("teardown", error));
        }
    }
    let output = Value::Vector(PVector::from_iter(results));
    state.intern("results", output.clone());
    Ok(output)
}

fn native_test_await(value: Value) -> Result<Value, String> {
    match value {
        Value::Promise(promise) => match promise.wait_state() {
            PromiseState::Fulfilled(value) => Ok(value),
            PromiseState::Rejected(error) => Err(promise_rejection_error(error)),
            PromiseState::Pending => Err("asynchronous test did not settle".into()),
        },
        value => Ok(value),
    }
}

fn native_test_require_result(value: Value, operation: &str) -> Result<Rc<ResultValue>, String> {
    match value {
        Value::Result(result) => Ok(result),
        _ => Err(format!("std.native.Test/{operation} expects a Result")),
    }
}

fn native_test_context_value(result: &ResultValue, key: &str) -> Value {
    native_test_map_value(&result.context, key).unwrap_or(Value::Nil)
}

fn native_test_map_value(value: &Value, key: &str) -> Option<Value> {
    map_entries(value)?
        .into_iter()
        .find_map(|(candidate, value)| {
            matches!(candidate, Value::Keyword(keyword) if keyword.as_str() == key).then_some(value)
        })
}

fn native_test_detail(result: &ResultValue, key: &str) -> Value {
    let test = native_test_context_value(result, "test");
    native_test_map_value(&test, key).unwrap_or(Value::Nil)
}

fn native_test_failure_shape(value: &Value) -> bool {
    let Some(_) = map_entries(value) else {
        return false;
    };
    let keyword = |key: &str| matches!(native_test_map_value(value, key), Some(Value::Keyword(_)));
    let vector = |key: &str| {
        matches!(
            native_test_map_value(value, key),
            Some(Value::Vector(_) | Value::Tuple(_))
        )
    };
    let string = |key: &str| matches!(native_test_map_value(value, key), Some(Value::String(_)));
    let map = |key: &str| {
        native_test_map_value(value, key).is_some_and(|value| map_entries(&value).is_some())
    };
    let children_valid = match native_test_map_value(value, "failure/children") {
        Some(Value::Vector(children)) => children.iter().all(native_test_failure_shape),
        Some(Value::Tuple(children)) => children.iter().all(native_test_failure_shape),
        _ => false,
    };
    keyword("failure/code")
        && vector("failure/path")
        && vector("failure/in")
        && native_test_map_value(value, "failure/actual").is_some()
        && native_test_map_value(value, "failure/expected").is_some()
        && string("failure/message")
        && map("failure/context")
        && vector("failure/children")
        && children_valid
}

fn native_test_failure_leaves(value: &Value, leaves: &mut Vec<Value>) {
    if !native_test_failure_shape(value) {
        return;
    }
    match native_test_map_value(value, "failure/children") {
        Some(Value::Vector(children)) => {
            if children.is_empty() {
                leaves.push(value.clone());
            } else {
                for child in children.iter() {
                    native_test_failure_leaves(child, leaves);
                }
            }
        }
        Some(Value::Tuple(children)) => {
            if children.is_empty() {
                leaves.push(value.clone());
            } else {
                for child in children.iter() {
                    native_test_failure_leaves(child, leaves);
                }
            }
        }
        _ => {}
    }
}

fn native_test_failures(result: &ResultValue) -> Value {
    match native_test_context_value(result, "failures") {
        Value::Vector(failures) => Value::Vector(failures),
        Value::Tuple(failures) => Value::Vector(PVector::from_iter(failures.iter().cloned())),
        _ => Value::Vector(PVector::new()),
    }
}

fn native_test_failure_seq(result: &ResultValue) -> Value {
    let mut leaves = Vec::new();
    if let Value::Vector(failures) = native_test_failures(result) {
        for failure in failures.iter() {
            native_test_failure_leaves(failure, &mut leaves);
        }
    }
    Value::Vector(PVector::from_iter(leaves))
}

fn native_test_values(operation: &str, values: Vec<Value>) -> Result<Value, String> {
    let operation = operation
        .strip_prefix("std.native.Test/")
        .unwrap_or(operation);
    match operation {
        "events" => {
            if !values.is_empty() {
                return Err("std.native.Test/events expects no arguments".into());
            }
            Ok(native_test_events())
        }
        "catalog" => {
            if !values.is_empty() {
                return Err("std.native.Test/catalog expects no arguments".into());
            }
            Ok(Value::Map(PMap::from_iter([
                (
                    Value::Keyword("runners".into()),
                    Value::Vector(PVector::from_iter([
                        Value::Keyword("code.test".into()),
                        Value::Keyword("native".into()),
                    ])),
                ),
                (
                    Value::Keyword("default".into()),
                    Value::Keyword("code.test".into()),
                ),
                (
                    Value::Keyword("runner".into()),
                    native_test_active_runner()?,
                ),
                (
                    Value::Keyword("context".into()),
                    Value::Keyword("test".into()),
                ),
                (Value::Keyword("events".into()), native_test_events()),
            ])))
        }
        "config" => {
            if values.len() > 1 {
                return Err("std.native.Test/config expects optional options".into());
            }
            let options = if values.is_empty() {
                Value::Map(PMap::new())
            } else {
                values[0].clone()
            };
            native_test_config(native_test_active_runner()?, options)
        }
        "context" => {
            if values.len() > 1 {
                return Err("std.native.Test/context expects an optional config".into());
            }
            let config = if values.is_empty() {
                native_test_config(native_test_active_runner()?, Value::Map(PMap::new()))?
            } else {
                let value = values[0].clone();
                let Some(runner) = map_value(&value, &Value::Keyword("runner".into())).cloned()
                else {
                    return Err("std.native.Test/context expects a Test/config map".into());
                };
                let runner = native_test_runner(runner)?;
                if runner != native_test_active_runner()? {
                    return Err(
                        "std.native.Test/context config runner does not match the runtime".into(),
                    );
                }
                value
            };
            Ok(Value::Pointer(PPointer::new(
                "test".into(),
                PMap::from_iter([
                    (Value::Keyword("id".into()), Value::Keyword("test".into())),
                    (Value::Keyword("config".into()), config),
                ]),
            )))
        }
        "compare" => {
            if values.len() != 2 {
                return Err("std.native.Test/compare expects actual and expected".into());
            }
            native_test_compare(values[0].clone(), values[1].clone())
        }
        "result" => {
            if values.len() != 4 {
                return Err(
                    "std.native.Test/result expects name, actual, expected, and comparison Result"
                        .into(),
                );
            }
            let name = values[0].clone();
            let actual = values[1].clone();
            let expected = values[2].clone();
            let comparison = values[3].clone();
            native_test_result(name, actual, expected, comparison)
        }
        "run" => {
            if values.is_empty() || values.len() > 3 {
                return Err(
                    "std.native.Test/run expects cases, an optional check function, and an optional lifecycle map".into(),
                );
            }
            let cases = values[0].clone();
            let second = if values.len() >= 2 {
                Some(values[1].clone())
            } else {
                None
            };
            let third = if values.len() == 3 {
                Some(values[2].clone())
            } else {
                None
            };
            let (check_function, lifecycle) = match (second, third) {
                (Some(check), Some(lifecycle)) => (Some(check), Some(lifecycle)),
                (Some(value), None) if map_entries(&value).is_some() => (None, Some(value)),
                (check, None) => (check, None),
                (None, Some(_)) => unreachable!(),
            };
            if lifecycle
                .as_ref()
                .is_some_and(|value| map_entries(value).is_none())
            {
                return Err("std.native.Test/run lifecycle must be a map".into());
            }
            native_test_run(cases, check_function, lifecycle)
        }
        "passed?" => {
            if values.len() != 1 {
                return Err("std.native.Test/passed? expects one result".into());
            }
            let result = native_test_require_result(values[0].clone(), "passed?")?;
            Ok(Value::Bool(
                result.is_success() && matches!(result.data, Value::Bool(true)),
            ))
        }
        "actual" | "expected" | "failures" | "failure-seq" | "failure-count" => {
            if values.len() != 1 {
                return Err(format!("std.native.Test/{operation} expects one Result"));
            }
            let result = native_test_require_result(values[0].clone(), operation)?;
            Ok(match operation {
                "actual" => native_test_detail(&result, "actual"),
                "expected" => native_test_detail(&result, "expected"),
                "failures" => native_test_failures(&result),
                "failure-seq" => native_test_failure_seq(&result),
                "failure-count" => match native_test_failure_seq(&result) {
                    Value::Vector(values) => Value::Number(values.len() as i64),
                    _ => unreachable!(),
                },
                _ => unreachable!(),
            })
        }
        "failure" => {
            if values.len() != 2 {
                return Err("std.native.Test/failure expects a Result and index".into());
            }
            let result = native_test_require_result(values[0].clone(), "failure")?;
            let index = match &values[1] {
                Value::Number(index) if *index >= 0 => *index as usize,
                _ => {
                    return Err(
                        "std.native.Test/failure index must be a non-negative integer".into(),
                    )
                }
            };
            match native_test_failure_seq(&result) {
                Value::Vector(values) => Ok(values.get(index).cloned().unwrap_or(Value::Nil)),
                _ => unreachable!(),
            }
        }
        "failure?" => {
            if values.len() != 1 {
                return Err("std.native.Test/failure? expects one value".into());
            }
            Ok(Value::Bool(native_test_failure_shape(&values[0])))
        }
        _ => Err(format!("unknown std.native.Test operation: {operation}")),
    }
}
fn native_regex_values(operation: &str, values: Vec<Value>) -> Result<Value, String> {
    let operation = operation
        .strip_prefix("std.native.RegExp/")
        .unwrap_or(operation);
    match operation {
        "compile" => {
            if values.len() != 1 {
                return Err("std.native.RegExp/compile expects one string".into());
            }
            let pattern = match &values[0] {
                Value::String(pattern) => pattern.clone(),
                _ => return Err("std.native.RegExp/compile expects one string".into()),
            };
            regex::Regex::new(&pattern).map_err(|error| format!("invalid regexp: {error}"))?;
            Ok(Value::Regex(pattern))
        }
        "pattern" => {
            if values.len() != 1 {
                return Err("std.native.RegExp/pattern expects one regexp".into());
            }
            match &values[0] {
                Value::Regex(pattern) => Ok(Value::String(pattern.clone())),
                _ => Err("std.native.RegExp/pattern expects one regexp".into()),
            }
        }
        "find?" => {
            if values.len() != 2 {
                return Err("std.native.RegExp/find? expects a regexp and string".into());
            }
            let pattern = match &values[0] {
                Value::Regex(pattern) => pattern.clone(),
                _ => return Err("std.native.RegExp/find? expects a regexp and string".into()),
            };
            let input = match &values[1] {
                Value::String(input) => input.clone(),
                _ => return Err("std.native.RegExp/find? expects a regexp and string".into()),
            };
            let regexp =
                regex::Regex::new(&pattern).map_err(|error| format!("invalid regexp: {error}"))?;
            Ok(Value::Bool(regexp.is_match(&input)))
        }
        "find" => {
            if values.len() != 2 {
                return Err("std.native.RegExp/find expects a regexp and string".into());
            }
            let pattern = match &values[0] {
                Value::Regex(pattern) => pattern.clone(),
                _ => return Err("std.native.RegExp/find expects a regexp and string".into()),
            };
            let input = match &values[1] {
                Value::String(input) => input.clone(),
                _ => return Err("std.native.RegExp/find expects a regexp and string".into()),
            };
            let regexp =
                regex::Regex::new(&pattern).map_err(|error| format!("invalid regexp: {error}"))?;
            Ok(regexp
                .find(&input)
                .map(|matched| Value::String(matched.as_str().to_owned()))
                .unwrap_or(Value::Nil))
        }
        "matches" => {
            if values.len() != 2 {
                return Err("std.native.RegExp/matches expects a regexp and string".into());
            }
            let pattern = match &values[0] {
                Value::Regex(pattern) => pattern.clone(),
                _ => return Err("std.native.RegExp/matches expects a regexp and string".into()),
            };
            let input = match &values[1] {
                Value::String(input) => input.clone(),
                _ => return Err("std.native.RegExp/matches expects a regexp and string".into()),
            };
            let anchored = format!(r"\A(?:{pattern})\z");
            let regexp =
                regex::Regex::new(&anchored).map_err(|error| format!("invalid regexp: {error}"))?;
            Ok(Value::Bool(regexp.is_match(&input)))
        }
        "replace" => {
            if values.len() != 3 {
                return Err(
                    "std.native.RegExp/replace expects a regexp, string, and replacement".into(),
                );
            }
            let pattern = match &values[0] {
                Value::Regex(pattern) => pattern.clone(),
                _ => {
                    return Err(
                        "std.native.RegExp/replace expects a regexp, string, and replacement"
                            .into(),
                    )
                }
            };
            let input = match &values[1] {
                Value::String(input) => input.clone(),
                _ => {
                    return Err(
                        "std.native.RegExp/replace expects a regexp, string, and replacement"
                            .into(),
                    )
                }
            };
            let replacement = match &values[2] {
                Value::String(replacement) => replacement.clone(),
                _ => {
                    return Err(
                        "std.native.RegExp/replace expects a regexp, string, and replacement"
                            .into(),
                    )
                }
            };
            let regexp =
                regex::Regex::new(&pattern).map_err(|error| format!("invalid regexp: {error}"))?;
            Ok(Value::String(
                regexp
                    .replace_all(&input, replacement.as_str())
                    .into_owned(),
            ))
        }
        "split" => {
            if values.len() != 2 {
                return Err("std.native.RegExp/split expects a regexp and string".into());
            }
            let pattern = match &values[0] {
                Value::Regex(pattern) => pattern.clone(),
                _ => return Err("std.native.RegExp/split expects a regexp and string".into()),
            };
            let input = match &values[1] {
                Value::String(input) => input.clone(),
                _ => return Err("std.native.RegExp/split expects a regexp and string".into()),
            };
            if input.is_empty() {
                return Ok(Value::Nil);
            }
            if pattern.is_empty() {
                return Ok(Value::Vector(PVector::from_iter(
                    input
                        .chars()
                        .map(|character| Value::String(character.to_string())),
                )));
            }
            let regexp =
                regex::Regex::new(&pattern).map_err(|error| format!("invalid regexp: {error}"))?;
            Ok(Value::Vector(PVector::from_iter(
                regexp
                    .split(&input)
                    .map(|part| Value::String(part.to_owned())),
            )))
        }
        _ => Err(format!("unknown std.native.RegExp operation: {operation}")),
    }
}

fn file_error(operation: &str, error: FileError) -> String {
    let method = operation
        .strip_prefix("std.native.File/")
        .unwrap_or(operation);
    format!("file/{method} failed: file/{}", error.code())
}

fn socket_error(operation: &str, error: SocketError) -> String {
    format!("{operation} failed: socket/{}", error.code())
}

fn active_file_provider() -> Option<Rc<dyn FileProvider>> {
    ACTIVE_FILE_PROVIDER.with(|active| active.borrow().clone())
}

fn rejected_file_effect(
    operation: &str,
    path: &str,
    target: Option<&str>,
    error: FileError,
) -> Value {
    let promise = Promise::new();
    promise.reject_value(crate::file::file_error_value(
        operation, path, target, &error,
    ));
    Value::Promise(promise)
}

fn file_effect(
    operation: &str,
    path: &str,
    target: Option<&str>,
    invoke: impl FnOnce(&dyn FileProvider) -> Result<Promise, FileError>,
) -> Value {
    let Some(provider) = active_file_provider() else {
        return rejected_file_effect(operation, path, target, FileError::Denied);
    };
    match invoke(provider.as_ref()) {
        Ok(promise) => Value::Promise(promise),
        Err(error) => rejected_file_effect(operation, path, target, error),
    }
}

fn file_option(options: &Value, name: &str) -> Option<Value> {
    let key = Value::Keyword(name.into());
    map_entries(options)?
        .into_iter()
        .find_map(|(candidate, value)| (candidate == key).then_some(value))
}

fn file_options_value(value: Value, operation: &str) -> Result<Value, String> {
    match value {
        Value::Nil => Ok(Value::Map(PMap::new())),
        value if map_entries(&value).is_some() => Ok(value),
        _ => Err(format!("{operation} options must be a map")),
    }
}

fn file_bool_option(
    options: &Value,
    name: &str,
    default: bool,
    operation: &str,
) -> Result<bool, String> {
    match file_option(options, name) {
        None => Ok(default),
        Some(Value::Bool(value)) => Ok(value),
        Some(_) => Err(format!("{operation} :{name} must be boolean")),
    }
}

fn file_string_option(
    options: &Value,
    name: &str,
    default: &str,
    operation: &str,
) -> Result<String, String> {
    match file_option(options, name) {
        None => Ok(default.into()),
        Some(Value::String(value)) => Ok(value),
        Some(_) => Err(format!("{operation} :{name} must be a string")),
    }
}

fn file_write_options(options: &Value) -> Result<WriteOptions, String> {
    let mode = match file_option(options, "mode") {
        None => WriteMode::Create,
        Some(Value::Keyword(value)) if value.as_str() == "create" => WriteMode::Create,
        Some(Value::Keyword(value)) if value.as_str() == "replace" => WriteMode::Replace,
        Some(Value::Keyword(value)) if value.as_str() == "append" => WriteMode::Append,
        Some(_) => {
            return Err("std.native.File/write :mode must be :create, :replace, or :append".into())
        }
    };
    Ok(WriteOptions {
        mode,
        parents: file_bool_option(options, "parents?", false, "std.native.File/write")?,
    })
}

fn file_mkdir_options(options: &Value) -> Result<MkdirOptions, String> {
    Ok(MkdirOptions {
        parents: file_bool_option(options, "parents?", true, "std.native.File/mkdir")?,
        exists_ok: file_bool_option(options, "exists-ok?", true, "std.native.File/mkdir")?,
    })
}

fn file_delete_options(options: &Value) -> Result<DeleteOptions, String> {
    Ok(DeleteOptions {
        missing_ok: file_bool_option(options, "missing-ok?", false, "std.native.File/delete")?,
    })
}

fn file_copy_options(options: &Value) -> Result<CopyOptions, String> {
    Ok(CopyOptions {
        replace: file_bool_option(options, "replace?", false, "std.native.File/copy")?,
        parents: file_bool_option(options, "parents?", false, "std.native.File/copy")?,
        preserve_modified: file_bool_option(
            options,
            "preserve-modified?",
            false,
            "std.native.File/copy",
        )?,
    })
}

fn file_move_options(options: &Value) -> Result<MoveOptions, String> {
    Ok(MoveOptions {
        replace: file_bool_option(options, "replace?", false, "std.native.File/move")?,
        parents: file_bool_option(options, "parents?", false, "std.native.File/move")?,
        atomic: file_bool_option(options, "atomic?", false, "std.native.File/move")?,
    })
}

fn file_values(operation: &str, values: Vec<Value>) -> Result<Value, String> {
    let effect_operation = operation
        .strip_prefix("std.native.File/")
        .map(|method| format!("file/{method}"))
        .unwrap_or_else(|| operation.to_owned());
    match operation {
        "std.native.File/join" | "std.native.File/resolve" => {
            if values.len() != 2 {
                return Err(format!("{operation} expects a base and path"));
            }
            let Value::String(base) = &values[0] else {
                return Err(format!("{operation} expects a base and path"));
            };
            let Value::String(path) = &values[1] else {
                return Err(format!("{operation} expects a base and path"));
            };
            let result = if operation == "std.native.File/join" {
                crate::file::logical_join(&base, &path)
            } else {
                crate::file::logical_resolve(&base, &path)
            };
            result
                .map(Value::String)
                .map_err(|error| file_error(operation, error))
        }
        "std.native.File/parent" => {
            if values.len() != 1 {
                return Err("std.native.File/parent expects a path".into());
            }
            let Value::String(path) = &values[0] else {
                return Err("std.native.File/parent expects a path".into());
            };
            crate::file::logical_parent(path)
                .map(|parent| parent.map(Value::String).unwrap_or(Value::Nil))
                .map_err(|error| file_error(operation, error))
        }
        "std.native.File/read"
        | "std.native.File/exists?"
        | "std.native.File/stat"
        | "std.native.File/entries"
        | "std.native.File/list"
        | "std.native.File/walk" => {
            if values.len() != 1 {
                return Err(format!("{operation} expects a path"));
            }
            let Value::String(path) = &values[0] else {
                return Err(format!("{operation} expects a path"));
            };
            Ok(file_effect(
                &effect_operation,
                &path,
                None,
                |provider| match operation {
                    "std.native.File/read" => provider.read(&path),
                    "std.native.File/exists?" => provider.exists(&path),
                    "std.native.File/stat" => provider.stat(&path),
                    "std.native.File/entries" => provider.entries(&path),
                    "std.native.File/list" => provider.list(&path),
                    "std.native.File/walk" => provider.walk(&path),
                    _ => unreachable!(),
                },
            ))
        }
        "std.native.File/write" => {
            if !(2..=3).contains(&values.len()) {
                return Err(
                    "std.native.File/write expects a path, bytes, and optional options".into(),
                );
            }
            let Value::String(path) = &values[0] else {
                return Err("std.native.File/write expects a path and bytes".into());
            };
            let bytes = match &values[1] {
                Value::Bytes(value) => value.clone(),
                Value::ByteBuffer(value) => value.borrow().clone(),
                _ => return Err("std.native.File/write expects a path and bytes".into()),
            };
            let options = if values.len() == 3 {
                file_options_value(values[2].clone(), operation)?
            } else {
                Value::Map(PMap::new())
            };
            let options = file_write_options(&options)?;
            Ok(file_effect(&effect_operation, &path, None, |provider| {
                provider.write_with_options(&path, bytes, options)
            }))
        }
        "std.native.File/mkdir" => {
            if !(1..=2).contains(&values.len()) {
                return Err("std.native.File/mkdir expects a path and optional options".into());
            }
            let Value::String(path) = &values[0] else {
                return Err("std.native.File/mkdir expects a path".into());
            };
            let options = if values.len() == 2 {
                file_options_value(values[1].clone(), operation)?
            } else {
                Value::Map(PMap::new())
            };
            let options = file_mkdir_options(&options)?;
            Ok(file_effect(&effect_operation, &path, None, |provider| {
                provider.mkdir_with_options(&path, options)
            }))
        }
        "std.native.File/delete" => {
            if !(1..=2).contains(&values.len()) {
                return Err("std.native.File/delete expects a path and optional options".into());
            }
            let Value::String(path) = &values[0] else {
                return Err("std.native.File/delete expects a path".into());
            };
            let options = if values.len() == 2 {
                file_options_value(values[1].clone(), operation)?
            } else {
                Value::Map(PMap::new())
            };
            let options = file_delete_options(&options)?;
            Ok(file_effect(&effect_operation, &path, None, |provider| {
                provider.delete_with_options(&path, options)
            }))
        }
        "std.native.File/copy" | "std.native.File/move" => {
            if !(2..=3).contains(&values.len()) {
                return Err(format!(
                    "{operation} expects source, target, and optional options"
                ));
            }
            let Value::String(source) = &values[0] else {
                return Err(format!("{operation} expects source and target paths"));
            };
            let Value::String(target) = &values[1] else {
                return Err(format!("{operation} expects source and target paths"));
            };
            let options = if values.len() == 3 {
                file_options_value(values[2].clone(), operation)?
            } else {
                Value::Map(PMap::new())
            };
            Ok(if operation == "std.native.File/copy" {
                let options = file_copy_options(&options)?;
                file_effect(&effect_operation, &source, Some(&target), |provider| {
                    provider.copy(&source, &target, options)
                })
            } else {
                let options = file_move_options(&options)?;
                file_effect(&effect_operation, &source, Some(&target), |provider| {
                    provider.move_entry(&source, &target, options)
                })
            })
        }
        "std.native.File/temp-file" | "std.native.File/temp-directory" => {
            if !(1..=2).contains(&values.len()) {
                return Err(format!("{operation} expects a parent and optional options"));
            }
            let Value::String(parent) = &values[0] else {
                return Err(format!("{operation} expects a parent path"));
            };
            let options = if values.len() == 2 {
                file_options_value(values[1].clone(), operation)?
            } else {
                Value::Map(PMap::new())
            };
            Ok(if operation == "std.native.File/temp-file" {
                let options = TempFileOptions {
                    prefix: file_string_option(&options, "prefix", "tmp", operation)?,
                    suffix: file_string_option(&options, "suffix", "", operation)?,
                };
                file_effect(&effect_operation, &parent, None, |provider| {
                    provider.temp_file(&parent, options)
                })
            } else {
                let options = TempDirectoryOptions {
                    prefix: file_string_option(&options, "prefix", "tmp", operation)?,
                };
                file_effect(&effect_operation, &parent, None, |provider| {
                    provider.temp_directory(&parent, options)
                })
            })
        }
        _ => Err(format!("unknown std.native.File operation: {operation}")),
    }
}

fn socket_values(operation: &str, values: Vec<Value>) -> Result<Value, String> {
    let operation = operation
        .strip_prefix("std.native.Socket/")
        .unwrap_or(operation);
    match operation {
        "receive-stream" | "socket/receive-stream" => {
            if values.len() != 1 {
                return Err(format!("Socket/{operation} expects a socket connection"));
            }
            let socket = socket_handle(&values[0], &format!("Socket/{operation}"))?;
            let events = socket_provider(operation)?
                .events(socket)
                .map_err(|e| socket_error(operation, e))?;
            Ok(host_stream(
                Rc::new(move || socket_receive_promise(events)),
                Rc::new(|| Ok(())),
            ))
        }
        "socket/connect" => {
            if values.len() != 4 {
                return Err("socket/connect expects a host, port, options, and callback".into());
            }
            let host = match &values[0] {
                Value::String(value) => value.clone(),
                _ => {
                    return Err("socket/connect expects a host, port, options, and callback".into())
                }
            };
            let port = value_u16_integer(&values[1], "socket/connect", false)?;
            let _options = &values[2];
            let callback = match &values[3] {
                Value::Function(value) => value.clone(),
                _ => return Err("socket/connect expects a callback".into()),
            };
            let callback = Rc::new(move |event| {
                let arguments = match event {
                    SocketEvent::Connected(handle) => {
                        vec![Value::Nil, Value::Number(handle as i64)]
                    }
                    SocketEvent::Failed(_, error) => vec![Value::String(error), Value::Nil],
                    SocketEvent::Data(_, _) | SocketEvent::Closed(_) => return,
                };
                let _ = call_function(&callback, arguments);
            });
            socket_provider(operation)?
                .connect(&host, port, callback)
                .map(|handle| Value::Number(handle as i64))
                .map_err(|error| socket_error(operation, error))
        }
        "socket/listen" => {
            if values.len() != 4 {
                return Err("socket/listen expects a host, port, options, and callback".into());
            }
            let host = match &values[0] {
                Value::String(value) => value.clone(),
                _ => return Err("socket/listen expects a host string".into()),
            };
            let port = value_u16_integer(&values[1], "socket/listen", true)?;
            let _options = &values[2];
            let callback = match &values[3] {
                Value::Function(value) => value.clone(),
                _ => return Err("socket/listen expects a callback".into()),
            };
            let callback = Rc::new(move |event| {
                let _ = call_function(&callback, vec![socket_server_event_value(event)]);
            });
            socket_provider(operation)?
                .listen(&host, port, callback)
                .map(|handle| Value::Number(handle as i64))
                .map_err(|error| socket_error(operation, error))
        }
        "socket/endpoint" => {
            if values.len() != 1 {
                return Err("socket/endpoint expects a server".into());
            }
            let server = socket_handle(&values[0], "socket/endpoint")?;
            socket_provider(operation)?
                .endpoint(server)
                .map(|(host, port)| {
                    Value::Map(PMap::from_iter([
                        (Value::Keyword("host".into()), Value::String(host)),
                        (Value::Keyword("port".into()), Value::Number(port as i64)),
                    ]))
                })
                .map_err(|error| socket_error(operation, error))
        }
        "socket/events" => {
            if values.len() != 2 {
                return Err("socket/events expects a socket handle and options".into());
            }
            let handle = socket_handle(&values[0], "socket/events")?;
            let _options = &values[1];
            socket_provider(operation)?
                .events(handle)
                .map(|stream| Value::Number(stream as i64))
                .map_err(|error| socket_error(operation, error))
        }
        "socket/next" => {
            if values.len() != 1 {
                return Err("socket/next expects a socket stream".into());
            }
            let stream = socket_handle(&values[0], "socket/next")?;
            socket_provider(operation)?
                .next(stream)
                .map(Value::Promise)
                .map_err(|error| socket_error(operation, error))
        }
        "socket/send" => {
            if values.len() != 2 {
                return Err("socket/send expects a socket connection and bytes".into());
            }
            let socket = socket_handle(&values[0], "socket/send")?;
            let bytes = match &values[1] {
                Value::Bytes(value) => value.clone(),
                Value::ByteBuffer(value) => value.borrow().clone(),
                _ => return Err("socket/send expects a socket connection and bytes".into()),
            };
            socket_provider(operation)?
                .send(socket, &bytes)
                .map(|count| Value::Number(count as i64))
                .map_err(|error| socket_error(operation, error))
        }
        "socket/close" => {
            if values.len() != 1 {
                return Err("socket/close expects a socket connection".into());
            }
            let socket = socket_handle(&values[0], "socket/close")?;
            socket_provider(operation)?
                .close(socket)
                .map(|()| Value::Nil)
                .map_err(|error| socket_error(operation, error))
        }
        _ => Err(format!("unknown std.native.Socket operation: {operation}")),
    }
}

fn socket_receive_promise(stream: SocketHandle) -> Result<Promise, String> {
    let source = socket_provider("Socket/receive-stream")?
        .next(stream)
        .map_err(|e| socket_error("Socket/receive-stream", e))?;
    let output = Promise::new();
    let settled = output.clone();
    source.on_settle(Rc::new(move |result| match result {
        PromiseState::Rejected(error) => {
            settled.reject_rejection(error);
        }
        PromiseState::Pending => {}
        PromiseState::Fulfilled(event) => {
            let entries = map_entries(&event).unwrap_or_default();
            let kind = entries.iter().find_map(|(k, v)| {
                if matches!(k, Value::Keyword(key) if key.as_str() == "type") {
                    Some(v.clone())
                } else {
                    None
                }
            });
            match kind {
                Some(Value::Keyword(kind)) if kind.as_str() == "data" => {
                    let bytes = entries
                        .into_iter()
                        .find_map(|(k, v)| {
                            if matches!(k, Value::Keyword(key) if key.as_str() == "bytes") {
                                Some(v)
                            } else {
                                None
                            }
                        })
                        .unwrap_or(Value::Nil);
                    settled.resolve(bytes);
                }
                Some(Value::Keyword(kind)) if kind.as_str() == "close" => {
                    settled.resolve(Value::Nil);
                }
                Some(Value::Keyword(kind)) if kind.as_str() == "error" => {
                    settled.reject("socket receive failed");
                }
                _ => {
                    settled.reject("Socket/receive-stream received an invalid event");
                }
            }
        }
    }));
    let poll = source.clone();
    output.set_poller(Rc::new(move || {
        poll.state();
    }));
    let wait = source.clone();
    output.set_waiter(Rc::new(move || {
        wait.wait_state();
    }));
    Ok(output)
}

fn socket_handle(value: &Value, operation: &str) -> Result<SocketHandle, String> {
    value_u64_integer(value, operation)
        .map(|value| value as SocketHandle)
        .map_err(|_| format!("{operation} expects a socket handle"))
}

fn native_host_values(operation: &str, values: Vec<Value>) -> Result<Value, String> {
    let method = operation
        .strip_prefix("std.native.Host/")
        .unwrap_or(operation);
    let (service, target, arguments) = match method {
        "call" => {
            if values.len() != 3 {
                return Err(
                    "std.native.Host/call expects service, method, and an argument vector".into(),
                );
            }
            let service = match &values[0] {
                Value::String(value) => value.clone(),
                _ => return Err("std.native.Host/call service must be a string".into()),
            };
            let target = match &values[1] {
                Value::String(value) => value.clone(),
                _ => return Err("std.native.Host/call method must be a string".into()),
            };
            let arguments = match &values[2] {
                Value::Vector(values) => values.iter().cloned().collect(),
                Value::Tuple(values) => values.iter().cloned().collect(),
                _ => return Err("std.native.Host/call arguments must be a vector".into()),
            };
            (service, target, arguments)
        }
        "describe" | "capabilities" => {
            if !values.is_empty() {
                return Err(format!("std.native.Host/{method} expects no arguments"));
            }
            ("host".into(), method.into(), Vec::new())
        }
        "capability?" => {
            if values.len() != 1 {
                return Err("std.native.Host/capability? expects one capability".into());
            }
            (
                "host".into(),
                "capability?".into(),
                vec![values[0].clone()],
            )
        }
        _ => return Err(format!("unknown std.native.Host method: {method}")),
    };
    HOST_CALL_HANDLER.with(|active| {
        let Some(handler) = active.borrow().as_ref().cloned() else {
            let promise = Promise::new();
            promise.reject_value(host_error(
                "host/unavailable",
                "Host capability provider is unavailable",
            ));
            return Ok(Value::Promise(promise));
        };
        handler(service, target, arguments)
    })
}

pub(crate) fn namespace_identifier(value: Value, operation: &str) -> Result<String, String> {
    match value {
        Value::Symbol(name) if name.get_namespace().is_none() => Ok(name.as_str().to_owned()),
        Value::String(name) => Ok(name),
        Value::Namespace(namespace) => Ok(namespace.name().as_str().to_owned()),
        _ => Err(format!(
            "{operation} expects an unqualified namespace symbol, string, or Namespace"
        )),
    }
}

fn namespace_descriptor(registry: &NamespaceRegistry<Value>, name: &str) -> Value {
    let state = registry
        .load_state(name)
        .or_else(|| registry.find(name).map(|_| NamespaceLoadState::Loaded))
        .map(NamespaceLoadState::as_str)
        .unwrap_or("unknown");
    let package = package_catalog().coordinate_for_namespace(name);
    let origin = if name.starts_with("std.native") {
        "embedded"
    } else if package.is_some() {
        "package"
    } else if registry.find(name).is_some() {
        "runtime"
    } else {
        "registered"
    };
    let mut fields = vec![
        (
            Value::Keyword("namespace/name".into()),
            Value::Symbol(Symbol::parse(name)),
        ),
        (
            Value::Keyword("namespace/state".into()),
            Value::Keyword(state.into()),
        ),
        (
            Value::Keyword("namespace/role".into()),
            Value::Keyword(
                registry
                    .find(name)
                    .map(|namespace| namespace.role())
                    .unwrap_or_else(|| "standard".into())
                    .into(),
            ),
        ),
        (
            Value::Keyword("namespace/revision".into()),
            Value::Number(registry.module_revision(name) as i64),
        ),
        (
            Value::Keyword("namespace/origin".into()),
            Value::Keyword(origin.into()),
        ),
    ];
    if let Some(package) = package {
        fields.push((
            Value::Keyword("namespace/package".into()),
            Value::String(package),
        ));
    }
    Value::OrderedMap(Box::new(POrderedMap::from_iter(fields)))
}

fn native_runtime_values(
    operation: &str,
    values: Vec<Value>,
    env: &mut HashMap<String, Value>,
) -> Result<Value, String> {
    let method = operation
        .strip_prefix("std.native.Runtime/")
        .unwrap_or(operation);
    let registry = namespace_registry()?;
    match method {
        "ns-publics" => {
            let namespace = match values.as_slice() {
                [Value::Symbol(name)] if name.get_namespace().is_none() => name.as_str().to_owned(),
                [Value::String(name)] => name.clone(),
                [Value::Namespace(namespace)] => namespace.name().as_str().to_owned(),
                _ => return Err("std.native.Runtime/ns-publics expects a namespace symbol or string".into()),
            };
            let target = registry
                .find(&namespace)
                .ok_or_else(|| format!("No such namespace: {namespace}"))?;
            let mut mappings = target.mappings();
            mappings.retain(|(_, var)| var.symbol().get_namespace() == Some(namespace.as_str()));
            mappings.sort_by(|(left, _), (right, _)| left.as_str().cmp(right.as_str()));
            Ok(Value::OrderedMap(Box::new(POrderedMap::from_iter(
                mappings.into_iter().map(|(name, var)| {
                    (Value::Symbol(Symbol::create(None, name.as_str())), Value::Var(var))
                }),
            ))))
        }
        "ns-aliases" => {
            let name = match values.as_slice() {
                [value] => namespace_identifier(value.clone(), "std.native.Runtime/ns-aliases")?,
                _ => return Err("std.native.Runtime/ns-aliases expects one namespace".into()),
            };
            let target = registry
                .find(&name)
                .ok_or_else(|| format!("No such namespace: {name}"))?;
            let mut aliases = target.aliases();
            aliases.sort_by(|(left, _), (right, _)| left.as_str().cmp(right.as_str()));
            Ok(Value::OrderedMap(Box::new(POrderedMap::from_iter(
                aliases.into_iter().map(|(alias, namespace)| {
                    (Value::Symbol(alias), Value::Namespace(Rc::new(namespace)))
                }),
            ))))
        }
        "ns-find" => {
            if values.len() != 1 {
                return Err("std.native.Runtime/ns-find expects one namespace".into());
            }
            let name = namespace_identifier(values[0].clone(), "std.native.Runtime/ns-find")?;
            Ok(registry
                .find(&name)
                .map(|namespace| Value::Namespace(Rc::new(namespace)))
                .unwrap_or(Value::Nil))
        }
        "ns-create" => match values.as_slice() {
            [Value::Symbol(name)] if name.get_namespace().is_none() => {
                let namespace = registry.find_or_create(name.as_str());
                Ok(Value::Namespace(Rc::new(namespace)))
            }
            _ => Err("std.native.Runtime/ns-create expects an unqualified symbol".into()),
        },
        "ns-name" => match values.as_slice() {
            [Value::Namespace(namespace)] => Ok(Value::Symbol(namespace.name().clone())),
            [Value::Symbol(name)]
                if name.get_namespace().is_none()
                    && namespace_registry()?.find(name.as_str()).is_some() =>
            {
                Ok(Value::Symbol(name.clone()))
            }
            _ => Err("std.native.Runtime/ns-name expects a namespace".into()),
        },
        "current" => {
            if !values.is_empty() {
                return Err("std.native.Runtime/current expects no arguments".into());
            }
            Ok(Value::Symbol(registry.current().name().clone()))
        }
        "snapshot" => {
            if !values.is_empty() {
                return Err("std.native.Runtime/snapshot expects no arguments".into());
            }
            let namespaces = registry
                .known_names()
                .into_iter()
                .map(|name| namespace_descriptor(&registry, name.as_str()))
                .collect::<Vec<_>>();
            Ok(Value::OrderedMap(Box::new(POrderedMap::from_iter([
                (
                    Value::Keyword("env/current".into()),
                    Value::Symbol(registry.current().name().clone()),
                ),
                (
                    Value::Keyword("env/namespaces".into()),
                    Value::Vector(PVector::from(namespaces)),
                ),
            ]))))
        }
        "namespaces" => {
            if !values.is_empty() {
                return Err("std.native.Runtime/namespaces expects no arguments".into());
            }
            Ok(Value::Vector(PVector::from(
                registry
                    .known_names()
                    .into_iter()
                    .map(|name| namespace_descriptor(&registry, name.as_str()))
                    .collect::<Vec<_>>(),
            )))
        }
        "namespace" => {
            if values.len() != 1 {
                return Err("std.native.Runtime/namespace expects one namespace".into());
            }
            let name = namespace_identifier(values[0].clone(), operation)?;
            if registry.load_state(&name).is_none() && registry.find(&name).is_none() {
                Ok(Value::Nil)
            } else {
                Ok(namespace_descriptor(&registry, &name))
            }
        }
        "module" => {
            if values.len() != 1 {
                return Err("std.native.Runtime/module expects one module path".into());
            }
            let requested = match &values[0] {
                Value::String(path) => path.clone(),
                Value::Symbol(name) => name.as_str().to_owned(),
                _ => {
                    return Err(
                        "std.native.Runtime/module expects a path string or namespace symbol"
                            .into(),
                    )
                }
            };
            let source = requested.strip_prefix("classpath:").unwrap_or(&requested);
            let namespace = if source.ends_with(".hal") || source.ends_with(".hrl") {
                source
                    .trim_end_matches(".hal")
                    .trim_end_matches(".hrl")
                    .trim_start_matches("./")
                    .replace('/', ".")
            } else {
                source.to_owned()
            };
            let revision = registry.module_revision(&namespace);
            if revision == 0
                && registry.load_state(&namespace).is_none()
                && registry.find(&namespace).is_none()
            {
                return Ok(Value::Nil);
            }
            let dependencies = registry
                .module_dependencies(&namespace)
                .into_iter()
                .map(|dependency| {
                    Value::String(format!("{}.hal", dependency.as_str().replace('.', "/")))
                })
                .collect::<Vec<_>>();
            Ok(Value::OrderedMap(Box::new(POrderedMap::from_iter([
                (
                    Value::Keyword("module/path".into()),
                    Value::String(requested),
                ),
                (
                    Value::Keyword("module/namespace".into()),
                    Value::Symbol(Symbol::parse(&namespace)),
                ),
                (
                    Value::Keyword("module/revision".into()),
                    Value::Number(revision as i64),
                ),
                (
                    Value::Keyword("module/dependencies".into()),
                    Value::Vector(PVector::from(dependencies)),
                ),
            ]))))
        }
        "vars" => {
            if values.len() > 1 {
                return Err("std.native.Runtime/vars expects zero or one namespace".into());
            }
            let name = if values.is_empty() {
                registry.current().name().as_str().to_owned()
            } else {
                namespace_identifier(values[0].clone(), operation)?
            };
            let namespace = registry
                .find(&name)
                .ok_or_else(|| format!("namespace/not-found: {name}"))?;
            let mut mappings = namespace.mappings();
            mappings.retain(|(_, var)| var.symbol().get_namespace() == Some(name.as_str()));
            mappings.sort_by(|(left, _), (right, _)| left.as_str().cmp(right.as_str()));
            Ok(Value::OrderedMap(Box::new(POrderedMap::from_iter(
                mappings.into_iter().map(|(symbol, var)| {
                    (
                        Value::Symbol(Symbol::create(None, symbol.as_str())),
                        Value::Var(var),
                    )
                }),
            ))))
        }
        "eval" => {
            if values.len() != 1 {
                return Err("std.native.Runtime/eval expects one form".into());
            }
            let mut environment = crate::core::current_namespace_environment()?;
            let result = eval_value(values[0].clone(), &mut environment);
            crate::core::save_namespace_environment(&registry, &mut environment);
            result
        }
        "load-string" => {
            let [Value::String(source)] = values.as_slice() else {
                return Err("std.native.Runtime/load-string expects one string".into());
            };
            #[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
            if direct_native_execution() {
                return eval_direct_native_source(source);
            }
            eval_value_text(source, env)
        }
        "var-sym" => {
            let [Value::Var(var)] = values.as_slice() else {
                return Err("std.native.Runtime/var-sym expects one Var".into());
            };
            Ok(Value::Symbol(var.symbol().clone()))
        }
        "macroexpand-1" => {
            if values.len() != 1 {
                return Err("std.native.Runtime/macroexpand-1 expects one form".into());
            }
            let form = value_to_form(&values[0])?;
            let mut environment = current_namespace_environment()?;
            form_to_value(&macroexpand_once(&form, &mut environment)?)
        }
        "gensym" => {
            let prefix = match values.as_slice() {
                [] => "G__".to_owned(),
                [Value::String(prefix)] => prefix.clone(),
                [value] => return Err(format!("gensym expects a string prefix, got {}", portable_type_name(value))),
                _ => return Err("gensym expects zero or one arguments".into()),
            };
            Ok(Value::Symbol(Symbol::from(gensym(&prefix))))
        }
        "eval-in" => {
            if values.len() != 2 {
                return Err("std.native.Runtime/eval-in expects namespace and forms".into());
            }
            let target = namespace_identifier(values[0].clone(), operation)?;
            if registry.find(&target).is_none() {
                return Err(format!(
                    "std.native.Runtime/eval-in requires an existing namespace: {target}"
                ));
            }
            let forms = iterator_values(values[1].clone())?
                .into_iter()
                .map(|value| value_to_form(&value))
                .collect::<Result<Vec<_>, _>>()?;
            let previous = registry.current().name().as_str().to_owned();
            select_namespace_environment(&registry, env, &target);
            #[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
            let result = if direct_native_execution() {
                let source = if forms.is_empty() {
                    "nil".to_owned()
                } else {
                    Form::List(
                        std::iter::once(Form::Symbol("do".into()))
                            .chain(forms.iter().cloned())
                            .collect(),
                    )
                    .to_string()
                };
                eval_direct_native_source(&source)
            } else {
                let mut result = Value::Nil;
                for form in &forms {
                    result = eval(form, env)?;
                }
                Ok(result)
            };
            #[cfg(not(all(feature = "direct-native", not(target_arch = "wasm32"))))]
            let result = {
                let mut result = Value::Nil;
                for form in &forms {
                    result = eval(form, env)?;
                }
                Ok(result)
            };
            select_namespace_environment(&registry, env, &previous);
            result
        }
        "alias-state" => {
            if values.len() != 1 && values.len() != 2 {
                return Err(
                    "std.native.Runtime/alias-state expects alias or namespace and alias".into(),
                );
            }
            let (owner, alias_value) = if values.len() == 2 {
                (namespace_identifier(values[0].clone(), operation)?, &values[1])
            } else {
                (registry.current().name().as_str().to_owned(), &values[0])
            };
            let Value::Symbol(alias) = alias_value else {
                return Err(
                    "std.native.Runtime/alias-state expects an unqualified alias symbol".into(),
                );
            };
            if alias.get_namespace().is_some() {
                return Err(
                    "std.native.Runtime/alias-state expects an unqualified alias symbol".into(),
                );
            }
            let Some(namespace) = registry.find(&owner) else {
                return Ok(Value::Nil);
            };
            // Global aliases are resolver-visible even when the owning namespace
            // has not materialized a local alias entry. Report the same target
            // and lifecycle state that qualified symbol resolution observes.
            let target = namespace.lazy_target(alias.as_str()).or_else(|| {
                namespace
                    .aliases()
                    .into_iter()
                    .find(|(name, _)| name == alias)
                    .map(|(_, target)| target.name().clone())
            }).or_else(|| {
                registry
                    .global_aliases()
                    .into_iter()
                    .find(|(name, _)| name == alias)
                    .map(|(_, target)| target)
            });
            let Some(target) = target else {
                return Ok(Value::Nil);
            };
            let state = registry
                .load_state(target.as_str())
                .or_else(|| {
                    registry
                        .find(target.as_str())
                        .map(|_| NamespaceLoadState::Loaded)
                })
                .map(NamespaceLoadState::as_str)
                .unwrap_or("unknown");
            Ok(Value::Map(PMap::from_iter([
                (Value::Keyword("alias".into()), Value::Symbol(alias.clone())),
                (Value::Keyword("target".into()), Value::Symbol(target)),
                (Value::Keyword("state".into()), Value::Keyword(state.into())),
            ])))
        }
        "intern-var" => {
            if values.len() != 3 && values.len() != 4 {
                return Err(
                    "std.native.Runtime/intern-var expects namespace, symbol, Var, and optional metadata"
                        .into(),
                );
            }
            let target = namespace_identifier(values[0].clone(), operation)?;
            let Value::Symbol(name) = &values[1] else {
                return Err(
                    "std.native.Runtime/intern-var expects an unqualified target symbol".into(),
                );
            };
            if name.get_namespace().is_some() {
                return Err(
                    "std.native.Runtime/intern-var expects an unqualified target symbol".into(),
                );
            }
            let Value::Var(source) = &values[2] else {
                return Err("std.native.Runtime/intern-var expects a source Var".into());
            };
            let mut metadata = source.metadata();
            if let Some(extension) = values.get(3) {
                let Some(entries) = map_entries(extension) else {
                    return Err(
                        "std.native.Runtime/intern-var metadata extension must be a map".into(),
                    );
                };
                for (key, value) in entries {
                    metadata.extra.insert(key.display(), value.display());
                }
            }
            let value = source.deref_value();
            if let Value::Function(function) = &value {
                if function.is_macro {
                    ACTIVE_MACROS.with(|active| {
                        if let Some(macros) = active.borrow().as_ref() {
                            macros.borrow_mut().insert(
                                (target.clone(), name.as_str().to_owned()),
                                function.clone(),
                            );
                        }
                    });
                }
            }
            Ok(Value::Var(
                registry.find_or_create(&target).intern_with_metadata(
                    name.as_str(),
                    value,
                    metadata,
                ),
            ))
        }
        _ => Err(format!("unknown std.native.Runtime method: {method}")),
    }
}

#[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
fn eval_direct_native_source(source: &str) -> Result<Value, String> {
    let context = DirectNativeContext::capture();
    let forms = crate::kernel::read_forms(source).map_err(|error| error.to_string())?;
    let has_namespace_form = forms.iter().any(|form| {
        matches!(
            form_without_metadata(&form.form),
            Form::List(items)
                if matches!(items.first(), Some(Form::Symbol(operator)) if operator == "ns" || operator == "ns+")
        )
    });
    let config = if has_namespace_form {
        crate::vm::source_namespace_config(&forms).map_err(|error| error.to_string())?
    } else {
        crate::kernel::GeneratedNamespaceConfig::defaults()
    };
    let namespaces = context.namespaces.clone();
    let program = context
        .with(|| {
            without_direct_native_execution(|| {
                crate::vm::compile_source_with_config_allow_unbound_globals(
                    source,
                    &namespaces,
                    config,
                )
            })
        })
        .map_err(|error| error.to_string())?;
    let mut program = program;
    if program.namespace.is_none() {
        program.namespace = Some(context.namespace.clone());
    }
    let engine = crate::direct_native::NativeEngine::new();
    // `context.with` already installs the captured providers, namespace, and
    // multimethod map. Calling the convenience entry point here would wrap
    // the same map in a second context and restore an old snapshot over any
    // declarations made by the nested program.
    let result = context.with(|| engine.execute_blocking(Rc::new(program)));
    // Nested Runtime/eval calls execute with a captured multimethod map. Carry
    // declarations made there back into the enclosing native frame so a later
    // form in the same frame can install methods or invoke the new multifn.
    let nested_multimethods = context.multimethods.borrow().clone();
    ACTIVE_MULTIMETHODS.with(|active| {
        active.borrow_mut().extend(nested_multimethods);
    });
    result.map(|report| report.value)
}

#[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
fn eval_direct_native_form(form: &Form) -> Result<Value, String> {
    eval_direct_native_source(&form.to_string())
}

#[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
fn eval_direct_native_function(parts: &[Form], declaration: &str) -> Result<Rc<Function>, String> {
    let mut function = Vec::with_capacity(parts.len() + 1);
    function.push(Form::Symbol("fn".into()));
    function.extend_from_slice(parts);
    let value = eval_direct_native_form(&Form::List(function))?;
    match value {
        Value::Function(function) => Ok(function),
        _ => Err(format!("{declaration} function did not produce a callable")),
    }
}

/// Handles declaration instructions without routing their nested expressions
/// or function bodies back through the tree evaluator. These forms mutate
/// protocol/multimethod registries, so the compiler keeps their declaration
/// form as a constant and this VM-side adapter performs the mutation.
#[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
fn eval_direct_native_declaration(expected_operator: &str, form: &Form) -> Result<Value, String> {
    let Form::List(items) = form_without_metadata(form) else {
        unreachable!("declaration validation checked the form shape")
    };
    match expected_operator {
        "defprotocol" => {
            if items.len() < 3 {
                return Err("defprotocol expects a name and method declarations".into());
            }
            let name = match form_without_metadata(&items[1]) {
                Form::Symbol(name) if !name.contains('/') => name.clone(),
                _ => return Err("defprotocol name must be an unqualified symbol".into()),
            };
            let mut methods = HashMap::new();
            for declaration in &items[2..] {
                let Form::List(parts) = form_without_metadata(declaration) else {
                    return Err("defprotocol method declaration must be a list".into());
                };
                if parts.len() != 2
                    || !matches!(form_without_metadata(&parts[0]), Form::Symbol(_))
                    || !matches!(form_without_metadata(&parts[1]), Form::Vector(_))
                {
                    return Err(
                        "defprotocol method declaration expects a name and parameter vector".into(),
                    );
                }
                let Form::Symbol(method) = form_without_metadata(&parts[0]) else {
                    unreachable!()
                };
                let Form::Vector(arguments) = form_without_metadata(&parts[1]) else {
                    unreachable!()
                };
                if arguments.is_empty() || methods.insert(method.clone(), arguments.len()).is_some()
                {
                    return Err("protocol methods must be unique and take a receiver".into());
                }
            }
            publish_guest_protocol(&name, methods, Vec::new(), &mut HashMap::new())
        }
        "extend-type" => {
            if items.len() < 4 {
                return Err(
                    "extend-type expects a type, protocol, and method implementations".into(),
                );
            }
            let type_value = eval_direct_native_form(&items[1])?;
            let type_name = match type_value {
                Value::StructType(ty) => ty.name.clone(),
                Value::MutableType(ty) => ty.name.clone(),
                _ => return Err("extend-type expects a struct or mutable type".into()),
            };
            let protocol = match eval_direct_native_form(&items[2])? {
                Value::Protocol(protocol) => protocol,
                Value::Var(var) => match var.deref_value() {
                    Value::Protocol(protocol) => protocol,
                    _ => return Err("extend-type expects a protocol".into()),
                },
                _ => return Err("extend-type expects a protocol".into()),
            };
            let mut seen = HashSet::new();
            let mut implementations = Vec::with_capacity(items.len() - 3);
            for implementation in &items[3..] {
                let Form::List(parts) = form_without_metadata(implementation) else {
                    return Err("extend-type implementations must be method forms".into());
                };
                if parts.len() < 3 {
                    return Err("extend-type implementations require a body".into());
                }
                let Form::Symbol(method) = form_without_metadata(&parts[0]) else {
                    return Err("extended method name must be a symbol".into());
                };
                let Form::Vector(arguments) = form_without_metadata(&parts[1]) else {
                    return Err("extended method arguments must be a vector".into());
                };
                if !seen.insert(method.clone()) {
                    return Err("Duplicate extended method".into());
                }
                let valid_arity = protocol.methods.get(method).is_some_and(|expected| {
                    *expected == arguments.len()
                        || (*expected == usize::MAX && !arguments.is_empty())
                });
                if !valid_arity {
                    return Err(format!("invalid protocol method implementation: {method}"));
                }
                implementations.push((
                    method.clone(),
                    eval_direct_native_function(&parts[1..], "extend-type")?,
                ));
            }
            let registry = active_protocol_registry()?;
            for (method, function) in implementations {
                registry.register_guest(protocol.name.clone(), type_name.clone(), method, function);
            }
            Ok(Value::Protocol(protocol))
        }
        "defmulti" => {
            if items.len() != 3 {
                return Err("defmulti expects a name and dispatch function".into());
            }
            let Form::Symbol(name) = form_without_metadata(&items[1]) else {
                return Err("defmulti name must be an unqualified symbol".into());
            };
            if name.contains('/') {
                return Err("defmulti name must be an unqualified symbol".into());
            }
            let Value::Function(dispatch) = eval_direct_native_form(&items[2])? else {
                return Err("defmulti dispatch function must be callable".into());
            };
            let registry = namespace_registry()?;
            let namespace = registry.current();
            let qualified = format!("{}/{}", namespace.name().as_str(), name);
            let state = Rc::new(RefCell::new(MultiMethod {
                dispatch,
                methods: Vec::new(),
                default: None,
            }));
            let invoke_state = state.clone();
            let value = native_variadic_function(&qualified, move |arguments| {
                let state = invoke_state.borrow();
                let key = call_function(&state.dispatch, arguments.clone())?;
                let method = state
                    .methods
                    .iter()
                    .find(|(candidate, _)| *candidate == key)
                    .map(|(_, method)| method.clone())
                    .or_else(|| state.default.clone())
                    .ok_or_else(|| {
                        format!("No multimethod method for dispatch value {}", key.display())
                    })?;
                call_function(&method, arguments)
            });
            let var = namespace.intern(name, value.clone());
            var.set_origin(definition_origin());
            register_multimethod(qualified, state);
            Ok(value)
        }
        "defmethod" => {
            if items.len() < 5 {
                return Err(
                    "defmethod expects a multifn, dispatch value, parameters, and body".into(),
                );
            }
            let Form::Symbol(name) = form_without_metadata(&items[1]) else {
                return Err("defmethod multifn must be a symbol".into());
            };
            let namespace = namespace_registry()?.current().name().as_str().to_owned();
            let qualified = if name.contains('/') {
                name.clone()
            } else {
                format!("{namespace}/{name}")
            };
            let key = eval_direct_native_form(&items[2])?;
            let function = eval_direct_native_function(&items[3..], "defmethod")?;
            let state = multimethod_state(&qualified)
                .ok_or_else(|| "defmethod expects an existing multifn".to_string())?;
            let mut state = state.borrow_mut();
            if matches!(
                &key,
                Value::Keyword(keyword)
                    if keyword.get_namespace().is_none() && keyword.get_name() == "default"
            ) {
                state.default = Some(function);
            } else if let Some((_, existing)) = state
                .methods
                .iter_mut()
                .find(|(candidate, _)| *candidate == key)
            {
                *existing = function;
            } else {
                state.methods.push((key, function));
            }
            Ok(Value::Nil)
        }
        _ => Err(format!(
            "unsupported native declaration: {expected_operator}"
        )),
    }
}

fn eval_value(value: Value, env: &mut HashMap<String, Value>) -> Result<Value, String> {
    #[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
    if direct_native_execution() {
        return eval_direct_native_source(&value_to_form(&value)?.to_string());
    }
    eval(&value_to_form(&value)?, env)
}

fn native_package_values(
    operation: &str,
    arguments: Vec<Value>,
    env: &mut HashMap<String, Value>,
) -> Result<Value, String> {
    let method = operation
        .strip_prefix("std.native.Package/")
        .unwrap_or(operation);
    let expected = match method {
        "catalog" => 0..=0,
        "find" | "ensure" | "load" | "state" => 1..=1,
        "unload" => 1..=2,
        _ => return Err(format!("unknown std.native.Package method: {method}")),
    };
    if !expected.contains(&arguments.len()) {
        return Err(format!(
            "std.native.Package/{method} expects {} arguments",
            expected.start()
        ));
    }
    let catalog = package_catalog();
    if method == "catalog" {
        return Ok(catalog.catalog_value());
    }
    let target = match arguments.first() {
        Some(Value::Symbol(value)) => value.as_str().to_owned(),
        Some(Value::String(value)) => value.clone(),
        Some(Value::Keyword(value)) => value.as_str().to_owned(),
        Some(value @ Value::OrderedMap(_)) if method == "ensure" || method == "unload" => {
            package_descriptor_coordinate(value).ok_or_else(|| {
                format!("std.native.Package/{method} descriptor requires :package/coordinate")
            })?
        }
        _ => {
            return Err(format!(
                "std.native.Package/{method} expects a namespace, coordinate, or exact descriptor"
            ))
        }
    };
    let found = catalog.find(&target);
    if method == "find" {
        return Ok(found.map(|(_, value)| value).unwrap_or(Value::Nil));
    }
    let Some((coordinate, descriptor)) = found else {
        if method == "state" {
            return Ok(Value::Nil);
        }
        return Err(format!("package/not-locked: {target}"));
    };
    if method == "state" {
        return Ok(Value::Keyword(
            catalog
                .state(&coordinate)
                .unwrap_or_else(|| "available".into())
                .into(),
        ));
    }
    if method == "load" {
        if catalog.coordinate_for_namespace(&target).as_deref() != Some(&coordinate) {
            return Err("std.native.Package/load expects a locked namespace".into());
        }
        if catalog.state(&coordinate).as_deref() != Some("ready") {
            return Err(format!(
                "package/not-ready: {coordinate}; call Package/ensure first"
            ));
        }
        let registry = namespace_registry()?;
        require_namespace(&registry, env, &target)?;
        return Ok(Value::Symbol(Symbol::parse(&target)));
    }
    if method == "ensure" {
        if catalog.state(&coordinate).as_deref() == Some("ready") {
            let promise = Promise::new();
            promise.resolve(descriptor);
            return Ok(Value::Promise(promise));
        }
        if let Some(pending) = catalog.pending(&coordinate) {
            return Ok(Value::Promise(pending));
        }
    } else if catalog.state(&coordinate).as_deref() == Some("available") {
        let promise = Promise::new();
        promise.resolve(Value::Vector(PVector::new()));
        return Ok(Value::Promise(promise));
    } else if catalog.pending(&coordinate).is_some() {
        return Err(format!("package/busy: {coordinate}"));
    }
    if method == "unload" {
        if let Some(options) = arguments.get(1) {
            if map_entries(options).is_none() {
                return Err("std.native.Package/unload options must be a map".into());
            }
            if let Some(value) = map_value(options, &Value::Keyword("cascade".into())) {
                if !matches!(value, Value::Bool(_)) {
                    return Err("std.native.Package/unload :cascade must be boolean".into());
                }
            }
        }
    }
    let previous_state = catalog
        .state(&coordinate)
        .unwrap_or_else(|| "available".into());
    catalog.set_state(
        &coordinate,
        if method == "ensure" {
            "ensuring"
        } else {
            "unloading"
        },
    );
    HOST_CALL_HANDLER.with(|active| {
        let Some(handler) = active.borrow().as_ref().cloned() else {
            let promise = Promise::new();
            promise.reject_value(host_error(
                "package/unsupported",
                "Package capability provider is unavailable",
            ));
            catalog.set_state(
                &coordinate,
                if method == "ensure" {
                    "failed"
                } else {
                    &previous_state
                },
            );
            return Ok(Value::Promise(promise));
        };
        let mut provider_arguments = vec![descriptor];
        provider_arguments.extend(arguments.iter().skip(1).cloned());
        let result = handler("package".into(), method.into(), provider_arguments);
        if let Ok(Value::Promise(promise)) = &result {
            let state = catalog.clone();
            let coordinate = coordinate.clone();
            let operation = method.to_owned();
            let rollback = previous_state.clone();
            state.set_pending(&coordinate, Some(promise.clone()));
            promise.on_settle(Rc::new(move |settlement| {
                let next = match (&operation[..], settlement) {
                    ("ensure", PromiseState::Fulfilled(_)) => "ready",
                    ("ensure", _) => "failed",
                    ("unload", PromiseState::Fulfilled(_)) => "available",
                    ("unload", _) => rollback.as_str(),
                    _ => rollback.as_str(),
                };
                state.set_state(&coordinate, next);
                state.set_pending(&coordinate, None);
            }));
        } else if result.is_ok() {
            catalog.set_state(
                &coordinate,
                if method == "ensure" {
                    "ready"
                } else {
                    "available"
                },
            );
        } else {
            catalog.set_state(
                &coordinate,
                if method == "ensure" {
                    "failed"
                } else {
                    &previous_state
                },
            );
        }
        result
    })
}

/// Invokes the active host capability provider with already-evaluated VM
/// values. This is the bytecode boundary for `std.native.Host/call`; the VM
/// remains unaware of timers, sockets, or any other concrete host operation.
pub fn call_host_value(service: Value, target: Value, arguments: Value) -> Result<Value, String> {
    let service = match service {
        Value::String(value) => value,
        _ => return Err("std.native.Host/call service must be a string".into()),
    };
    let target = match target {
        Value::String(value) => value,
        _ => return Err("std.native.Host/call method must be a string".into()),
    };
    let arguments = match arguments {
        Value::Vector(values) => values.iter().cloned().collect(),
        Value::Tuple(values) => values.iter().cloned().collect(),
        _ => return Err("std.native.Host/call arguments must be a vector".into()),
    };
    if !native_capability_granted("host-call") {
        return Ok(native_capability_denied_promise(
            "Host",
            "call",
            "host-call",
        ));
    }
    HOST_CALL_HANDLER.with(|active| {
        let Some(handler) = active.borrow().as_ref().cloned() else {
            let promise = Promise::new();
            promise.reject_value(host_error(
                "host/unavailable",
                "Host capability provider is unavailable",
            ));
            return Ok(Value::Promise(promise));
        };
        handler(service, target, arguments)
    })
}

fn host_error(code: &str, message: &str) -> Value {
    Value::ExceptionInfo(Rc::new(ExceptionInfo {
        message: message.into(),
        data: Box::new(Value::Map(
            vec![
                (
                    Value::Keyword("ex/code".into()),
                    Value::Keyword(code.into()),
                ),
                (
                    Value::Keyword("ex/class".into()),
                    Value::Keyword("ex.class/host".into()),
                ),
            ]
            .into_iter()
            .collect(),
        )),
        cause: None,
        provenance: Rc::new(RefCell::new(Default::default())),
    }))
}
/// Installs the explicit host-call boundary for one evaluation.
pub fn with_host_calls<R>(
    handler: Rc<dyn Fn(String, String, Vec<Value>) -> Result<Value, String>>,
    operation: impl FnOnce() -> R,
) -> R {
    HOST_CALL_HANDLER.with(|active| {
        let previous = active.replace(Some(handler));
        let result = operation();
        active.replace(previous);
        result
    })
}

/// Runs an evaluation with a source provider used to satisfy `require` loads.
pub fn with_namespace_source<R>(
    provider: Rc<dyn Fn(&str) -> Option<NamespaceResource>>,
    action: impl FnOnce() -> R,
) -> R {
    NAMESPACE_SOURCE_PROVIDER.with(|active| {
        let previous = active.borrow_mut().replace(provider);
        let result = action();
        *active.borrow_mut() = previous;
        result
    })
}

/// Installs the direct-native namespace loader for one runtime evaluation.
/// The ordinary source/bytecode loader remains the default; this hook lets a
/// Runtime replace only the execution of a materialized namespace while the
/// shared namespace transaction, dependency tracking, and rollback logic stay
/// in one place.
#[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
pub(crate) fn with_direct_native_namespace_loader<R>(
    loader: Rc<dyn Fn(&str, NamespaceResource, &mut HashMap<String, Value>) -> Result<(), String>>,
    action: impl FnOnce() -> R,
) -> R {
    ACTIVE_DIRECT_NATIVE_NAMESPACE_LOADER.with(|active| {
        let previous = active.borrow_mut().replace(loader);
        let result = action();
        *active.borrow_mut() = previous;
        result
    })
}

/// Marks the duration of generated direct-native code. Native helpers may
/// still call one another during this scope, but any attempt to invoke the
/// tree evaluator through a helper is rejected at the shared boundary.
#[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
pub(crate) fn with_direct_native_execution<R>(action: impl FnOnce() -> R) -> R {
    ACTIVE_DIRECT_NATIVE_EXECUTION.with(|active| {
        let previous = active.replace(true);
        let result = action();
        active.set(previous);
        result
    })
}

/// Temporarily leaves the generated-code execution scope while compiling a
/// nested program. Macro expansion and namespace configuration are
/// compilation-time compatibility seams; they may use the tree evaluator,
/// but the validated program must re-enter the direct guard before execution.
#[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
pub(crate) fn without_direct_native_execution<R>(action: impl FnOnce() -> R) -> R {
    ACTIVE_DIRECT_NATIVE_EXECUTION.with(|active| {
        let previous = active.replace(false);
        let result = action();
        active.set(previous);
        result
    })
}

#[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
fn direct_native_namespace_loader(
) -> Option<Rc<dyn Fn(&str, NamespaceResource, &mut HashMap<String, Value>) -> Result<(), String>>> {
    ACTIVE_DIRECT_NATIVE_NAMESPACE_LOADER.with(|active| active.borrow().clone())
}

#[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
pub(crate) fn direct_native_execution() -> bool {
    ACTIVE_DIRECT_NATIVE_EXECUTION.with(Cell::get)
}

/// Runtime state captured when a VM closure crosses the synchronous machine
/// boundary. The no-op shape keeps ordinary VM and wasm builds free of
/// direct-native dependencies while native callbacks and resumptions can
/// restore providers, namespace selection, protocols, and the evaluator
/// guard on native builds.
#[derive(Clone, Default)]
pub(crate) struct NativeCallbackContext {
    #[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
    scope: Option<crate::direct_native::NativeExecutionScope>,
    #[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
    context: Option<DirectNativeContext>,
}

impl NativeCallbackContext {
    pub(crate) fn capture() -> Self {
        #[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
        {
            let scope = crate::direct_native::capture_execution_scope();
            let context = scope.as_ref().map(|_| DirectNativeContext::capture());
            return Self { scope, context };
        }
        #[cfg(not(all(feature = "direct-native", not(target_arch = "wasm32"))))]
        {
            Self::default()
        }
    }

    pub(crate) fn with<R>(&self, action: impl FnOnce() -> R) -> R {
        #[cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]
        {
            return crate::direct_native::with_captured_context(
                self.scope.as_ref(),
                self.context.as_ref(),
                action,
            );
        }
        #[cfg(not(all(feature = "direct-native", not(target_arch = "wasm32"))))]
        {
            action()
        }
    }
}
