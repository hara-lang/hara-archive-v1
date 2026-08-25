use super::*;
use crate::core::{ExtensionValue, ProtocolRegistry};
use std::cell::RefCell;
use std::collections::HashMap;
use std::time::Duration;

const PROVIDER: &str = "std.native.Work";
const HOST_TYPE: &str = "WorkHost";
const RUN_TYPE: &str = "WorkRun";
const HOST_HANDLE: u64 = 1;

#[derive(Default)]
struct GuestHandles {
    next: u64,
    runs: HashMap<u64, WorkRun>,
    ids: HashMap<String, u64>,
}

thread_local! {
    static HANDLES: RefCell<GuestHandles> = RefCell::new(GuestHandles::default());
}

fn extension(type_name: &str, handle: u64) -> Value {
    Value::Extension(ExtensionValue {
        provider: PROVIDER.into(),
        type_name: type_name.into(),
        handle,
    })
}

pub(crate) fn default_host_value() -> Value {
    extension(HOST_TYPE, HOST_HANDLE)
}

pub(crate) fn values() -> Vec<(&'static str, Value)> {
    vec![
        (
            "default-host",
            crate::core::native_function("std.native.Work/default-host", 0, |_| {
                Ok(default_host_value())
            }),
        ),
        (
            "current-run",
            crate::core::native_function("std.native.Work/current-run", 0, |_| {
                Ok(current_work_context()
                    .map(|context| register_run(context.run))
                    .unwrap_or(Value::Nil))
            }),
        ),
        (
            "cancelled?",
            crate::core::native_function("std.native.Work/cancelled?", 0, |_| {
                current_work_context()
                    .map(|context| Value::Bool(context.cancelled()))
                    .ok_or_else(|| "cancelled? requires an active native work context".into())
            }),
        ),
        (
            "check-cancelled",
            crate::core::native_function("std.native.Work/check-cancelled", 0, |_| {
                current_work_context()
                    .ok_or_else(|| {
                        "check-cancelled requires an active native work context".to_string()
                    })?
                    .check_cancelled()
                    .map(|_| Value::Nil)
                    .map_err(|error| error.message())
            }),
        ),
        (
            "deadline-nanos",
            crate::core::native_function("std.native.Work/deadline-nanos", 0, |_| {
                let context = current_work_context().ok_or_else(|| {
                    "deadline-nanos requires an active native work context".to_string()
                })?;
                Ok(context
                    .deadline_nanos()
                    .map(|value| Value::Number(value as i64))
                    .unwrap_or(Value::Nil))
            }),
        ),
        (
            "emit",
            crate::core::native_function("std.native.Work/emit", 2, |arguments| {
                let context = current_work_context()
                    .ok_or_else(|| "emit requires an active native work context".to_string())?;
                Ok(Value::Bool(
                    context.emit(arguments[0].clone(), arguments[1].clone()),
                ))
            }),
        ),
        (
            "submit-child",
            crate::core::native_fixed_variadic_function(
                "std.native.Work/submit-child",
                2,
                |arguments| {
                    if !(2..=3).contains(&arguments.len()) {
                        return Err("submit-child expects 2 or 3 arguments".into());
                    }
                    let context = current_work_context().ok_or_else(|| {
                        "submit-child requires an active native work context".to_string()
                    })?;
                    let work = arguments[0].clone();
                    let input = arguments[1].clone();
                    let options = arguments
                        .get(2)
                        .cloned()
                        .unwrap_or_else(|| Value::Map([].into_iter().collect()));
                    let executor = option(&options, "work/execute").unwrap_or_else(|| work.clone());
                    let Value::Function(function) = executor else {
                        return Err(
                            "submit-child requires callable work or a :work/execute adapter".into(),
                        );
                    };
                    let run =
                        context.submit_child(work_options(&options)?, move |child_context| {
                            crate::core::invoke_function_sync(
                                function,
                                vec![
                                    work,
                                    input,
                                    options,
                                    Value::String(child_context.work_id().to_string()),
                                ],
                            )
                        })?;
                    Ok(register_run(run))
                },
            ),
        ),
        (
            "on-close",
            crate::core::native_function("std.native.Work/on-close", 1, |arguments| {
                let context = current_work_context()
                    .ok_or_else(|| "on-close requires an active native work context".to_string())?;
                let Value::Function(function) = arguments[0].clone() else {
                    return Err("on-close requires a callable finalizer".into());
                };
                Ok(Value::Bool(context.on_close(move |context| {
                    crate::core::invoke_function_sync(function, vec![register_run(context.run)])
                        .map(|_| ())
                        .map_err(work_failure)
                })))
            }),
        ),
    ]
}

fn host(arguments: &[Value], arity: usize) -> Result<WorkHost, String> {
    if arguments.len() != arity {
        return Err(format!(
            "native WorkHost operation expects {arity} arguments"
        ));
    }
    if arguments.first().is_some_and(is_work_host) {
        Ok(process_work_host())
    } else {
        Err("native WorkHost operation requires a native work host".into())
    }
}

