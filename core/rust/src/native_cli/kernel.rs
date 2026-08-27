use super::*;

pub(super) fn kernel_call(
    broker: &RuntimeBroker,
    operation: &str,
    arguments: &[Value],
) -> Result<Value, String> {
    match operation {
        "sandbox-open" => {
            let promise = Promise::new();
            match sandbox_spec_argument(arguments, operation) {
                Ok(spec) => match broker.sandbox_open(spec) {
                    Ok(id) => {
                        promise.resolve(Value::Number(id.get() as i64));
                    }
                    Err(error) => {
                        promise.reject_value(sandbox_error_value(&error));
                    }
                },
                Err(error) => {
                    promise.reject_value(sandbox_error_value(&format!(
                        "sandbox/invalid-spec: {error}"
                    )));
                }
            }
            Ok(Value::Promise(promise))
        }
        "sandbox-eval" => {
            let sandbox = sandbox_id_argument(arguments, 0, operation)?;
            let source = string_argument(arguments, 1, operation)?;
            match broker.sandbox_eval_receiver(sandbox, source) {
                Ok((evaluation, receiver)) => Ok(Value::Promise(sandbox_string_promise(
                    broker.clone(),
                    sandbox,
                    evaluation,
                    receiver,
                ))),
                Err(error) => Ok(Value::Promise(sandbox_rejected_promise(&error))),
            }
        }
        "sandbox-call" => {
            let sandbox = sandbox_id_argument(arguments, 0, operation)?;
            let callable = match arguments.get(1) {
                Some(Value::Symbol(callable)) if callable.get_namespace().is_some() => {
                    callable.as_str()
                }
                _ => return Err(format!("{operation}: callable must be a qualified symbol")),
            };
            let supplied = arguments
                .get(2)
                .ok_or_else(|| format!("{operation}: missing argument vector"))?;
            let normalized = match supplied {
                Value::Vector(_) => supplied.clone(),
                Value::Tuple(values) => Value::Vector(values.iter().cloned().collect()),
                _ => return Err(format!("{operation}: expected an argument vector")),
            };
            if !crate::core::session_transferable(&normalized) {
                return Err(format!(
                    "{operation}: arguments must be immutable portable values"
                ));
            }
            let encoded = crate::hta::encode(&normalized)?;
            match broker.sandbox_call_receiver(sandbox, callable, &encoded) {
                Ok((evaluation, receiver)) => Ok(Value::Promise(sandbox_hta_promise(
                    broker.clone(),
                    sandbox,
                    evaluation,
                    receiver,
                ))),
                Err(error) => Ok(Value::Promise(sandbox_rejected_promise(&error))),
            }
        }
        "sandbox-cancel" => {
            let promise = Promise::new();
            match broker.sandbox_cancel(sandbox_id_argument(arguments, 0, operation)?) {
                Ok(cancelled) => {
                    promise.resolve(Value::Bool(cancelled));
                }
                Err(error) => {
                    promise.reject_value(sandbox_error_value(&error));
                }
            }
            Ok(Value::Promise(promise))
        }
        "sandbox-status" => broker
            .sandbox_status(sandbox_id_argument(arguments, 0, operation)?)
            .map(sandbox_status_value),
        "sandbox-close" => {
            let promise = Promise::new();
            match broker.sandbox_close(sandbox_id_argument(arguments, 0, operation)?) {
                Ok(()) => {
                    promise.resolve(Value::Nil);
                }
                Err(error) => {
                    promise.reject_value(sandbox_error_value(&error));
                }
            }
            Ok(Value::Promise(promise))
        }
        "package-check" => {
            let (identity, version) = crate::package::check_path(std::path::Path::new(
                string_argument(arguments, 0, operation)?,
            ))?;
            Ok(Value::Map(
                [
                    (keyword("identity"), Value::String(identity)),
                    (keyword("version"), Value::String(version)),
                ]
                .into_iter()
                .collect(),
            ))
        }
        "package-build" => {
            let input = string_argument(arguments, 0, operation)?.to_owned();
            let output = optional_string_argument(arguments, 1, operation)?.map(str::to_owned);
            let package = optional_string_argument(arguments, 2, operation)?.map(str::to_owned);
            let profile = optional_string_argument(arguments, 3, operation)?.map(str::to_owned);
            let built = std::thread::spawn(move || {
                crate::package::build_path_with_package(
                    std::path::Path::new(&input),
                    output.as_deref().map(std::path::Path::new),
                    package.as_deref(),
                    profile.as_deref().map(std::path::Path::new),
                )
            })
            .join()
            .map_err(|_| format!("{operation}: package build thread panicked"))??;
            Ok(Value::String(built.to_string_lossy().into_owned()))
        }
        "package-inspect" => Ok(Value::String(crate::package::inspect_path(
            std::path::Path::new(string_argument(arguments, 0, operation)?),
        )?)),
        "package-install" => Ok(Value::String(
            crate::package::install_path(std::path::Path::new(string_argument(
                arguments, 0, operation,
            )?))?
            .to_string_lossy()
            .into_owned(),
        )),
        "package-publish" => Ok(Value::String(crate::package::publish_path(
            std::path::Path::new(string_argument(arguments, 0, operation)?),
            string_argument(arguments, 1, operation)?,
            boolean_argument(arguments, 2, operation)?,
        )?)),
        "package-registry-verify" => {
            let request = std::path::Path::new(string_argument(arguments, 0, operation)?);
            let identity = std::path::Path::new(string_argument(arguments, 1, operation)?);
            crate::package::verify_registry_request_paths(request, identity)?;
            Ok(Value::String(format!(
                "registry request verified: {}",
                request.display()
            )))
        }
        "tap-config-root" => Ok(Value::String(
            crate::tap::config_root().to_string_lossy().into_owned(),
        )),
        "tap-add" => {
            let root = std::path::Path::new(string_argument(arguments, 0, operation)?);
            let name = string_argument(arguments, 1, operation)?;
            let tap = crate::tap::Tap {
                name: name.into(),
                registry: strings_argument(arguments, 2, operation)?,
                identity: strings_argument(arguments, 3, operation)?,
                identity_key: string_argument(arguments, 4, operation)?.into(),
                trust: crate::tap::TrustMode::SignedRoot,
            };
            crate::tap::add(root, tap.clone())?;
            Ok(tap_value(&tap))
        }
        "tap-bootstrap" => Ok(tap_value(&crate::tap::bootstrap(
            std::path::Path::new(string_argument(arguments, 0, operation)?),
            string_argument(arguments, 1, operation)?,
        )?)),
        "tap-remove" => {
            crate::tap::remove(
                std::path::Path::new(string_argument(arguments, 0, operation)?),
                string_argument(arguments, 1, operation)?,
            )?;
            Ok(Value::Nil)
        }
        "tap-list" => Ok(Value::Vector(
            crate::tap::load(std::path::Path::new(string_argument(
                arguments, 0, operation,
            )?))?
            .values()
            .map(tap_value)
            .collect(),
        )),
        "tap-mirror-add" => Ok(tap_value(&crate::tap::add_mirror(
            std::path::Path::new(string_argument(arguments, 0, operation)?),
            string_argument(arguments, 1, operation)?,
            optional_string_argument(arguments, 2, operation)?.map(str::to_owned),
            optional_string_argument(arguments, 3, operation)?.map(str::to_owned),
        )?)),
        "tap-initialize" => {
            let initialized = crate::tap::initialize(
                string_argument(arguments, 1, operation)?,
                std::path::Path::new(string_argument(arguments, 2, operation)?),
                std::path::Path::new(string_argument(arguments, 3, operation)?),
                string_argument(arguments, 4, operation)?,
            )?;
            crate::tap::add(
                std::path::Path::new(string_argument(arguments, 0, operation)?),
                initialized.tap.clone(),
            )?;
            Ok(Value::Map(
                [
                    (keyword("tap"), tap_value(&initialized.tap)),
                    (
                        keyword("fingerprint"),
                        Value::String(initialized.fingerprint),
                    ),
                ]
                .into_iter()
                .collect(),
            ))
        }
        "tap-verify" => {
            let name = string_argument(arguments, 1, operation)?;
            let policy = crate::tap::verify_trusted(
                std::path::Path::new(string_argument(arguments, 0, operation)?),
                name,
            )?;
            Ok(Value::Map(
                [
                    (keyword("name"), Value::String(name.into())),
                    (keyword("revision"), Value::String(policy.revision)),
                ]
                .into_iter()
                .collect(),
            ))
        }
        "snapshot-build" => Ok(Value::String(crate::snapshot_tool::build_paths(
            std::path::Path::new(string_argument(arguments, 0, operation)?),
            std::path::Path::new(string_argument(arguments, 1, operation)?),
        )?)),
        "snapshot-verify" => Ok(Value::String(crate::snapshot_tool::verify_paths(
            std::path::Path::new(string_argument(arguments, 0, operation)?),
            optional_string_argument(arguments, 1, operation)?.map(std::path::Path::new),
        )?)),
        "snapshot-inspect" => Ok(Value::String(crate::snapshot_tool::inspect_path(
            std::path::Path::new(string_argument(arguments, 0, operation)?),
        )?)),
        "snapshot-diff" => Ok(Value::String(crate::snapshot_tool::diff_paths(
            std::path::Path::new(string_argument(arguments, 0, operation)?),
            std::path::Path::new(string_argument(arguments, 1, operation)?),
        )?)),
        "session-create" => {
            broker.create(string_argument(arguments, 0, operation)?)?;
            Ok(Value::Nil)
        }
        "session-close" => {
            broker.close(string_argument(arguments, 0, operation)?)?;
            Ok(Value::Nil)
        }
        "session-list" => Ok(strings_value(broker.list()?)),
        "session-info" => {
            let name = string_argument(arguments, 0, operation)?;
            let info = broker.info(name)?;
            let namespace = info
                .split_once(' ')
                .map(|(_, namespace)| namespace)
                .unwrap_or("user");
            Ok(Value::Map(
                [
                    (keyword("name"), Value::String(name.into())),
                    (
                        keyword("namespace"),
                        Value::Symbol(Symbol::parse(namespace)),
                    ),
                    (keyword("state"), keyword("idle")),
                    (keyword("filesystem"), Value::Nil),
                ]
                .into_iter()
                .collect(),
            ))
        }
        "session-eval" => {
            let output = broker.eval(
                string_argument(arguments, 0, operation)?,
                string_argument(arguments, 1, operation)?,
            )?;
            let form = crate::kernel::parse(&output)?;
            crate::core::form_to_value(&form)
        }
        "session-namespace" => Ok(Value::Symbol(Symbol::parse(
            &broker.namespace(string_argument(arguments, 0, operation)?)?,
        ))),
        "session-complete" => Ok(strings_value(broker.complete(
            string_argument(arguments, 0, operation)?,
            string_argument(arguments, 1, operation)?,
        )?)),
        "resource-register" => {
            broker.register_resource(
                string_argument(arguments, 0, operation)?,
                string_argument(arguments, 1, operation)?,
            )?;
            Ok(Value::Nil)
        }
        "resource-remove" => {
            broker.remove_resource(string_argument(arguments, 0, operation)?)?;
            Ok(Value::Nil)
        }
        "resource-list" => Ok(strings_value(broker.resources()?)),
        "capabilities" => Ok(Value::Map(
            [
                (keyword("sessions"), Value::Bool(true)),
                (keyword("resources"), Value::Bool(true)),
                (keyword("filesystems"), Value::Bool(false)),
            ]
            .into_iter()
            .collect(),
        )),
        operation if operation.starts_with("filesystem-") => {
            Err(format!("{operation} is unavailable in the runtime broker"))
        }
        _ => Err(format!("unknown foundation.kernel operation: {operation}")),
    }
}

