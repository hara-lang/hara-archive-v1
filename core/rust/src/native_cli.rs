#![cfg(not(target_arch = "wasm32"))]

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{mpsc, Arc};

use crate::core::{ExceptionInfo, Promise, Value};
use crate::invoke_hta::InvokeHtaError;
use crate::lang::data::Symbol;
use crate::lang::protocol::INamespaced;
use crate::{
    EvaluationId, InProcessSandboxProvider, Runtime, SandboxId, SandboxSpec, SandboxStatus,
    SessionId, SessionKernel,
};

mod arguments;
mod documentation;
mod kernel;
use arguments::{
    boolean as boolean_argument, keyword, optional_string as optional_string_argument,
    string as string_argument, strings as strings_argument, strings_value, tap_value,
};
pub use documentation::{Documentation, DocumentationValue};
use kernel::kernel_call;

// Optimized brokers stay within the production 8 MiB ceiling. Debug evaluator
// frames are much larger and need the same development allowance as the CLI
// and portable test runner while loading the full language library.
const RUNTIME_BROKER_STACK_SIZE: usize = if cfg!(debug_assertions) {
    64 * 1024 * 1024
} else {
    8 * 1024 * 1024
};

#[derive(Clone, Copy)]
enum RuntimeBootstrap {
    Full,
    Core,
    Source,
}

enum Request {
    Eval {
        session: String,
        source: String,
        reply: mpsc::Sender<Result<String, String>>,
    },
    Namespace {
        session: String,
        reply: mpsc::Sender<Result<String, String>>,
    },
    Complete {
        session: String,
        prefix: String,
        reply: mpsc::Sender<Result<Vec<String>, String>>,
    },
    Doc {
        session: String,
        symbol: String,
        reply: mpsc::Sender<Result<Documentation, String>>,
    },
    Create {
        session: String,
        reply: mpsc::Sender<Result<String, String>>,
    },
    Close {
        session: String,
        reply: mpsc::Sender<Result<String, String>>,
    },
    List {
        reply: mpsc::Sender<Result<Vec<String>, String>>,
    },
    Info {
        session: String,
        reply: mpsc::Sender<Result<String, String>>,
    },
    RegisterResource {
        name: String,
        source: String,
        reply: mpsc::Sender<Result<(), String>>,
    },
    RemoveResource {
        name: String,
        reply: mpsc::Sender<Result<(), String>>,
    },
    ListResources {
        reply: mpsc::Sender<Result<Vec<String>, String>>,
    },
    InstallModule {
        session: String,
        manifest: String,
        module: crate::wasmtime_provider::CompiledWasmModule,
        reply: mpsc::Sender<Result<String, String>>,
    },
    InvokeModule {
        session: String,
        namespace: String,
        export: String,
        arguments: Vec<u8>,
        reply: mpsc::Sender<Result<Vec<u8>, String>>,
    },
    InvokeHta {
        session: String,
        qualified_var: String,
        arguments: Vec<u8>,
        reply: mpsc::Sender<Result<Vec<u8>, InvokeHtaError>>,
    },
    SandboxOpen {
        spec: SandboxSpec,
        reply: mpsc::Sender<Result<SandboxId, String>>,
    },
    SandboxEval {
        sandbox: SandboxId,
        source: String,
        started: mpsc::Sender<Result<EvaluationId, String>>,
        reply: mpsc::Sender<Result<String, String>>,
    },
    SandboxCall {
        sandbox: SandboxId,
        callable: String,
        arguments: Vec<u8>,
        started: mpsc::Sender<Result<EvaluationId, String>>,
        reply: mpsc::Sender<Result<Vec<u8>, String>>,
    },
    SandboxCancel {
        sandbox: SandboxId,
        evaluation: Option<EvaluationId>,
        reply: mpsc::Sender<Result<bool, String>>,
    },
    SandboxStatus {
        sandbox: SandboxId,
        reply: mpsc::Sender<Result<SandboxStatus, String>>,
    },
    SandboxClose {
        sandbox: SandboxId,
        reply: mpsc::Sender<Result<(), String>>,
    },
    Shutdown,
}

struct BrokerHandle {
    sender: mpsc::Sender<Request>,
}

impl Drop for BrokerHandle {
    fn drop(&mut self) {
        let _ = self.sender.send(Request::Shutdown);
    }
}