fn is_work_host(value: &Value) -> bool {
    matches!(
        value,
        Value::Extension(value)
            if value.provider == PROVIDER
                && value.type_name == HOST_TYPE
                && value.handle == HOST_HANDLE
    )
}

fn is_work_run(value: &Value) -> bool {
    matches!(
        value,
        Value::Extension(value) if value.provider == PROVIDER && value.type_name == RUN_TYPE
    )
}

fn run(arguments: &[Value], arity: usize) -> Result<WorkRun, String> {
    if arguments.len() != arity {
        return Err(format!(
            "native WorkRun operation expects {arity} arguments"
        ));
    }
    let handle = match arguments.first() {
        Some(Value::Extension(value)) if is_work_run(&arguments[0]) => value.handle,
        _ => return Err("native WorkRun operation requires a native work run".into()),
    };
    HANDLES.with(|handles| {
        handles
            .borrow()
            .runs
            .get(&handle)
            .cloned()
            .ok_or_else(|| "native work run handle is no longer available".into())
    })
}

fn register_run(run: WorkRun) -> Value {
    HANDLES.with(|handles| {
        let mut handles = handles.borrow_mut();
        if let Some(handle) = handles.ids.get(run.work_id().as_str()).copied() {
            handles.runs.insert(handle, run);
            return extension(RUN_TYPE, handle);
        }
        handles.next += 1;
        let handle = handles.next;
        handles.ids.insert(run.work_id().to_string(), handle);
        handles.runs.insert(handle, run);
        extension(RUN_TYPE, handle)
    })
}

fn option(options: &Value, name: &str) -> Option<Value> {
    let key = Value::Keyword(name.into());
    match options {
        Value::Map(values) => values.get(&key).cloned(),
        Value::OrderedMap(values) => values.get(&key).cloned(),
        Value::SortedMap(values) => values.get(&key).cloned(),
        _ => None,
    }
}

fn id_text(value: Value) -> Result<String, String> {
    match value {
        Value::String(value) => Ok(value),
        Value::Keyword(value) => Ok(value.to_string()),
        Value::Symbol(value) => Ok(value.to_string()),
        _ => Err("work run ID must be a string, keyword, or symbol".into()),
    }
}

fn reference_id(value: &Value) -> Result<String, String> {
    match value {
        Value::String(value) => Ok(value.clone()),
        Value::Keyword(value) => Ok(value.to_string()),
        Value::Symbol(value) => Ok(value.to_string()),
        Value::Extension(_) => Ok(run(std::slice::from_ref(value), 1)?.work_id().to_string()),
        Value::Struct(value) => value
            .get("id")
            .cloned()
            .ok_or_else(|| "work reference does not contain :id".to_string())
            .and_then(id_text),
        _ => Err("work-resolve requires a work reference or run ID".into()),
    }
}

fn work_options(options: &Value) -> Result<WorkOptions, String> {
    let id = option(options, "id")
        .or_else(|| option(options, "run/id"))
        .or_else(|| option(options, "work/id"))
        .map(id_text)
        .transpose()?
        .map(WorkId::new)
        .transpose()?;
    let timeout = option(options, "timeout-ms")
        .map(|value| match value {
            Value::Number(value) if value >= 0 => Ok(Duration::from_millis(value as u64)),
            _ => Err(":timeout-ms must be a non-negative integer".to_string()),
        })
        .transpose()?;
    let deadline = option(options, "deadline-nanos")
        .map(|value| match value {
            Value::Number(value) if value > 0 => {
                let remaining = (value as u64).saturating_sub(monotonic_nanos());
                Ok(std::time::Instant::now()
                    .checked_add(Duration::from_nanos(remaining))
                    .unwrap_or_else(std::time::Instant::now))
            }
            _ => Err(":deadline-nanos must be a positive integer".to_string()),
        })
        .transpose()?;
    let detached = matches!(option(options, "detached"), Some(Value::Bool(true)));
    Ok(WorkOptions {
        id,
        timeout,
        deadline,
        detached,
    })
}

fn work_submit(arguments: &[Value]) -> Result<Value, String> {
    let host = host(arguments, 4)?;
    let work = arguments[1].clone();
    let input = arguments[2].clone();
    let options = arguments[3].clone();
    let executor = option(&options, "work/execute").unwrap_or_else(|| work.clone());
    let function = match executor {
        Value::Function(function) => function,
        _ => return Err("work-submit requires callable work or a :work/execute adapter".into()),
    };
    let run = host.submit_scoped(work_options(&options)?, move |context| {
        crate::core::invoke_function_sync(
            function,
            vec![
                work,
                input,
                options,
                Value::String(context.work_id().to_string()),
            ],
        )
    })?;
    Ok(register_run(run))
}

fn work_resolve(arguments: &[Value]) -> Result<Value, String> {
    let host = host(arguments, 2)?;
    let id = reference_id(&arguments[1])?;
    Ok(register_run(host.resolve(&id)?))
}

fn work_id(arguments: &[Value]) -> Result<Value, String> {
    Ok(Value::String(run(arguments, 1)?.work_id().to_string()))
}