fn sandbox_id_argument(
    arguments: &[Value],
    index: usize,
    operation: &str,
) -> Result<SandboxId, String> {
    match arguments.get(index) {
        Some(Value::Number(value)) if *value > 0 => {
            SandboxId::parse(*value as u64).map_err(|error| error.to_string())
        }
        _ => Err(format!(
            "{operation}: sandbox id must be a positive integer"
        )),
    }
}

fn sandbox_spec_argument(arguments: &[Value], operation: &str) -> Result<SandboxSpec, String> {
    let entries = sandbox_map_entries(
        arguments
            .first()
            .ok_or_else(|| format!("{operation}: missing SandboxSpec"))?,
        operation,
        "SandboxSpec",
    )?;
    let allowed = [
        "protocol",
        "provider",
        "runtime",
        "entry-namespace",
        "bundles",
        "mount",
        "provider-options",
        "limits",
    ];
    for (key, _) in &entries {
        let Value::Keyword(key) = key else {
            return Err(format!("{operation}: SandboxSpec keys must be keywords"));
        };
        if !allowed.contains(&key.as_str()) {
            return Err(format!(
                "{operation}: unknown SandboxSpec key :{}",
                key.as_str()
            ));
        }
    }
    if entries.len() != allowed.len() {
        return Err(format!(
            "{operation}: SandboxSpec requires exactly eight keys"
        ));
    }
    let lookup = |name: &str| {
        entries
            .iter()
            .find(|(key, _)| matches!(key, Value::Keyword(value) if value.as_str() == name))
            .map(|(_, value)| *value)
    };
    let text = |name: &str, fallback: &str| -> Result<String, String> {
        match lookup(name) {
            None | Some(Value::Nil) => Ok(fallback.into()),
            Some(Value::String(value)) => Ok(value.clone()),
            Some(Value::Keyword(value)) => Ok(value.as_str().into()),
            Some(Value::Symbol(value)) => Ok(value.as_str().into()),
            _ => Err(format!("{operation}: :{name} must be text-like")),
        }
    };
    let bundles = match lookup("bundles") {
        Some(Value::Vector(values)) => values.iter().collect::<Vec<_>>(),
        Some(Value::Tuple(values)) => values.iter().collect::<Vec<_>>(),
        _ => return Err(format!("{operation}: :bundles must be a vector")),
    }
    .into_iter()
    .map(|value| sandbox_bundle_reference(value, operation))
    .collect::<Result<Vec<_>, _>>()?;
    let mount = match lookup("mount") {
        None | Some(Value::Nil) => None,
        Some(Value::Number(value)) if *value > 0 => Some(crate::SessionMountId::new(*value as u64)),
        _ => {
            return Err(format!(
                "{operation}: :mount must be an opaque positive mount id or nil"
            ))
        }
    };
    let provider_options_hta = match lookup("provider-options") {
        Some(value @ (Value::Map(_) | Value::OrderedMap(_)))
            if crate::core::session_transferable(value) =>
        {
            crate::hta::encode(value)?
        }
        _ => {
            return Err(format!(
                "{operation}: :provider-options must be an immutable portable map"
            ))
        }
    };
    let limits = sandbox_limits(lookup("limits"), operation)?;
    let entry_namespace: String = match lookup("entry-namespace") {
        Some(Value::Symbol(value)) if value.get_namespace().is_none() => value.as_str().into(),
        _ => {
            return Err(format!(
                "{operation}: :entry-namespace must be an unqualified symbol"
            ))
        }
    };
    SandboxSpec::with_inputs(
        match lookup("protocol") {
            Some(Value::String(value)) => value.clone(),
            _ => return Err(format!("{operation}: :protocol must be a string")),
        },
        text("provider", "")?,
        text("runtime", "")?,
        entry_namespace,
        bundles,
        mount,
        provider_options_hta,
        limits,
    )
    .map_err(|error| error.to_string())
}