#[derive(Clone)]
pub struct RuntimeBroker {
    handle: Arc<BrokerHandle>,
}

impl RuntimeBroker {
    pub fn start() -> Result<Self, String> {
        Self::start_with_bootstrap(None, false, false, false, RuntimeBootstrap::Full)
    }

    /// Starts an isolated broker with the portable core-language runtime.
    ///
    /// This is intended for small embedding surfaces and focused tests
    /// that do not require the language-level Foundation bundle.
    pub fn start_core() -> Result<Self, String> {
        Self::start_with_bootstrap(None, false, false, false, RuntimeBootstrap::Core)
    }

    pub fn start_with(
        root: Option<PathBuf>,
        native_sockets: bool,
        allow_process: bool,
        allow_postgres: bool,
    ) -> Result<Self, String> {
        Self::start_with_bootstrap(
            root,
            native_sockets,
            allow_process,
            allow_postgres,
            RuntimeBootstrap::Full,
        )
    }

    /// Starts a full broker with the requested ordinary evaluation backend.
    /// Library callers retain the interpreter-default `start_with` entrypoint;
    /// command-line frontends use this method to make native execution explicit
    /// while preserving an interpreter escape hatch.
    pub fn start_with_backend(
        root: Option<PathBuf>,
        native_sockets: bool,
        allow_process: bool,
        allow_postgres: bool,
        execution_backend: &str,
    ) -> Result<Self, String> {
        Self::start_with_bootstrap_and_backend(
            root,
            native_sockets,
            allow_process,
            allow_postgres,
            RuntimeBootstrap::Full,
            execution_backend,
        )
    }

    /// Starts a full Foundation-backed broker with project namespaces loaded
    /// lazily from a native source catalog.
    pub fn start_with_backend_and_source_catalog(
        root: Option<PathBuf>,
        native_sockets: bool,
        allow_process: bool,
        allow_postgres: bool,
        execution_backend: &str,
        source_catalog: crate::project::SourceCatalog,
    ) -> Result<Self, String> {
        Self::start_with_bootstrap_and_backend_and_catalog(
            root,
            native_sockets,
            allow_process,
            allow_postgres,
            RuntimeBootstrap::Full,
            execution_backend,
            Some(source_catalog),
        )
    }

    /// Starts a broker whose language libraries are resolved from a native
    /// project source catalog. Foundation is bootstrapped from source before
    /// the requested ordinary backend is enabled.
    pub fn start_with_source_catalog(
        root: Option<PathBuf>,
        native_sockets: bool,
        allow_process: bool,
        allow_postgres: bool,
        execution_backend: &str,
        source_catalog: crate::project::SourceCatalog,
    ) -> Result<Self, String> {
        Self::start_with_bootstrap_and_backend_and_catalog(
            root,
            native_sockets,
            allow_process,
            allow_postgres,
            RuntimeBootstrap::Source,
            execution_backend,
            Some(source_catalog),
        )
    }

    fn start_with_bootstrap(
        root: Option<PathBuf>,
        native_sockets: bool,
        allow_process: bool,
        allow_postgres: bool,
        bootstrap: RuntimeBootstrap,
    ) -> Result<Self, String> {
        Self::start_with_bootstrap_and_backend(
            root,
            native_sockets,
            allow_process,
            allow_postgres,
            bootstrap,
            "interpreter",
        )
    }

    fn start_with_bootstrap_and_backend(
        root: Option<PathBuf>,
        native_sockets: bool,
        allow_process: bool,
        allow_postgres: bool,
        bootstrap: RuntimeBootstrap,
        execution_backend: &str,
    ) -> Result<Self, String> {
        Self::start_with_bootstrap_and_backend_and_catalog(
            root,
            native_sockets,
            allow_process,
            allow_postgres,
            bootstrap,
            execution_backend,
            None,
        )
    }