fn work_status(arguments: &[Value]) -> Result<Value, String> {
    let state = run(arguments, 1)?.work_status().state;
    let keyword = match state {
        WorkRunState::Queued => "queued",
        WorkRunState::Running => "running",
        WorkRunState::Waiting => "waiting",
        WorkRunState::Cancelling => "cancelling",
        WorkRunState::Completed => "completed",
        WorkRunState::Failed => "failed",
        WorkRunState::Cancelled => "cancelled",
    };
    Ok(Value::Keyword(keyword.into()))
}

fn work_result(arguments: &[Value]) -> Result<Value, String> {
    Ok(Value::Promise(run(arguments, 1)?.work_result()))
}

fn work_events(arguments: &[Value]) -> Result<Value, String> {
    let run = run(arguments, 2)?;
    let after = option(&arguments[1], "after")
        .map(|value| match value {
            Value::Number(value) if value >= 0 => Ok(value as usize),
            _ => Err(":after must be a non-negative integer".to_string()),
        })
        .transpose()?
        .unwrap_or(0);
    Ok(run.work_events(after))
}

fn work_cancel(arguments: &[Value]) -> Result<Value, String> {
    Ok(Value::Promise(
        run(arguments, 2)?.work_cancel(arguments[1].clone()),
    ))
}

fn closed(arguments: &[Value]) -> Result<Value, String> {
    Ok(Value::Bool(run(arguments, 1)?.closed()))
}

pub(crate) fn install(protocols: &mut ProtocolRegistry) {
    protocols.register_when("IWorkHost", "work-submit", is_work_host, work_submit);
    protocols.register_when("IWorkHost", "work-resolve", is_work_host, work_resolve);
    protocols.register_when("IWorkRef", "work-id", is_work_run, work_id);
    protocols.register_when("IWorkRun", "work-status", is_work_run, work_status);
    protocols.register_when("IWorkRun", "work-result", is_work_run, work_result);
    protocols.register_when("IWorkRun", "work-events", is_work_run, work_events);
    protocols.register_when("IWorkRun", "work-cancel", is_work_run, work_cancel);
    protocols.register_when("IClosed", "closed?", is_work_run, closed);

    protocols.register_when("IComponent", "props", is_work_host, |_| {
        Ok(Value::Map(
            [(
                Value::Keyword("work/host".into()),
                Value::Keyword("ephemeral".into()),
            )]
            .into_iter()
            .collect(),
        ))
    });
    protocols.register_when("IComponent", "status", is_work_host, |arguments| {
        let status = host(arguments, 1)?.status();
        Ok(Value::Keyword(status.state.into()))
    });
    protocols.register_when("IComponent", "started?", is_work_host, |arguments| {
        Ok(Value::Bool(host(arguments, 1)?.started()))
    });
    protocols.register_when("IComponent", "stopped?", is_work_host, |arguments| {
        Ok(Value::Bool(!host(arguments, 1)?.started()))
    });
    protocols.register_when("IComponent", "start", is_work_host, |arguments| {
        let host = host(arguments, 1)?;
        host.start();
        Ok(default_host_value())
    });
    protocols.register_when("IComponent", "stop", is_work_host, |arguments| {
        let host = host(arguments, 1)?;
        host.stop();
        Ok(default_host_value())
    });
    protocols.register_when("IComponent", "kill", is_work_host, |arguments| {
        let host = host(arguments, 1)?;
        host.kill();
        Ok(default_host_value())
    });
    protocols.register_when("IComponent", "remote?", is_work_host, |arguments| {
        host(arguments, 1)?;
        Ok(Value::Bool(false))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guest_handles_are_opaque_and_stable_per_run() {
        let host = process_work_host();
        let run = host
            .submit(Some("guest-stable"), || Ok(Value::Number(42)))
            .unwrap();
        let first = register_run(run.clone());
        let second = register_run(run);
        assert_eq!(first, second);
        assert_eq!(reference_id(&first).unwrap(), "guest-stable");
    }

    #[test]
    fn native_run_events_are_ordered_and_close_after_terminal() {
        let host = WorkHost::new();
        let run = host
            .submit(Some("guest-events"), || Ok(Value::Number(42)))
            .unwrap();
        let stream = run.work_events(0);
        host.drain();
        let values = (0..3)
            .map(|_| {
                crate::core::stream_next_value(&stream)
                    .unwrap()
                    .wait_state()
            })
            .map(|state| match state {
                PromiseState::Fulfilled(value) => value,
                state => panic!("event pull did not fulfil: {state:?}"),
            })
            .collect::<Vec<_>>();
        assert_eq!(
            values
                .iter()
                .map(|value| option(value, "event/type").unwrap())
                .collect::<Vec<_>>(),
            vec![
                Value::Keyword("work/run-queued".into()),
                Value::Keyword("work/run-running".into()),
                Value::Keyword("work/run-completed".into()),
            ]
        );
        assert_eq!(
            crate::core::stream_next_value(&stream)
                .unwrap()
                .wait_state(),
            PromiseState::Fulfilled(Value::Nil)
        );
    }
}