fn sandbox_map_entries<'a>(
    value: &'a Value,
    operation: &str,
    label: &str,
) -> Result<Vec<(&'a Value, &'a Value)>, String> {
    match value {
        Value::Map(entries) => Ok(entries.iter().collect()),
        Value::OrderedMap(entries) => Ok(entries.iter().map(|(key, value)| (key, value)).collect()),
        _ => Err(format!("{operation}: {label} must be a map")),
    }
}

fn sandbox_bundle_reference(
    value: &Value,
    operation: &str,
) -> Result<crate::SandboxBundleReference, String> {
    let entries = sandbox_map_entries(value, operation, "bundle reference")?;
    if entries.len() != 2 {
        return Err(format!(
            "{operation}: bundle references require exactly :digest and :format"
        ));
    }
    let find = |name: &str| {
        entries
            .iter()
            .find(|(key, _)| matches!(key, Value::Keyword(key) if key.as_str() == name))
            .map(|(_, value)| *value)
    };
    let text = |name: &str| match find(name) {
        Some(Value::String(value)) => Ok(value.clone()),
        Some(Value::Keyword(value)) => Ok(value.as_str().into()),
        _ => Err(format!("{operation}: bundle :{name} must be text-like")),
    };
    crate::SandboxBundleReference::new(text("digest")?, text("format")?)
        .map_err(|error| error.to_string())
}