    fn start_with_bootstrap_and_backend_and_catalog(
        root: Option<PathBuf>,
        native_sockets: bool,
        allow_process: bool,
        allow_postgres: bool,
        bootstrap: RuntimeBootstrap,
        execution_backend: &str,
        source_catalog: Option<crate::project::SourceCatalog>,
    ) -> Result<Self, String> {
        crate::validate_execution_backend(execution_backend)?;
        let execution_backend = execution_backend.to_owned();
        let (sender, receiver) = mpsc::channel();
        std::thread::Builder::new()
            .name("hara-runtime-broker".into())
            .stack_size(RUNTIME_BROKER_STACK_SIZE)
            .spawn(move || {
                run(
                    receiver,
                    root,
                    native_sockets,
                    allow_process,
                    allow_postgres,
                    bootstrap,
                    execution_backend,
                    source_catalog,
                )
            })
            .map_err(|error| format!("runtime broker failed: {error}"))?;
        Ok(Self {
            handle: Arc::new(BrokerHandle { sender }),
        })
    }

    pub fn eval(&self, session: &str, source: &str) -> Result<String, String> {
        self.call(|reply| Request::Eval {
            session: session.into(),
            source: source.into(),
            reply,
        })
    }

    pub fn namespace(&self, session: &str) -> Result<String, String> {
        self.call(|reply| Request::Namespace {
            session: session.into(),
            reply,
        })
    }

    pub fn complete(&self, session: &str, prefix: &str) -> Result<Vec<String>, String> {
        self.call(|reply| Request::Complete {
            session: session.into(),
            prefix: prefix.into(),
            reply,
        })
    }

    pub fn documentation(&self, session: &str, symbol: &str) -> Result<Documentation, String> {
        self.call(|reply| Request::Doc {
            session: session.into(),
            symbol: symbol.into(),
            reply,
        })
    }

    pub fn create(&self, session: &str) -> Result<String, String> {
        self.call(|reply| Request::Create {
            session: session.into(),
            reply,
        })
    }

    pub fn close(&self, session: &str) -> Result<String, String> {
        self.call(|reply| Request::Close {
            session: session.into(),
            reply,
        })
    }

    pub fn list(&self) -> Result<Vec<String>, String> {
        self.call(|reply| Request::List { reply })
    }

    pub fn info(&self, session: &str) -> Result<String, String> {
        self.call(|reply| Request::Info {
            session: session.into(),
            reply,
        })
    }

    pub fn register_resource(&self, name: &str, source: &str) -> Result<(), String> {
        self.call(|reply| Request::RegisterResource {
            name: name.into(),
            source: source.into(),
            reply,
        })
    }

    pub fn remove_resource(&self, name: &str) -> Result<(), String> {
        self.call(|reply| Request::RemoveResource {
            name: name.into(),
            reply,
        })
    }

    pub fn resources(&self) -> Result<Vec<String>, String> {
        self.call(|reply| Request::ListResources { reply })
    }

    pub fn install_module(
        &self,
        session: &str,
        manifest: &str,
        module: &crate::wasmtime_provider::CompiledWasmModule,
    ) -> Result<String, String> {
        self.call(|reply| Request::InstallModule {
            session: session.into(),
            manifest: manifest.into(),
            module: module.clone(),
            reply,
        })
    }

    pub fn invoke_hta(
        &self,
        session: &str,
        qualified_var: &str,
        arguments: &[u8],
    ) -> Result<Vec<u8>, InvokeHtaError> {
        let (reply, response) = mpsc::channel();
        self.handle
            .sender
            .send(Request::InvokeHta {
                session: session.into(),
                qualified_var: qualified_var.into(),
                arguments: arguments.into(),
                reply,
            })
            .map_err(|_| InvokeHtaError::BrokerClosed)?;
        response.recv().map_err(|_| InvokeHtaError::BrokerStopped)?
    }

    pub fn invoke_module(
        &self,
        session: &str,
        namespace: &str,
        export: &str,
        arguments: &[u8],
    ) -> Result<Vec<u8>, String> {
        self.call(|reply| Request::InvokeModule {
            session: session.into(),
            namespace: namespace.into(),
            export: export.into(),
            arguments: arguments.into(),
            reply,
        })
    }

    fn sandbox_open(&self, spec: SandboxSpec) -> Result<SandboxId, String> {
        self.call(|reply| Request::SandboxOpen { spec, reply })
    }

    fn sandbox_eval_receiver(
        &self,
        sandbox: SandboxId,
        source: &str,
    ) -> Result<(EvaluationId, mpsc::Receiver<Result<String, String>>), String> {
        let (reply, response) = mpsc::channel();
        let (started_reply, started_response) = mpsc::channel();
        self.handle
            .sender
            .send(Request::SandboxEval {
                sandbox,
                source: source.into(),
                started: started_reply,
                reply,
            })
            .map_err(|_| "runtime broker is closed".to_owned())?;
        let evaluation = started_response.recv().map_err(|_| {
            "runtime broker stopped before starting sandbox evaluation".to_owned()
        })??;
        Ok((evaluation, response))
    }

    fn sandbox_call_receiver(
        &self,
        sandbox: SandboxId,
        callable: &str,
        arguments: &[u8],
    ) -> Result<(EvaluationId, mpsc::Receiver<Result<Vec<u8>, String>>), String> {
        let (reply, response) = mpsc::channel();
        let (started_reply, started_response) = mpsc::channel();
        self.handle
            .sender
            .send(Request::SandboxCall {
                sandbox,
                callable: callable.into(),
                arguments: arguments.into(),
                started: started_reply,
                reply,
            })
            .map_err(|_| "runtime broker is closed".to_owned())?;
        let evaluation = started_response
            .recv()
            .map_err(|_| "runtime broker stopped before starting sandbox call".to_owned())??;
        Ok((evaluation, response))
    }

    fn sandbox_cancel(&self, sandbox: SandboxId) -> Result<bool, String> {
        self.call(|reply| Request::SandboxCancel {
            sandbox,
            evaluation: None,
            reply,
        })
    }

    fn sandbox_cancel_evaluation(
        &self,
        sandbox: SandboxId,
        evaluation: EvaluationId,
    ) -> Result<bool, String> {
        self.call(|reply| Request::SandboxCancel {
            sandbox,
            evaluation: Some(evaluation),
            reply,
        })
    }

    fn sandbox_status(&self, sandbox: SandboxId) -> Result<SandboxStatus, String> {
        self.call(|reply| Request::SandboxStatus { sandbox, reply })
    }

    fn sandbox_close(&self, sandbox: SandboxId) -> Result<(), String> {
        self.call(|reply| Request::SandboxClose { sandbox, reply })
    }

    fn call<T>(
        &self,
        request: impl FnOnce(mpsc::Sender<Result<T, String>>) -> Request,
    ) -> Result<T, String> {
        let (reply, response) = mpsc::channel();
        self.handle
            .sender
            .send(request(reply))
            .map_err(|_| "runtime broker is closed".to_owned())?;
        response
            .recv()
            .map_err(|_| "runtime broker stopped without a response".to_owned())?
    }
}

fn runtime(
    root: Option<&PathBuf>,
    native_sockets: bool,
    allow_process: bool,
    allow_postgres: bool,
    bootstrap: RuntimeBootstrap,
    execution_backend: &str,
    source_catalog: Option<&crate::project::SourceCatalog>,
) -> Runtime {
    let mut runtime = match bootstrap {
        RuntimeBootstrap::Full => Runtime::new(),
        RuntimeBootstrap::Core | RuntimeBootstrap::Source => Runtime::core(),
    };
    if let Some(source_catalog) = source_catalog {
        runtime.register_source_catalog(source_catalog);
    }
    if matches!(bootstrap, RuntimeBootstrap::Source) {
        runtime
            .bootstrap_source_foundation()
            .expect("source Foundation bootstrap must be valid");
    }
    if let Some(root) = root {
        runtime.install_native_file_provider(root.to_string_lossy().as_ref());
    }
    if native_sockets {
        runtime.install_native_socket_provider();
    }
    if allow_process {
        runtime.install_native_process_provider();
    }
    // Native bootstrap recompiles evaluator-created protocol closures. Install
    // the host providers first so those closures capture the runtime's actual
    // authority instead of the zero-provider bootstrap context.
    runtime
        .configure_execution_backend(execution_backend)
        .expect("validated execution backend must configure");
    if allow_postgres {
        runtime
            .install_native_module(hara_db_postgres::module())
            .expect("db.postgres native module must install once per runtime");
    }
    runtime
}