fn sandbox_limits(value: Option<&Value>, operation: &str) -> Result<crate::SandboxLimits, String> {
    let value = value.ok_or_else(|| format!("{operation}: :limits must be a map"))?;
    let entries = sandbox_map_entries(value, operation, "limits")?;
    let allowed = [
        "source-bytes",
        "result-bytes",
        "output-bytes",
        "evaluation-ms",
        "memory-bytes",
        "active-evaluations",
    ];
    for (key, _) in &entries {
        let Value::Keyword(key) = key else {
            return Err(format!("{operation}: limit keys must be keywords"));
        };
        if !allowed.contains(&key.as_str()) {
            return Err(format!(
                "{operation}: unknown sandbox limit :{}",
                key.as_str()
            ));
        }
    }
    if entries.len() != allowed.len() {
        return Err(format!("{operation}: limits require exactly six keys"));
    }
    let defaults = crate::SandboxLimits::default();
    let positive = |name: &str, fallback: u64| -> Result<u64, String> {
        match entries
            .iter()
            .find(|(key, _)| matches!(key, Value::Keyword(key) if key.as_str() == name))
            .map(|(_, value)| *value)
        {
            None => Ok(fallback),
            Some(Value::Number(value)) if *value > 0 => Ok(*value as u64),
            _ => Err(format!("{operation}: :{name} must be a positive integer")),
        }
    };
    Ok(crate::SandboxLimits {
        source_bytes: usize::try_from(positive("source-bytes", defaults.source_bytes as u64)?)
            .map_err(|_| format!("{operation}: :source-bytes is too large"))?,
        result_bytes: usize::try_from(positive("result-bytes", defaults.result_bytes as u64)?)
            .map_err(|_| format!("{operation}: :result-bytes is too large"))?,
        output_bytes: usize::try_from(positive("output-bytes", defaults.output_bytes as u64)?)
            .map_err(|_| format!("{operation}: :output-bytes is too large"))?,
        evaluation_ms: positive("evaluation-ms", defaults.evaluation_ms)?,
        memory_bytes: usize::try_from(positive("memory-bytes", defaults.memory_bytes as u64)?)
            .map_err(|_| format!("{operation}: :memory-bytes is too large"))?,
        active_evaluations: usize::try_from(positive(
            "active-evaluations",
            defaults.active_evaluations as u64,
        )?)
        .map_err(|_| format!("{operation}: :active-evaluations is too large"))?,
    })
}