fn run(
    receiver: mpsc::Receiver<Request>,
    root: Option<PathBuf>,
    native_sockets: bool,
    allow_process: bool,
    allow_postgres: bool,
    bootstrap: RuntimeBootstrap,
    execution_backend: String,
    source_catalog: Option<crate::project::SourceCatalog>,
) {
    let runtime_root = root.clone();
    let runtime_backend = execution_backend.clone();
    let runtime_catalog = source_catalog;
    let runtime_factory: Rc<dyn Fn() -> Runtime> = Rc::new(move || {
        runtime(
            runtime_root.as_ref(),
            native_sockets,
            allow_process,
            allow_postgres,
            bootstrap,
            &runtime_backend,
            runtime_catalog.as_ref(),
        )
    });
    let root_runtime = runtime_factory();
    let mut kernel = SessionKernel::with_runtime_factory(root_runtime, runtime_factory);
    kernel.register_sandbox_provider(Rc::new(InProcessSandboxProvider));
    while let Ok(request) = receiver.recv() {
        match request {
            Request::Eval {
                session,
                source,
                reply,
            } => {
                let result = broker_session_id(&session).and_then(|id| {
                    let runtime = kernel.session_mut(&id)?.runtime_mut()?;
                    if execution_backend == "direct-native" {
                        runtime.eval_native(&source)
                    } else {
                        runtime.eval_native_traced(&source)
                    }
                });
                let _ = reply.send(result);
            }
            Request::Namespace { session, reply } => {
                let result =
                    broker_session_id(&session).and_then(|id| kernel.session_namespace(&id));
                let _ = reply.send(result);
            }
            Request::Complete {
                session,
                prefix,
                reply,
            } => {
                let result = broker_session_id(&session).and_then(|id| {
                    kernel.session(&id)?.runtime().map(|runtime| {
                        let mut symbols = runtime
                            .visible_symbols()
                            .into_iter()
                            .filter(|symbol| symbol.starts_with(&prefix))
                            .collect::<Vec<_>>();
                        symbols.dedup();
                        symbols
                    })
                });
                let _ = reply.send(result);
            }
            Request::Doc {
                session,
                symbol,
                reply,
            } => {
                let result = broker_session_id(&session)
                    .and_then(|id| documentation(kernel.session(&id)?.runtime()?, &symbol));
                let _ = reply.send(result);
            }
            Request::Create { session, reply } => {
                let result = SessionId::parse(&session)
                    .map_err(|_| format!("Session already exists or is invalid: {session}"))
                    .and_then(|id| {
                        kernel
                            .create_session(id)
                            .map_err(|_| format!("Session already exists or is invalid: {session}"))
                    })
                    .map(|_| session);
                let _ = reply.send(result);
            }
            Request::Close { session, reply } => {
                let result = broker_session_id(&session)
                    .and_then(|id| kernel.close_session(&id))
                    .map(|_| session)
                    .map_err(|error| match error.as_str() {
                        "ROOT_CANNOT_CLOSE" => "ROOT cannot be closed".into(),
                        _ if error.starts_with("NO_SESSION ") => {
                            format!("No session: {}", error.trim_start_matches("NO_SESSION "))
                        }
                        _ => error,
                    });
                let _ = reply.send(result);
            }
            Request::List { reply } => {
                let names = kernel
                    .session_names()
                    .into_iter()
                    .map(|id| id.to_string())
                    .collect();
                let _ = reply.send(Ok(names));
            }
            Request::Info { session, reply } => {
                let result = broker_session_id(&session)
                    .and_then(|id| kernel.session_namespace(&id))
                    .map(|namespace| format!("{session} {namespace}"));
                let _ = reply.send(result);
            }
            Request::RegisterResource {
                name,
                source,
                reply,
            } => {
                kernel.register_resource(&name, &source);
                let _ = reply.send(Ok(()));
            }
            Request::RemoveResource { name, reply } => {
                kernel.remove_resource(&name);
                let _ = reply.send(Ok(()));
            }
            Request::ListResources { reply } => {
                let _ = reply.send(Ok(kernel.resource_names()));
            }
            Request::InstallModule {
                session,
                manifest,
                module,
                reply,
            } => {
                let result = broker_session_id(&session).and_then(|id| {
                    let runtime = kernel.session_mut(&id)?.runtime_mut()?;
                    let provider = module.provider();
                    let parsed =
                        crate::extension::ExtensionManifest::parse(&manifest, "MODULE PUT")?;
                    let namespace = parsed.namespace.clone();
                    runtime.install_wasm_extension(&manifest, "MODULE PUT", provider)?;
                    Ok(namespace)
                });
                let _ = reply.send(result);
            }
            Request::InvokeModule {
                session,
                namespace,
                export,
                arguments,
                reply,
            } => {
                let result = broker_session_id(&session).and_then(|id| {
                    let runtime = kernel.session_mut(&id)?.runtime_mut()?;
                    let arguments = crate::hta::decode(&arguments)?;
                    let arguments: Vec<crate::extension::Value> = match arguments {
                        crate::extension::Value::Vector(values) => values.iter().cloned().collect(),
                        crate::extension::Value::Tuple(values) => values.iter().cloned().collect(),
                        other => {
                            return Err(format!(
                                "hta/arguments: expected vector, got {}",
                                other.display()
                            ))
                        }
                    };
                    let result = runtime.invoke_wasm_extension(&namespace, &export, &arguments)?;
                    crate::hta::encode(&result)
                });
                let _ = reply.send(result);
            }
            Request::InvokeHta {
                session,
                qualified_var,
                arguments,
                reply,
            } => {
                let result = SessionId::parse(&session)
                    .map_err(|_| InvokeHtaError::SessionMissing(session.clone()))
                    .and_then(|id| {
                        kernel
                            .session_mut(&id)
                            .map_err(|_| InvokeHtaError::SessionMissing(session.clone()))?
                            .runtime_mut()
                            .map_err(InvokeHtaError::Execution)?
                            .invoke_hta(&qualified_var, &arguments)
                    });
                let _ = reply.send(result);
            }
            Request::SandboxOpen { spec, reply } => {
                let _ = reply.send(kernel.open_sandbox(spec).map_err(|error| error.to_string()));
            }
            Request::SandboxEval {
                sandbox,
                source,
                started,
                reply,
            } => match kernel.sandbox_eval(sandbox, &source) {
                Ok(pending) => {
                    let _ = started.send(Ok(pending.evaluation()));
                    std::thread::spawn(move || {
                        let _ = reply.send(pending.wait().map_err(|error| error.to_string()));
                    });
                }
                Err(error) => {
                    let error = error.to_string();
                    let _ = started.send(Err(error.clone()));
                    let _ = reply.send(Err(error));
                }
            },
            Request::SandboxCall {
                sandbox,
                callable,
                arguments,
                started,
                reply,
            } => match kernel.sandbox_call(sandbox, &callable, &arguments) {
                Ok(pending) => {
                    let _ = started.send(Ok(pending.evaluation()));
                    std::thread::spawn(move || {
                        let _ = reply.send(pending.wait().map_err(|error| error.to_string()));
                    });
                }
                Err(error) => {
                    let error = error.to_string();
                    let _ = started.send(Err(error.clone()));
                    let _ = reply.send(Err(error));
                }
            },
            Request::SandboxCancel {
                sandbox,
                evaluation,
                reply,
            } => {
                let result = match evaluation {
                    Some(evaluation) => kernel.cancel_sandbox_evaluation(sandbox, evaluation),
                    None => kernel.cancel_sandbox(sandbox),
                };
                let _ = reply.send(result.map_err(|error| error.to_string()));
            }
            Request::SandboxStatus { sandbox, reply } => {
                let _ = reply.send(
                    kernel
                        .sandbox_status(sandbox)
                        .map_err(|error| error.to_string()),
                );
            }
            Request::SandboxClose { sandbox, reply } => {
                let _ = reply.send(
                    kernel
                        .close_sandbox(sandbox)
                        .map_err(|error| error.to_string()),
                );
            }
            Request::Shutdown => break,
        }
    }
}

fn broker_session_id(session: &str) -> Result<SessionId, String> {
    SessionId::parse(session).map_err(|_| format!("No session: {session}"))
}

fn documentation(runtime: &Runtime, symbol: &str) -> Result<Documentation, String> {
    documentation::lookup(runtime, symbol)
}

/// Installs the generic native driver behind `std.native.Kernel/*`.
/// Command policy remains in Hara; this adapter only multiplexes isolated
/// evaluator sessions and transfers portable values across the boundary.
pub fn install_native_kernel(runtime: &mut Runtime, broker: RuntimeBroker) {
    runtime.install_native_kernel_provider(Rc::new(move |operation, arguments| {
        kernel_call(&broker, &operation, &arguments)
    }));
}

#[cfg(test)]
mod tests;