fn sandbox_string_promise(
    broker: RuntimeBroker,
    sandbox: SandboxId,
    evaluation: EvaluationId,
    receiver: mpsc::Receiver<Result<String, String>>,
) -> Promise {
    let promise = Promise::new();
    let waiting = Rc::new(RefCell::new(Some(receiver)));
    let settled = promise.clone();
    promise.set_waiter(Rc::new(move || {
        let Some(receiver) = waiting.borrow_mut().take() else {
            return;
        };
        match receiver.recv() {
            Ok(Ok(value)) => {
                let form =
                    crate::kernel::parse(&value).and_then(|form| crate::core::form_to_value(&form));
                match form {
                    Ok(value) => {
                        settled.resolve(value);
                    }
                    Err(error) => {
                        settled.reject(error);
                    }
                }
            }
            Ok(Err(error)) => {
                settled.reject_value(sandbox_error_value(&error));
            }
            Err(_) => {
                settled.reject("sandbox provider dropped the evaluation result");
            }
        }
    }));
    promise.set_cancel_hook(Rc::new(move || {
        let _ = broker.sandbox_cancel_evaluation(sandbox, evaluation);
    }));
    promise
}

fn sandbox_hta_promise(
    broker: RuntimeBroker,
    sandbox: SandboxId,
    evaluation: EvaluationId,
    receiver: mpsc::Receiver<Result<Vec<u8>, String>>,
) -> Promise {
    let promise = Promise::new();
    let waiting = Rc::new(RefCell::new(Some(receiver)));
    let settled = promise.clone();
    promise.set_waiter(Rc::new(move || {
        let Some(receiver) = waiting.borrow_mut().take() else {
            return;
        };
        match receiver.recv() {
            Ok(Ok(value)) => match crate::hta::decode(&value) {
                Ok(value) => {
                    settled.resolve(value);
                }
                Err(error) => {
                    settled.reject(error);
                }
            },
            Ok(Err(error)) => {
                settled.reject_value(sandbox_error_value(&error));
            }
            Err(_) => {
                settled.reject("sandbox provider dropped the call result");
            }
        }
    }));
    promise.set_cancel_hook(Rc::new(move || {
        let _ = broker.sandbox_cancel_evaluation(sandbox, evaluation);
    }));
    promise
}

fn sandbox_status_value(status: SandboxStatus) -> Value {
    let error = status.error.map_or(Value::Nil, |error| {
        Value::Map(
            [
                (keyword("code"), keyword(error.code.as_str())),
                (keyword("message"), Value::String(error.message)),
            ]
            .into_iter()
            .collect(),
        )
    });
    Value::Map(
        [
            (keyword("sandbox/id"), Value::Number(status.id.get() as i64)),
            (keyword("sandbox/provider"), Value::String(status.provider)),
            (keyword("sandbox/state"), keyword(status.state.as_str())),
            (keyword("sandbox/secure"), Value::Bool(status.secure)),
            (
                keyword("sandbox/evaluation-active"),
                Value::Bool(status.evaluation_active),
            ),
            (keyword("sandbox/error"), error),
        ]
        .into_iter()
        .collect(),
    )
}

fn sandbox_error_value(error: &str) -> Value {
    let (code, message) = error
        .split_once(": ")
        .filter(|(code, _)| code.starts_with("sandbox/"))
        .unwrap_or(("sandbox/provider-failed", error));
    Value::ExceptionInfo(Rc::new(ExceptionInfo {
        message: message.into(),
        data: Box::new(Value::Map(
            [
                (keyword("ex/code"), keyword(code)),
                (keyword("ex/class"), keyword(sandbox_error_class(code))),
            ]
            .into_iter()
            .collect(),
        )),
        cause: None,
        provenance: Rc::new(RefCell::new(Default::default())),
    }))
}

fn sandbox_error_class(code: &str) -> &'static str {
    match code {
        "sandbox/invalid-spec" => "ex.class/argument",
        "sandbox/provider-not-found"
        | "sandbox/bundle-not-found"
        | "sandbox/mount-not-found"
        | "sandbox/not-found" => "ex.class/not-found",
        "sandbox/provider-unavailable" => "ex.class/dependency",
        "sandbox/bundle-digest-mismatch" | "sandbox/result-not-transferable" => {
            "ex.class/serialization"
        }
        "sandbox/busy" => "ex.class/conflict",
        "sandbox/cancelled" | "sandbox/evaluation-failed" => "ex.class/state",
        "sandbox/timeout" => "ex.class/timeout",
        "sandbox/limit-exceeded" => "ex.class/limit",
        "sandbox/transport-failed" => "ex.class/io",
        _ => "ex.class/host",
    }
}

fn sandbox_rejected_promise(error: &str) -> Promise {
    let promise = Promise::new();
    promise.reject_value(sandbox_error_value(error));
    promise
}
