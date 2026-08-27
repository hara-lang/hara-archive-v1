pub use hara_runtime::file;
#[cfg(feature = "evaluation-journal")]
use hara_runtime::journal;
#[cfg(feature = "bytecode-vm")]
use hara_runtime::vm;
use hara_runtime::{core, hta, kernel, lang};

use core::{EvalFiber, EvalFiberState, Promise, PromiseRejection, PromiseState, Value};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;

#[cfg(target_arch = "wasm32")]
mod wasm_random {
    #[link(wasm_import_module = "env")]
    extern "C" {
        fn hara_random_fill(pointer: *mut u8, length: usize) -> i32;
    }

    fn fill(bytes: &mut [u8]) -> Result<(), getrandom::Error> {
        let status = unsafe { hara_random_fill(bytes.as_mut_ptr(), bytes.len()) };
        if status == 0 {
            Ok(())
        } else {
            Err(getrandom::Error::UNSUPPORTED)
        }
    }

    getrandom::register_custom_getrandom!(fill);
}

const FOUNDATION_RESOURCES: &[(&str, &str)] = &[
    (
        "std.foundation",
        include_str!("../../../lib/src/std/foundation.hal"),
    ),
    (
        "std.foundation.promise",
        include_str!("../../../lib/src/std/foundation/promise.hal"),
    ),
    (
        "std.foundation.string",
        include_str!("../../../lib/src/std/foundation/string.hal"),
    ),
    (
        "std.foundation.bytes",
        include_str!("../../../lib/src/std/foundation/bytes.hal"),
    ),
    (
        "std.foundation.coroutine",
        include_str!("../../../lib/src/std/foundation/coroutine.hal"),
    ),
    (
        "std.foundation.pretty",
        include_str!("../../../lib/src/std/foundation/pretty.hal"),
    ),
];

#[cfg(feature = "rich-hta")]
const RICH_HTA_RESOURCES: &[(&str, &str)] = &[(
    "hara.hta.provider.common",
    include_str!("../../../../providers/shared/hta/src/hara/hta/provider/common.hal"),
)];

const SUBSTRATE_RESOURCES: &[(&str, &str)] = &[
    (
        "std.substrate.core",
        include_str!("../../../lib/src/std/substrate/core.hal"),
    ),
    (
        "std.substrate.frame",
        include_str!("../../../lib/src/std/substrate/frame.hal"),
    ),
    (
        "std.substrate.json",
        include_str!("../../../lib/src/std/substrate/json.hal"),
    ),
    (
        "std.substrate.protocol",
        include_str!("../../../lib/src/std/substrate/protocol.hal"),
    ),
    (
        "std.substrate.pubsub",
        include_str!("../../../lib/src/std/substrate/pubsub.hal"),
    ),
    (
        "std.substrate.request",
        include_str!("../../../lib/src/std/substrate/request.hal"),
    ),
    (
        "std.substrate.result",
        include_str!("../../../lib/src/std/substrate/result.hal"),
    ),
    (
        "std.substrate.router",
        include_str!("../../../lib/src/std/substrate/router.hal"),
    ),
    (
        "std.substrate.space",
        include_str!("../../../lib/src/std/substrate/space.hal"),
    ),
    (
        "std.substrate.transport-memory",
        include_str!("../../../lib/src/std/substrate/transport_memory.hal"),
    ),
    (
        "std.substrate.util",
        include_str!("../../../lib/src/std/substrate/util.hal"),
    ),
    (
        "std.substrate.util-handlers",
        include_str!("../../../lib/src/std/substrate/util_handlers.hal"),
    ),
    (
        "std.substrate",
        include_str!("../../../lib/src/std/substrate.hal"),
    ),
];

const SANDBOX_FORBIDDEN_NATIVE_TYPES: &[&str] = &[
    "Runtime", "Kernel", "Sandbox", "Package", "Crypto", "OS", "Process", "File", "Socket", "Host",
    "Work",
];
const MAX_SANDBOX_SOURCE_BYTES: usize = 1_048_576;

#[cfg(feature = "rich-hta")]
mod rich_hta {
    /// A rich HTA artifact is a normal raw Hara runtime with a provider
    /// dispatch source embedded at build time.  The source must define the
    /// stable `hara.hta.provider/dispatch` entrypoint; the runtime supplies
    /// the operation name and decoded arguments as bindings.
    pub(crate) const SOURCE: &str = include_str!(env!("HARA_HTA_PROVIDER_SOURCE"));
}

#[no_mangle]
pub extern "C" fn version() -> i32 {
    1
}
#[no_mangle]
pub extern "C" fn add(left: i32, right: i32) -> i32 {
    left.wrapping_add(right)
}
#[no_mangle]
pub extern "C" fn alloc(size: usize) -> *mut u8 {
    unsafe { std::alloc::alloc(std::alloc::Layout::from_size_align(size.max(1), 1).unwrap()) }
}
#[no_mangle]
pub extern "C" fn hta_alloc(size: usize) -> *mut u8 {
    alloc(size)
}
#[no_mangle]
pub extern "C" fn hta_dealloc(pointer: *mut u8, size: usize) {
    if !pointer.is_null() {
        unsafe {
            std::alloc::dealloc(
                pointer,
                std::alloc::Layout::from_size_align(size.max(1), 1).unwrap(),
            )
        }
    }
}
#[no_mangle]
pub extern "C" fn hta_abi_version() -> i32 {
    4
}

struct Session {
    name: String,
    env: HashMap<String, Value>,
    namespaces: kernel::NamespaceRegistry<Value>,
    /// Guest protocol declarations and extensions must survive across HTA
    /// evaluations just like namespace bindings.  The native runtime owns the
    /// same registry; without this raw WASM kernels could load frame helpers
    /// but not the concrete `std.substrate` node.
    protocols: core::ProtocolRegistry,
    macros: Rc<RefCell<HashMap<(String, String), Rc<core::Function>>>>,
    generated_configs: HashMap<String, kernel::GeneratedNamespaceConfig>,
    next_call: u64,
    events: Rc<RefCell<VecDeque<Vec<u8>>>>,
    ready: Rc<RefCell<VecDeque<(u64, PromiseState)>>>,
    calls: HashMap<u64, (u64, Promise)>,
    fibers: HashMap<u64, EvalFiber>,
    #[cfg(feature = "bytecode-vm")]
    vm_fibers: HashMap<u64, vm::VmFiber>,
    #[cfg(feature = "bytecode-vm")]
    next_vm_program: u64,
    #[cfg(feature = "bytecode-vm")]
    vm_programs: HashMap<u64, Rc<vm::Program>>,
    tasks: HashMap<u64, Promise>,
    active_evaluation: Option<u64>,
    evaluation_queue: VecDeque<EvaluationRequest>,
    resources: Rc<RefCell<HashMap<String, String>>>,
    mount_id: Option<u64>,
    allow_host_calls: bool,
    #[cfg(feature = "evaluation-journal")]
    next_journal_id: u64,
}

enum EvaluationRequest {
    Source {
        task: u64,
        source: String,
        bindings: Vec<Value>,
    },
    Halc {
        task: u64,
        modules: Vec<Vec<u8>>,
    },
    #[cfg(feature = "bytecode-vm")]
    Vm {
        task: u64,
        source: String,
    },
    #[cfg(feature = "bytecode-vm")]
    PreparedVm {
        task: u64,
        program: u64,
    },
}

impl EvaluationRequest {
    fn task(&self) -> u64 {
        match self {
            Self::Source { task, .. } | Self::Halc { task, .. } => *task,
            #[cfg(feature = "bytecode-vm")]
            Self::Vm { task, .. } | Self::PreparedVm { task, .. } => *task,
        }
    }
}
impl Session {
    fn new() -> Self {
        let resources = Rc::new(RefCell::new(
            FOUNDATION_RESOURCES
                .iter()
                .map(|(name, source)| ((*name).into(), (*source).into()))
                .collect::<HashMap<_, _>>(),
        ));
        #[cfg(feature = "rich-hta")]
        resources.borrow_mut().extend(
            RICH_HTA_RESOURCES
                .iter()
                .map(|(name, source)| ((*name).into(), (*source).into())),
        );
        Self::shared("ROOT", resources, Rc::new(RefCell::new(VecDeque::new())))
    }

    fn sandbox(name: &str, events: Rc<RefCell<VecDeque<Vec<u8>>>>) -> Self {
        let resources = Rc::new(RefCell::new(
            FOUNDATION_RESOURCES
                .iter()
                .map(|(name, source)| ((*name).into(), (*source).into()))
                .collect(),
        ));
        let mut session = Self::shared(name, resources, events);
        for native_type in SANDBOX_FORBIDDEN_NATIVE_TYPES {
            let qualified = format!("std.native.{native_type}");
            session.namespaces.remove(&qualified);
            for owner in ["user", "std.foundation", "std.native"] {
                if let Some(owner) = session.namespaces.find(owner) {
                    owner.unalias(native_type);
                    owner.unmap(&lang::data::Symbol::parse(native_type));
                    owner.unmap(&lang::data::Symbol::parse(&qualified));
                }
            }
            session.env.retain(|binding, _| {
                binding != native_type
                    && binding != &qualified
                    && !binding.starts_with(&format!("{native_type}/"))
                    && !binding.starts_with(&format!("{qualified}/"))
            });
        }
        session.allow_host_calls = false;
        session
    }

    fn shared(
        name: &str,
        resources: Rc<RefCell<HashMap<String, String>>>,
        events: Rc<RefCell<VecDeque<Vec<u8>>>>,
    ) -> Self {
        let namespaces = core::minimal_namespace_registry();
        core::install_foundation_intrinsics(&namespaces);
        let mut env = HashMap::new();
        core::select_namespace_environment(&namespaces, &mut env, "user");
        let provider_resources = resources.clone();
        let provider = Rc::new(move |name: &str| {
            provider_resources
                .borrow()
                .get(name)
                .cloned()
                .map(core::NamespaceResource::Source)
        });
        namespaces.set_load_state("std.foundation", kernel::NamespaceLoadState::Unloaded);
        let protocols = core::ProtocolRegistry::core();
        let macros = Rc::new(RefCell::new(HashMap::new()));
        core::with_macros(macros.clone(), || {
            core::with_protocols(&protocols, || {
                core::with_namespace_registry(&namespaces, || {
                    core::with_namespace_source(provider, || {
                        core::require_namespace(&namespaces, &mut env, "std.foundation")?;
                        for &(name, _) in FOUNDATION_RESOURCES.iter().skip(1) {
                            core::require_namespace(&namespaces, &mut env, name)?;
                        }
                        Ok::<(), String>(())
                    })
                })
            })
        })
        .expect("raw runtime foundation resource must load");
        core::apply_global_aliases(&namespaces, "user");
        core::select_namespace_environment(&namespaces, &mut env, "user");
        Self {
            name: name.into(),
            env,
            namespaces,
            protocols,
            macros,
            generated_configs: HashMap::from([(
                "user".into(),
                kernel::GeneratedNamespaceConfig::defaults(),
            )]),
            next_call: 1,
            events,
            ready: Rc::new(RefCell::new(VecDeque::new())),
            calls: HashMap::new(),
            fibers: HashMap::new(),
            #[cfg(feature = "bytecode-vm")]
            vm_fibers: HashMap::new(),
            #[cfg(feature = "bytecode-vm")]
            next_vm_program: 1,
            #[cfg(feature = "bytecode-vm")]
            vm_programs: HashMap::new(),
            tasks: HashMap::new(),
            active_evaluation: None,
            evaluation_queue: VecDeque::new(),
            resources,
            mount_id: None,
            allow_host_calls: true,
            #[cfg(feature = "evaluation-journal")]
            next_journal_id: 1,
        }
    }

    #[cfg(feature = "evaluation-journal")]
    fn journal_eval(&mut self, source: &str) -> journal::Journal {
        let journal_id = journal::JournalId(self.next_journal_id);
        self.next_journal_id += 1;
        let namespaces = self.namespaces.clone();
        let protocols = self.protocols.clone();
        let macros = self.macros.clone();
        let resources = self.resources.clone();
        let provider = Rc::new(move |name: &str| {
            resources
                .borrow()
                .get(name)
                .cloned()
                .map(core::NamespaceResource::Source)
        });
        let mut environment = self.env.clone();
        let (result, journal) = core::with_evaluation_journal(
            journal_id,
            journal::JournalLimits::default(),
            || {
                core::with_namespace_registry(&namespaces, || {
                    core::with_namespace_source(provider, || {
                        core::with_protocols(&protocols, || {
                            let forms = kernel::parse_forms(source)?;
                            let mut fiber = EvalFiber::start_forms(forms, environment.clone())?;
                            let value = fiber.drive_sync()?;
                            environment = fiber.environment();
                            Ok(value)
                        })
                    })
                })
            },
            |value, collector| {
                collector.preview_value(core::portable_type_name(value), value.display())
            },
        );
        if result.is_ok() {
            self.env = environment;
            core::save_namespace_environment(&self.namespaces, &mut self.env);
            core::refresh_namespace_environment(&self.namespaces, &mut self.env);
        }
        journal
    }

    fn busy(&self) -> bool {
        self.active_evaluation.is_some()
            || !self.evaluation_queue.is_empty()
            || !self.fibers.is_empty()
            || {
                #[cfg(feature = "bytecode-vm")]
                {
                    !self.vm_fibers.is_empty()
                }
                #[cfg(not(feature = "bytecode-vm"))]
                {
                    false
                }
            }
            || !self.tasks.is_empty()
            || !self.calls.is_empty()
    }

    fn complete(&self, prefix: &str) -> Value {
        let mut names = self.namespaces.visible_symbol_names();
        let mut extras = self.env.keys().cloned().collect::<Vec<_>>();
        extras.extend(
            core::completion_symbols()
                .iter()
                .map(|name| (*name).to_owned()),
        );
        extras.sort();
        names.extend(extras);
        let mut seen = HashSet::new();
        Value::Vector(
            names
                .into_iter()
                .filter(|name| seen.insert(name.clone()))
                .filter(|name| name.starts_with(prefix))
                .map(Value::String)
                .collect::<Vec<_>>()
                .into(),
        )
    }
    fn event(&self, value: Value) {
        enqueue_event(&self.events, value);
    }
    fn host_handler(
        &mut self,
        _task: u64,
    ) -> (
        Rc<dyn Fn(String, String, Vec<Value>) -> Result<Value, String>>,
        Rc<RefCell<Vec<(u64, Promise, String, String, Vec<Value>)>>>,
        Rc<RefCell<u64>>,
    ) {
        let pending = Rc::new(RefCell::new(Vec::new()));
        let queue = pending.clone();
        let next = Rc::new(RefCell::new(self.next_call));
        let ids = next.clone();
        let allow_host_calls = self.allow_host_calls;
        let handler = Rc::new(move |service: String, method: String, args: Vec<Value>| {
            if !allow_host_calls {
                return Err("hta/host-call-denied: sandbox has no host calls".into());
            }
            let call = *ids.borrow();
            *ids.borrow_mut() += 1;
            let promise = Promise::new();
            queue
                .borrow_mut()
                .push((call, promise.clone(), service, method, args));
            Ok(Value::Promise(promise))
        });
        (handler, pending, next)
    }
    fn collect_calls(
        &mut self,
        task: u64,
        pending: Rc<RefCell<Vec<(u64, Promise, String, String, Vec<Value>)>>>,
        next: Rc<RefCell<u64>>,
    ) {
        self.next_call = *next.borrow();
        for (call, promise, service, method, args) in pending.borrow_mut().drain(..) {
            let value = Value::Vector(
                vec![
                    Value::Number(2),
                    Value::Number(call as i64),
                    Value::Number(task as i64),
                    Value::String(self.name.clone()),
                    self.mount_id
                        .map(|mount| Value::Number(mount as i64))
                        .unwrap_or(Value::Nil),
                    Value::String(service),
                    Value::String(method),
                    Value::Vector(args.into()),
                ]
                .into(),
            );
            match hta::encode(&value) {
                Ok(bytes) => {
                    self.calls.insert(call, (task, promise));
                    self.events.borrow_mut().push_back(bytes);
                }
                Err(error) => {
                    promise.reject(format!("hta/value-unsupported: {error}"));
                }
            }
        }
    }
    fn start_fiber(&mut self, task: u64, source: &str) -> Result<(), String> {
        self.start_fiber_with_bindings(task, source, Vec::new())
    }

    fn enqueue_source(&mut self, task: u64, source: &str, bindings: Vec<Value>) {
        self.evaluation_queue.push_back(EvaluationRequest::Source {
            task,
            source: source.into(),
            bindings,
        });
        self.start_next_evaluation();
    }

    fn enqueue_halc(&mut self, task: u64, modules: Vec<Vec<u8>>) {
        self.evaluation_queue
            .push_back(EvaluationRequest::Halc { task, modules });
        self.start_next_evaluation();
    }

    fn start_next_evaluation(&mut self) {
        if self.active_evaluation.is_some() {
            return;
        }
        while let Some(request) = self.evaluation_queue.pop_front() {
            let task = request.task();
            self.active_evaluation = Some(task);
            let result = match request {
                EvaluationRequest::Source {
                    source, bindings, ..
                } => self.start_fiber_with_bindings(task, &source, bindings),
                EvaluationRequest::Halc { modules, .. } => {
                    let module_refs = modules.iter().map(Vec::as_slice).collect::<Vec<_>>();
                    self.start_halc_bundle(task, &module_refs)
                }
                #[cfg(feature = "bytecode-vm")]
                EvaluationRequest::Vm { source, .. } => self.start_vm_fiber(task, &source),
                #[cfg(feature = "bytecode-vm")]
                EvaluationRequest::PreparedVm { program, .. } => {
                    self.start_prepared_vm_fiber(task, program)
                }
            };
            if let Err(error) = result {
                self.event(event(1, task, error_value("eval/error", error)));
                self.active_evaluation = None;
            }
            if self.active_evaluation.is_some() {
                break;
            }
        }
    }

    fn finish_evaluation(&mut self, task: u64) {
        if self.active_evaluation == Some(task) {
            self.active_evaluation = None;
            self.start_next_evaluation();
        }
    }

    fn commit_environment(&mut self, fiber: &EvalFiber) {
        self.env = fiber.environment();
        core::save_namespace_environment(&self.namespaces, &mut self.env);
        core::refresh_namespace_environment(&self.namespaces, &mut self.env);
    }

    #[cfg(feature = "bytecode-vm")]
    fn enqueue_vm(&mut self, task: u64, source: &str) {
        self.evaluation_queue.push_back(EvaluationRequest::Vm {
            task,
            source: source.into(),
        });
        self.start_next_evaluation();
    }

    #[cfg(feature = "bytecode-vm")]
    fn prepare_vm(&mut self, source: &str) -> Result<u64, String> {
        let program = vm::compile_source_with(source, &self.namespaces)
            .map(Rc::new)
            .map_err(|error| error.to_string())?;
        let id = self.next_vm_program;
        self.next_vm_program = id.saturating_add(1);
        self.vm_programs.insert(id, program);
        Ok(id)
    }

    #[cfg(feature = "bytecode-vm")]
    fn enqueue_prepared_vm(&mut self, task: u64, program: u64) -> Result<(), String> {
        if !self.vm_programs.contains_key(&program) {
            return Err(format!("vm/program-missing: {program}"));
        }
        self.evaluation_queue
            .push_back(EvaluationRequest::PreparedVm { task, program });
        self.start_next_evaluation();
        Ok(())
    }

    fn refresh_environment_from_namespaces(&mut self) {
        let current = self.namespaces.current().name().as_str().to_owned();
        core::select_namespace_environment(&self.namespaces, &mut self.env, &current);
    }

    fn prepare_forms(&mut self, forms: Vec<kernel::Form>) -> Result<Vec<kernel::Form>, String> {
        let mut namespace = self.namespaces.current().name().as_str().to_owned();
        let mut prepared = Vec::with_capacity(forms.len());
        for form in forms {
            if let kernel::Form::List(values) = &form {
                if matches!(values.first(), Some(kernel::Form::Symbol(head)) if head == "ns") {
                    namespace = match values.get(1) {
                        Some(kernel::Form::Symbol(name)) if !name.contains('/') => name.clone(),
                        _ => return Err("ns expects an unqualified namespace symbol".into()),
                    };
                    let resources = self.resources.borrow();
                    let config =
                        kernel::GeneratedNamespaceConfig::configure_with(&values[2..], |target| {
                            self.namespaces.find(target).is_some()
                                || resources.contains_key(target)
                                || target == "std.foundation"
                                || target.starts_with("std.foundation.")
                                || target.starts_with("std.lib.")
                        })?;
                    self.generated_configs.insert(namespace.clone(), config);
                    prepared.push(form);
                    continue;
                }
                if matches!(values.first(), Some(kernel::Form::Symbol(head)) if head == "require") {
                    let mut config = self
                        .generated_configs
                        .get(&namespace)
                        .cloned()
                        .unwrap_or_else(kernel::GeneratedNamespaceConfig::defaults);
                    for spec in &values[1..] {
                        // Preserve the evaluator's asynchronous error contract:
                        // missing standalone requires become task failures, not
                        // synchronous request-dispatch failures.
                        config.apply_require(spec, &|_| true)?;
                    }
                    self.generated_configs.insert(namespace.clone(), config);
                    prepared.push(form);
                    continue;
                }
            }
            let config = self
                .generated_configs
                .get(&namespace)
                .cloned()
                .unwrap_or_else(kernel::GeneratedNamespaceConfig::defaults);
            prepared.push(config.rewrite(form));
        }
        Ok(prepared)
    }

    fn start_fiber_with_bindings(
        &mut self,
        task: u64,
        source: &str,
        bindings: Vec<Value>,
    ) -> Result<(), String> {
        let (handler, pending, next) = self.host_handler(task);
        let file_provider = self.mount_id.map(|_| {
            Rc::new(HostFileProvider {
                handler: handler.clone(),
            }) as Rc<dyn core::FileProvider>
        });
        let namespaces = self.namespaces.clone();
        let protocols = self.protocols.clone();
        let macros = self.macros.clone();
        let resources = self.resources.clone();
        let provider = Rc::new(move |name: &str| {
            resources
                .borrow()
                .get(name)
                .cloned()
                .map(core::NamespaceResource::Source)
        });
        let mut environment = self.env.clone();
        for (index, value) in bindings.into_iter().enumerate() {
            environment.insert(format!("__hta_arg_{index}"), value);
        }
        let forms = kernel::read_forms(source)
            .map_err(|error| error.to_string())?
            .iter()
            .map(core::exception_located_form)
            .collect::<Vec<_>>();
        let forms = self.prepare_forms(forms)?;
        let initial_provider = provider.clone();
        let fiber = core::with_capability_providers(file_provider, None, false, None, || {
            core::with_macros(macros, || {
                core::with_namespace_registry(&namespaces, || {
                    core::with_namespace_source(initial_provider, || {
                        core::with_protocols(&protocols, || {
                            core::with_host_calls(handler.clone(), || {
                                EvalFiber::start_forms(forms, environment)
                            })
                        })
                    })
                })
            })
        })?;
        let drive_provider = provider.clone();
        let drive_macros = self.macros.clone();
        let drive_namespaces = self.namespaces.clone();
        let drive_protocols = self.protocols.clone();
        let drive_file_provider = self.mount_id.map(|_| {
            Rc::new(HostFileProvider {
                handler: handler.clone(),
            }) as Rc<dyn core::FileProvider>
        });
        let drive_handler = handler.clone();
        core::with_capability_providers(drive_file_provider, None, false, None, || {
            core::with_promise_provider(Rc::new(core::LocalPromiseProvider), || {
                core::with_macros(drive_macros, || {
                    core::with_namespace_registry(&drive_namespaces, || {
                        core::with_namespace_source(drive_provider, || {
                            core::with_protocols(&drive_protocols, || {
                                core::with_host_calls(drive_handler, || {
                                    self.collect_calls(task, pending, next);
                                    self.drive(task, fiber);
                                })
                            })
                        })
                    })
                })
            })
        });
        Ok(())
    }
    fn start_halc_fiber(&mut self, task: u64, bytes: &[u8]) -> Result<(), String> {
        self.start_halc_bundle(task, &[bytes])
    }
    fn start_halc_bundle(&mut self, task: u64, modules: &[&[u8]]) -> Result<(), String> {
        let mut forms = Vec::new();
        for bytes in modules {
            forms.extend(kernel::halc::decode_halc(bytes)?.forms);
        }
        let mut forms = self.prepare_forms(forms)?;
        forms.push(kernel::Form::Bool(true));
        let environment = self.env.clone();
        let (handler, pending, next) = self.host_handler(task);
        let file_provider = self.mount_id.map(|_| {
            Rc::new(HostFileProvider {
                handler: handler.clone(),
            }) as Rc<dyn core::FileProvider>
        });
        let namespaces = self.namespaces.clone();
        let protocols = self.protocols.clone();
        let macros = self.macros.clone();
        let resources = self.resources.clone();
        let provider = Rc::new(move |name: &str| {
            resources
                .borrow()
                .get(name)
                .cloned()
                .map(core::NamespaceResource::Source)
        });
        let fiber = core::with_capability_providers(file_provider, None, false, None, || {
            core::with_macros(macros, || {
                core::with_namespace_registry(&namespaces, || {
                    core::with_namespace_source(provider, || {
                        core::with_protocols(&protocols, || {
                            core::with_host_calls(handler, || {
                                EvalFiber::start_forms(forms, environment)
                            })
                        })
                    })
                })
            })
        })?;
        self.collect_calls(task, pending, next);
        self.drive(task, fiber);
        Ok(())
    }
    #[cfg(feature = "bytecode-vm")]
    fn start_vm_fiber(&mut self, task: u64, source: &str) -> Result<(), String> {
        let (handler, pending, next) = self.host_handler(task);
        let namespaces = self.namespaces.clone();
        let protocols = self.protocols.clone();
        let program = vm::compile_source_with(source, &namespaces)
            .map(Rc::new)
            .map_err(|error| error.to_string())?;
        let fiber = core::with_namespace_registry(&namespaces, || {
            core::with_protocols(&protocols, || {
                core::with_host_calls(handler, || vm::VmFiber::start(program))
            })
        });
        self.collect_calls(task, pending, next);
        self.drive_vm(task, fiber);
        Ok(())
    }

    #[cfg(feature = "bytecode-vm")]
    fn start_prepared_vm_fiber(&mut self, task: u64, program: u64) -> Result<(), String> {
        let program = self
            .vm_programs
            .get(&program)
            .cloned()
            .ok_or_else(|| format!("vm/program-missing: {program}"))?;
        let (handler, pending, next) = self.host_handler(task);
        let namespaces = self.namespaces.clone();
        let protocols = self.protocols.clone();
        let fiber = core::with_namespace_registry(&namespaces, || {
            core::with_protocols(&protocols, || {
                core::with_host_calls(handler, || vm::VmFiber::start(program))
            })
        });
        self.collect_calls(task, pending, next);
        self.drive_vm(task, fiber);
        Ok(())
    }

    #[cfg(feature = "bytecode-vm")]
    fn resume_vm_fiber(&mut self, task: u64, state: PromiseState) {
        let Some(mut fiber) = self.vm_fibers.remove(&task) else {
            return;
        };
        let (handler, pending, next) = self.host_handler(task);
        let namespaces = self.namespaces.clone();
        let protocols = self.protocols.clone();
        core::with_namespace_registry(&namespaces, || {
            core::with_protocols(&protocols, || {
                core::with_host_calls(handler, || {
                    fiber.resume(state);
                });
            });
        });
        self.collect_calls(task, pending, next);
        self.drive_vm(task, fiber);
    }

    #[cfg(feature = "bytecode-vm")]
    fn drive_vm(&mut self, task: u64, fiber: vm::VmFiber) {
        match fiber.state() {
            vm::VmFiberState::Suspended => {
                let promise = fiber.pending().expect("suspended VM fiber promise");
                let ready = self.ready.clone();
                promise.on_settle(Rc::new(move |state| {
                    ready.borrow_mut().push_back((task, state))
                }));
                self.vm_fibers.insert(task, fiber);
            }
            vm::VmFiberState::Completed(Value::Promise(promise)) => {
                self.refresh_environment_from_namespaces();
                let events = self.events.clone();
                promise.on_settle(Rc::new(move |state| emit_settlement(&events, task, state)));
                self.tasks.insert(task, promise);
                self.finish_evaluation(task);
            }
            vm::VmFiberState::Completed(value) => {
                self.refresh_environment_from_namespaces();
                self.event(event(0, task, value));
                self.finish_evaluation(task);
            }
            vm::VmFiberState::Failed(error) => {
                self.event(event(1, task, error_value("vm/error", error.to_string())));
                self.finish_evaluation(task);
            }
            vm::VmFiberState::Cancelled => {
                self.event(event(1, task, PromiseRejection::cancelled().value()));
                self.finish_evaluation(task);
            }
            vm::VmFiberState::Yielded(_) => {
                self.event(event(
                    1,
                    task,
                    error_value(
                        "vm/invalid-state",
                        "VM fiber yielded outside of a coroutine driver".into(),
                    ),
                ));
                self.finish_evaluation(task);
            }
            vm::VmFiberState::Running => {
                self.event(event(
                    1,
                    task,
                    error_value("vm/invalid-state", "running VM fiber escaped".into()),
                ));
                self.finish_evaluation(task);
            }
        }
    }
    fn resume_fiber(&mut self, task: u64, state: PromiseState) {
        let Some(mut fiber) = self.fibers.remove(&task) else {
            return;
        };
        let (handler, pending, next) = self.host_handler(task);
        let file_provider = self.mount_id.map(|_| {
            Rc::new(HostFileProvider {
                handler: handler.clone(),
            }) as Rc<dyn core::FileProvider>
        });
        let namespaces = self.namespaces.clone();
        let protocols = self.protocols.clone();
        let macros = self.macros.clone();
        let resources = self.resources.clone();
        let provider = Rc::new(move |name: &str| {
            resources
                .borrow()
                .get(name)
                .cloned()
                .map(core::NamespaceResource::Source)
        });
        core::with_capability_providers(file_provider, None, false, None, || {
            core::with_macros(macros, || {
                core::with_namespace_registry(&namespaces, || {
                    core::with_namespace_source(provider, || {
                        core::with_protocols(&protocols, || {
                            core::with_host_calls(handler, || {
                                fiber.resume(state);
                            });
                        });
                    });
                });
            });
        });
        self.collect_calls(task, pending, next);
        self.drive(task, fiber);
    }
    fn drive(&mut self, task: u64, fiber: EvalFiber) {
        match fiber.state() {
            EvalFiberState::Suspended => {
                let promise = fiber.pending().expect("suspended fiber promise");
                let ready = self.ready.clone();
                promise.on_settle(Rc::new(move |state| {
                    ready.borrow_mut().push_back((task, state))
                }));
                self.fibers.insert(task, fiber);
            }
            EvalFiberState::Completed(Value::Promise(promise)) => {
                self.commit_environment(&fiber);
                let events = self.events.clone();
                promise.on_settle(Rc::new(move |state| emit_settlement(&events, task, state)));
                self.tasks.insert(task, promise);
                self.finish_evaluation(task);
            }
            EvalFiberState::Completed(value) => {
                self.commit_environment(&fiber);
                self.event(event(0, task, value));
                self.finish_evaluation(task);
            }
            EvalFiberState::Failed(error) => {
                self.event(event(1, task, error_value("eval/error", error)));
                self.finish_evaluation(task);
            }
            EvalFiberState::Cancelled => {
                self.event(event(1, task, PromiseRejection::cancelled().value()));
                self.finish_evaluation(task);
            }
            EvalFiberState::Running => {
                self.event(event(
                    1,
                    task,
                    error_value("fiber/invalid-state", "running fiber escaped".into()),
                ));
                self.finish_evaluation(task);
            }
        }
    }
    fn drain_ready(&mut self) {
        loop {
            let next = { self.ready.borrow_mut().pop_front() };
            match next {
                Some((task, state)) => {
                    #[cfg(feature = "bytecode-vm")]
                    if self.vm_fibers.contains_key(&task) {
                        self.resume_vm_fiber(task, state);
                        continue;
                    }
                    self.resume_fiber(task, state)
                }
                None => break,
            }
        }
    }
}

struct HostFileProvider {
    handler: Rc<dyn Fn(String, String, Vec<Value>) -> Result<Value, String>>,
}

impl HostFileProvider {
    fn promise(&self, method: &str, arguments: Vec<Value>) -> Result<Promise, core::FileError> {
        match (self.handler)("file".into(), method.into(), arguments) {
            Ok(Value::Promise(promise)) => Ok(promise),
            Ok(_) => Err(core::FileError::Io(
                "file host call did not return a promise".into(),
            )),
            Err(error) => Err(core::FileError::Io(error)),
        }
    }
}

impl core::FileProvider for HostFileProvider {
    fn read_bytes(&self, _path: &str) -> Result<Vec<u8>, core::FileError> {
        Err(core::FileError::Unsupported)
    }

    fn write_bytes(
        &self,
        _path: &str,
        _bytes: Vec<u8>,
        _options: core::WriteOptions,
    ) -> Result<String, core::FileError> {
        Err(core::FileError::Unsupported)
    }

    fn exists_value(&self, _path: &str) -> Result<bool, core::FileError> {
        Err(core::FileError::Unsupported)
    }

    fn stat_entry(&self, _path: &str) -> Result<core::FileEntry, core::FileError> {
        Err(core::FileError::Unsupported)
    }

    fn entries_values(&self, _path: &str) -> Result<Vec<core::FileEntry>, core::FileError> {
        Err(core::FileError::Unsupported)
    }

    fn mkdir_path(
        &self,
        _path: &str,
        _options: core::MkdirOptions,
    ) -> Result<String, core::FileError> {
        Err(core::FileError::Unsupported)
    }

    fn delete_path(
        &self,
        _path: &str,
        _options: core::DeleteOptions,
    ) -> Result<String, core::FileError> {
        Err(core::FileError::Unsupported)
    }

    fn copy_path(
        &self,
        _source: &str,
        _target: &str,
        _options: core::CopyOptions,
    ) -> Result<String, core::FileError> {
        Err(core::FileError::Unsupported)
    }

    fn move_path(
        &self,
        _source: &str,
        _target: &str,
        _options: core::MoveOptions,
    ) -> Result<String, core::FileError> {
        Err(core::FileError::Unsupported)
    }

    fn temp_file_path(
        &self,
        _parent: &str,
        _options: core::TempFileOptions,
    ) -> Result<String, core::FileError> {
        Err(core::FileError::Unsupported)
    }

    fn temp_directory_path(
        &self,
        _parent: &str,
        _options: core::TempDirectoryOptions,
    ) -> Result<String, core::FileError> {
        Err(core::FileError::Unsupported)
    }

    fn read(&self, path: &str) -> Result<Promise, core::FileError> {
        self.promise("read", vec![Value::String(path.into())])
    }

    fn write(&self, path: &str, bytes: Vec<u8>) -> Result<Promise, core::FileError> {
        self.promise(
            "write",
            vec![Value::String(path.into()), Value::Bytes(bytes)],
        )
    }

    fn write_with_options(
        &self,
        path: &str,
        bytes: Vec<u8>,
        _options: core::WriteOptions,
    ) -> Result<Promise, core::FileError> {
        self.write(path, bytes)
    }

    fn exists(&self, path: &str) -> Result<Promise, core::FileError> {
        self.promise("exists", vec![Value::String(path.into())])
    }

    fn stat(&self, path: &str) -> Result<Promise, core::FileError> {
        self.promise("stat", vec![Value::String(path.into())])
    }

    fn entries(&self, path: &str) -> Result<Promise, core::FileError> {
        self.promise("entries", vec![Value::String(path.into())])
    }

    fn list(&self, path: &str) -> Result<Promise, core::FileError> {
        self.promise("list", vec![Value::String(path.into())])
    }

    fn walk(&self, path: &str) -> Result<Promise, core::FileError> {
        self.promise("walk", vec![Value::String(path.into())])
    }

    fn mkdir(&self, path: &str) -> Result<Promise, core::FileError> {
        self.promise("mkdir", vec![Value::String(path.into())])
    }

    fn mkdir_with_options(
        &self,
        path: &str,
        _options: core::MkdirOptions,
    ) -> Result<Promise, core::FileError> {
        self.mkdir(path)
    }

    fn delete(&self, path: &str) -> Result<Promise, core::FileError> {
        self.promise("delete", vec![Value::String(path.into())])
    }

    fn delete_with_options(
        &self,
        path: &str,
        _options: core::DeleteOptions,
    ) -> Result<Promise, core::FileError> {
        self.delete(path)
    }

    fn copy(
        &self,
        source: &str,
        target: &str,
        _options: core::CopyOptions,
    ) -> Result<Promise, core::FileError> {
        self.promise(
            "copy",
            vec![Value::String(source.into()), Value::String(target.into())],
        )
    }

    fn move_entry(
        &self,
        source: &str,
        target: &str,
        _options: core::MoveOptions,
    ) -> Result<Promise, core::FileError> {
        self.promise(
            "move",
            vec![Value::String(source.into()), Value::String(target.into())],
        )
    }

    fn temp_file(
        &self,
        parent: &str,
        _options: core::TempFileOptions,
    ) -> Result<Promise, core::FileError> {
        self.promise("temp-file", vec![Value::String(parent.into())])
    }

    fn temp_directory(
        &self,
        parent: &str,
        _options: core::TempDirectoryOptions,
    ) -> Result<Promise, core::FileError> {
        self.promise("temp-directory", vec![Value::String(parent.into())])
    }
}

struct FilesystemMount {
    provider: String,
    key: Option<String>,
    attachments: usize,
}

struct SessionKernel {
    next_task: u64,
    resources: Rc<RefCell<HashMap<String, String>>>,
    events: Rc<RefCell<VecDeque<Vec<u8>>>>,
    sessions: HashMap<String, Session>,
    task_sessions: HashMap<u64, String>,
    sandbox_sessions: HashSet<String>,
    mounts: HashMap<u64, FilesystemMount>,
    next_mount_id: u64,
}

impl SessionKernel {
    fn new() -> Self {
        let resources = Rc::new(RefCell::new(HashMap::new()));
        resources.borrow_mut().extend(
            FOUNDATION_RESOURCES
                .iter()
                .map(|(name, source)| ((*name).into(), (*source).into())),
        );
        #[cfg(feature = "rich-hta")]
        resources.borrow_mut().extend(
            RICH_HTA_RESOURCES
                .iter()
                .map(|(name, source)| ((*name).into(), (*source).into())),
        );
        let events = Rc::new(RefCell::new(VecDeque::new()));
        let mut sessions = HashMap::new();
        sessions.insert(
            "ROOT".into(),
            Session::shared("ROOT", resources.clone(), events.clone()),
        );
        Self {
            next_task: 1,
            resources,
            events,
            sessions,
            task_sessions: HashMap::new(),
            sandbox_sessions: HashSet::new(),
            mounts: HashMap::new(),
            next_mount_id: 1,
        }
    }

    fn task(&mut self) -> u64 {
        let task = self.next_task;
        self.next_task += 1;
        task
    }

    fn session(&self, name: &str) -> Result<&Session, String> {
        self.sessions
            .get(name)
            .ok_or_else(|| format!("NO_SESSION {name}"))
    }

    fn session_mut(&mut self, name: &str) -> Result<&mut Session, String> {
        self.sessions
            .get_mut(name)
            .ok_or_else(|| format!("NO_SESSION {name}"))
    }

    fn create_session(&mut self, name: &str) -> Result<(), String> {
        validate_session_name(name)?;
        if self.sessions.contains_key(name) {
            return Err(format!("SESSION_EXISTS {name}"));
        }
        self.sessions.insert(
            name.into(),
            Session::shared(name, self.resources.clone(), self.events.clone()),
        );
        Ok(())
    }

    fn create_filesystem(&mut self, descriptor: &Value) -> Result<u64, String> {
        let entries = core::map_entries(descriptor)
            .ok_or_else(|| "filesystem/create expects a provider descriptor map".to_string())?;
        let field = |name: &str| {
            entries.iter().find_map(|(key, value)| match key {
                Value::String(key) if key == name => Some(value.clone()),
                Value::Keyword(key) if key.as_str() == name => Some(value.clone()),
                _ => None,
            })
        };
        let provider = match field("provider") {
            Some(Value::String(provider)) => provider,
            Some(Value::Keyword(provider)) => provider.as_str().to_owned(),
            _ => return Err("filesystem/create requires a provider".into()),
        };
        if !matches!(provider.as_str(), "memory" | "indexeddb") {
            return Err(format!("FILESYSTEM_PROVIDER_UNSUPPORTED {provider}"));
        }
        let key = match field("key") {
            Some(Value::String(key)) if !key.is_empty() => Some(key),
            Some(Value::Nil) | None if provider == "memory" => None,
            None => return Err("filesystem/create indexeddb requires a key".into()),
            _ => return Err("filesystem/create key must be a non-empty string".into()),
        };
        let mount_id = self.next_mount_id;
        self.next_mount_id = self
            .next_mount_id
            .checked_add(1)
            .filter(|value| *value <= i64::MAX as u64)
            .ok_or_else(|| "FILESYSTEM_IDS_EXHAUSTED".to_string())?;
        self.mounts.insert(
            mount_id,
            FilesystemMount {
                provider,
                key,
                attachments: 0,
            },
        );
        Ok(mount_id)
    }

    fn attach_filesystem(&mut self, name: &str, mount_id: u64) -> Result<(), String> {
        let current = self.session(name)?;
        if current.busy() {
            return Err(format!("SESSION_BUSY {name}"));
        }
        if !self.mounts.contains_key(&mount_id) {
            return Err(format!("NO_FILESYSTEM {mount_id}"));
        }
        if self.session(name)?.mount_id == Some(mount_id) {
            return Ok(());
        }
        self.detach_filesystem(name)?;
        self.mounts.get_mut(&mount_id).unwrap().attachments += 1;
        self.session_mut(name)?.mount_id = Some(mount_id);
        Ok(())
    }

    fn detach_filesystem(&mut self, name: &str) -> Result<(), String> {
        let current = self.session(name)?;
        if current.busy() {
            return Err(format!("SESSION_BUSY {name}"));
        }
        let mount_id = current.mount_id;
        self.session_mut(name)?.mount_id = None;
        if let Some(mount_id) = mount_id {
            if let Some(mount) = self.mounts.get_mut(&mount_id) {
                mount.attachments = mount.attachments.saturating_sub(1);
            }
        }
        Ok(())
    }

    fn close_filesystem(&mut self, mount_id: u64) -> Result<(), String> {
        let mount = self
            .mounts
            .get(&mount_id)
            .ok_or_else(|| format!("NO_FILESYSTEM {mount_id}"))?;
        if mount.attachments != 0 {
            return Err(format!("FILESYSTEM_ATTACHED {mount_id}"));
        }
        self.mounts.remove(&mount_id);
        Ok(())
    }

    fn close_session(&mut self, name: &str) -> Result<(), String> {
        validate_session_name(name)?;
        if name == "ROOT" {
            return Err("ROOT_CANNOT_CLOSE".into());
        }
        if !self.sessions.contains_key(name) {
            return Err(format!("NO_SESSION {name}"));
        }
        let owned = self
            .task_sessions
            .iter()
            .filter_map(|(task, session)| (session == name).then_some(*task))
            .collect::<Vec<_>>();
        for task in owned {
            self.task_sessions.remove(&task);
            enqueue_event(
                &self.events,
                event(
                    1,
                    task,
                    error_value("session/closed", format!("session closed: {name}")),
                ),
            );
        }
        let mount_id = self
            .sessions
            .remove(name)
            .and_then(|runtime| runtime.mount_id);
        if let Some(mount_id) = mount_id {
            if let Some(mount) = self.mounts.get_mut(&mount_id) {
                mount.attachments = mount.attachments.saturating_sub(1);
            }
        }
        self.sandbox_sessions.remove(name);
        Ok(())
    }

    fn cleanup_task(&mut self, task: u64) {
        let Some(session) = self.task_sessions.remove(&task) else {
            return;
        };
        if !self.sandbox_sessions.remove(&session) {
            return;
        }
        if let Some(runtime) = self.sessions.remove(&session) {
            if let Some(mount_id) = runtime.mount_id {
                if let Some(mount) = self.mounts.get_mut(&mount_id) {
                    mount.attachments = mount.attachments.saturating_sub(1);
                }
            }
        }
    }

    fn drain_ready(&mut self) {
        for session in self.sessions.values_mut() {
            session.drain_ready();
        }
    }
}

fn validate_session_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'))
    {
        return Err("INVALID_SESSION_NAME".into());
    }
    Ok(())
}

thread_local! {static KERNEL:RefCell<SessionKernel>=RefCell::new(SessionKernel::new());}
fn event(kind: i64, id: u64, value: Value) -> Value {
    Value::Vector(vec![Value::Number(kind), Value::Number(id as i64), value].into())
}
fn error_value(code: &str, message: String) -> Value {
    Value::Map(
        vec![
            (Value::Keyword("code".into()), Value::Keyword(code.into())),
            (Value::Keyword("message".into()), Value::String(message)),
            (
                Value::Keyword("origin".into()),
                Value::Keyword("wasm".into()),
            ),
            (Value::Keyword("retryable".into()), Value::Bool(false)),
        ]
        .into_iter()
        .collect(),
    )
}
fn emit_settlement(events: &Rc<RefCell<VecDeque<Vec<u8>>>>, task: u64, state: PromiseState) {
    let value = match state {
        PromiseState::Pending => return,
        PromiseState::Fulfilled(value) => event(0, task, value),
        PromiseState::Rejected(PromiseRejection::Value(value))
        | PromiseState::Rejected(PromiseRejection::Cancelled(value)) => event(1, task, value),
        PromiseState::Rejected(PromiseRejection::Message(message)) => {
            event(1, task, error_value("promise/rejected", message))
        }
    };
    enqueue_event(events, value);
}
fn enqueue_event(events: &Rc<RefCell<VecDeque<Vec<u8>>>>, value: Value) {
    match hta::encode(&value) {
        Ok(bytes) => events.borrow_mut().push_back(bytes),
        Err(error) => {
            let id = match &value {
                Value::Vector(values) => match values.get(1) {
                    Some(Value::Number(id)) => *id as u64,
                    _ => 0,
                },
                _ => 0,
            };
            let fallback = event(1, id, error_value("hta/value-unsupported", error));
            if let Ok(bytes) = hta::encode(&fallback) {
                events.borrow_mut().push_back(bytes);
            }
        }
    }
}

fn terminal_task(bytes: &[u8]) -> Option<u64> {
    match hta::decode(bytes) {
        Ok(Value::Vector(values)) if values.len() >= 2 => match (&values[0], &values[1]) {
            (Value::Number(kind), Value::Number(task))
                if (*kind == 0 || *kind == 1) && *task > 0 =>
            {
                Some(*task as u64)
            }
            _ => None,
        },
        _ => None,
    }
}

fn request(bytes: &[u8]) -> Result<(String, Vec<Value>), String> {
    match hta::decode(bytes)? {
        Value::Vector(values) if values.len() == 2 => {
            let target = match &values[0] {
                Value::String(value) => value.clone(),
                _ => return Err("hta/start target must be a string".into()),
            };
            let arguments = match &values[1] {
                Value::Vector(value) => value.iter().cloned().collect(),
                _ => return Err("hta/start arguments must be a vector".into()),
            };
            Ok((target, arguments))
        }
        _ => Err("hta/start expects [target arguments]".into()),
    }
}
#[no_mangle]
pub extern "C" fn hta_start(pointer: *const u8, size: usize) -> i64 {
    let bytes = if pointer.is_null() {
        &[][..]
    } else {
        unsafe { std::slice::from_raw_parts(pointer, size) }
    };
    KERNEL.with(|cell| {
        let mut kernel = cell.borrow_mut();
        let task = kernel.task();
        let result = match request(bytes) {
            Ok((target, args)) => dispatch(&mut kernel, task, &target, args),
            Err(error) => Err(error),
        };
        if let Err(error) = result {
            enqueue_event(
                &kernel.events,
                event(1, task, error_value("eval/error", error)),
            );
        }
        kernel.drain_ready();
        task as i64
    })
}

fn dispatch(
    kernel: &mut SessionKernel,
    task: u64,
    target: &str,
    args: Vec<Value>,
) -> Result<(), String> {
    match target {
        "eval" => dispatch_eval(kernel, task, "ROOT", &args, false),
        "eval-vm" => dispatch_eval_vm(kernel, task, "ROOT", &args),
        "eval-halc" | "eval-hir" => dispatch_eval_halc(kernel, task, "ROOT", &args),
        "eval-halc-bundle" | "eval-hir-bundle" => {
            dispatch_eval_halc_bundle(kernel, task, "ROOT", &args)
        }
        "session/eval-halc" | "session/eval-hir" => match args.as_slice() {
            [Value::String(session), Value::Bytes(bytes)] => {
                dispatch_eval_halc_bytes(kernel, task, session, bytes)
            }
            _ => Err("hta session/eval-halc expects session and byte array".into()),
        },
        "session/eval-halc-bundle" | "session/eval-hir-bundle" => match args.as_slice() {
            [Value::String(session), Value::Vector(modules)] => {
                let modules = modules.iter().cloned().collect::<Vec<_>>();
                dispatch_eval_halc_bundle_values(kernel, task, session, &modules)
            }
            _ => Err("hta session/eval-halc-bundle expects session and byte arrays".into()),
        },
        "eval-bound" => dispatch_eval(kernel, task, "ROOT", &args, true),
        "complete" => dispatch_complete(kernel, task, "ROOT", &args),
        "sandbox/eval" => match args.as_slice() {
            [Value::String(source)] => dispatch_sandbox_eval(kernel, task, source),
            _ => Err("hta sandbox/eval expects one source string".into()),
        },
        "sandbox/call" | "sandbox/check" => Err(format!("hta/capability-unsupported: {target}")),
        "session/eval" => match args.as_slice() {
            [Value::String(session), Value::String(source)] => {
                dispatch_eval_values(kernel, task, session, source, None)
            }
            _ => Err("hta session/eval expects session and source strings".into()),
        },
        "session/eval-vm" => match args.as_slice() {
            [Value::String(session), Value::String(source)] => {
                dispatch_eval_vm_values(kernel, task, session, source)
            }
            _ => Err("hta session/eval-vm expects session and source strings".into()),
        },
        "session/prepare-vm" => match args.as_slice() {
            [Value::String(session), Value::String(source)] => {
                #[cfg(feature = "bytecode-vm")]
                {
                    let program = kernel.session_mut(session)?.prepare_vm(source)?;
                    enqueue_event(
                        &kernel.events,
                        event(0, task, Value::Number(program as i64)),
                    );
                    Ok(())
                }
                #[cfg(not(feature = "bytecode-vm"))]
                {
                    let _ = (kernel, task, session, source);
                    Err("VM_UNAVAILABLE".into())
                }
            }
            _ => Err("hta session/prepare-vm expects session and source strings".into()),
        },
        "session/invoke-vm" => match args.as_slice() {
            [Value::String(session), Value::Number(program)] if *program > 0 => {
                #[cfg(feature = "bytecode-vm")]
                {
                    validate_session_name(session)?;
                    kernel.session(session)?;
                    kernel.task_sessions.insert(task, session.into());
                    kernel
                        .session_mut(session)?
                        .enqueue_prepared_vm(task, *program as u64)
                }
                #[cfg(not(feature = "bytecode-vm"))]
                {
                    let _ = (kernel, task, session, program);
                    Err("VM_UNAVAILABLE".into())
                }
            }
            _ => Err("hta session/invoke-vm expects a session string and program id".into()),
        },
        "session/journal-eval" | "session/trace-eval" => match args.as_slice() {
            [Value::String(session), Value::String(source)] => {
                #[cfg(feature = "evaluation-journal")]
                {
                    let journal = kernel.session_mut(session)?.journal_eval(source);
                    enqueue_event(&kernel.events, event(0, task, journal_value(&journal)));
                    Ok(())
                }
                #[cfg(not(feature = "evaluation-journal"))]
                {
                    let _ = (kernel, task, session, source);
                    Err("TRACE_UNAVAILABLE".into())
                }
            }
            _ => Err("hta session/journal-eval expects session and source strings".into()),
        },
        "session/eval-bound" => match args.as_slice() {
            [Value::String(session), Value::String(source), Value::Vector(bindings)] => {
                dispatch_eval_values(
                    kernel,
                    task,
                    session,
                    source,
                    Some(bindings.iter().cloned().collect()),
                )
            }
            _ => Err("hta session/eval-bound expects session, source, and binding vector".into()),
        },
        "session/complete" => match args.as_slice() {
            [Value::String(session), Value::String(prefix)] => {
                dispatch_complete_values(kernel, task, session, prefix)
            }
            _ => Err("hta session/complete expects session and prefix strings".into()),
        },
        "session/create" => match args.as_slice() {
            [Value::String(session)] => {
                kernel.create_session(session)?;
                enqueue_event(
                    &kernel.events,
                    event(0, task, Value::String(session.clone())),
                );
                Ok(())
            }
            _ => Err("hta session/create expects one session string".into()),
        },
        "session/list" => {
            if !args.is_empty() {
                return Err("hta session/list expects no arguments".into());
            }
            let mut sessions = kernel.sessions.keys().cloned().collect::<Vec<_>>();
            sessions.sort();
            enqueue_event(
                &kernel.events,
                event(
                    0,
                    task,
                    Value::Vector(
                        sessions
                            .into_iter()
                            .map(Value::String)
                            .collect::<Vec<_>>()
                            .into(),
                    ),
                ),
            );
            Ok(())
        }
        "session/info" => match args.as_slice() {
            [Value::String(session)] => {
                let runtime = kernel.session(session)?;
                let value = Value::Map(
                    vec![
                        (
                            Value::Keyword("session/id".into()),
                            Value::String(session.clone()),
                        ),
                        (
                            Value::Keyword("session/state".into()),
                            Value::Keyword(if runtime.busy() { "busy" } else { "idle" }.into()),
                        ),
                        (
                            Value::Keyword("session/filesystem".into()),
                            runtime
                                .mount_id
                                .map(|mount| Value::Number(mount as i64))
                                .unwrap_or(Value::Nil),
                        ),
                    ]
                    .into_iter()
                    .collect(),
                );
                enqueue_event(&kernel.events, event(0, task, value));
                Ok(())
            }
            _ => Err("hta session/info expects one session string".into()),
        },
        "session/attach-filesystem" => match args.as_slice() {
            [Value::String(session), Value::Number(mount_id)] if *mount_id > 0 => {
                kernel.attach_filesystem(session, *mount_id as u64)?;
                enqueue_event(&kernel.events, event(0, task, Value::Bool(true)));
                Ok(())
            }
            _ => Err("hta session/attach-filesystem expects a session string and mount id".into()),
        },
        "session/detach-filesystem" => match args.as_slice() {
            [Value::String(session)] => {
                kernel.detach_filesystem(session)?;
                enqueue_event(&kernel.events, event(0, task, Value::Bool(true)));
                Ok(())
            }
            _ => Err("hta session/detach-filesystem expects one session string".into()),
        },
        "filesystem/create" => match args.as_slice() {
            [descriptor] => {
                let mount_id = kernel.create_filesystem(descriptor)?;
                enqueue_event(
                    &kernel.events,
                    event(0, task, Value::Number(mount_id as i64)),
                );
                Ok(())
            }
            _ => Err("hta filesystem/create expects one provider descriptor".into()),
        },
        "filesystem/info" => match args.as_slice() {
            [Value::Number(mount_id)] if *mount_id > 0 => {
                let mount = kernel
                    .mounts
                    .get(&(*mount_id as u64))
                    .ok_or_else(|| format!("NO_FILESYSTEM {mount_id}"))?;
                let value = Value::Map(
                    vec![
                        (
                            Value::Keyword("filesystem/id".into()),
                            Value::Number(*mount_id),
                        ),
                        (
                            Value::Keyword("filesystem/provider".into()),
                            Value::Keyword(mount.provider.clone().into()),
                        ),
                        (
                            Value::Keyword("filesystem/key".into()),
                            mount.key.clone().map(Value::String).unwrap_or(Value::Nil),
                        ),
                        (
                            Value::Keyword("filesystem/attachments".into()),
                            Value::Number(mount.attachments as i64),
                        ),
                    ]
                    .into_iter()
                    .collect(),
                );
                enqueue_event(&kernel.events, event(0, task, value));
                Ok(())
            }
            _ => Err("hta filesystem/info expects one mount id".into()),
        },
        "filesystem/close" => match args.as_slice() {
            [Value::Number(mount_id)] if *mount_id > 0 => {
                kernel.close_filesystem(*mount_id as u64)?;
                enqueue_event(&kernel.events, event(0, task, Value::Bool(true)));
                Ok(())
            }
            _ => Err("hta filesystem/close expects one mount id".into()),
        },
        "session/close" => match args.as_slice() {
            [Value::String(session)] => {
                kernel.close_session(session)?;
                enqueue_event(&kernel.events, event(0, task, Value::Bool(true)));
                Ok(())
            }
            _ => Err("hta session/close expects one session string".into()),
        },
        "register-resource" => match args.as_slice() {
            [Value::String(name), Value::String(source)] => {
                kernel
                    .resources
                    .borrow_mut()
                    .insert(name.clone(), source.clone());
                enqueue_event(&kernel.events, event(0, task, Value::Bool(true)));
                Ok(())
            }
            _ => Err("hta register-resource expects name and source strings".into()),
        },
        "register-resources" => match args.as_slice() {
            [Value::Vector(resources)] => {
                let mut staged = Vec::with_capacity(resources.len());
                for entry in resources.iter() {
                    let Value::Vector(entry) = entry else {
                        return Err("hta register-resources expects string pairs".into());
                    };
                    let (Some(Value::String(name)), Some(Value::String(source))) =
                        (entry.get(0), entry.get(1))
                    else {
                        return Err("hta register-resources expects string pairs".into());
                    };
                    if entry.len() != 2 {
                        return Err("hta register-resources expects string pairs".into());
                    }
                    staged.push((name.clone(), source.clone()));
                }
                kernel.resources.borrow_mut().extend(staged);
                enqueue_event(&kernel.events, event(0, task, Value::Bool(true)));
                Ok(())
            }
            _ => Err("hta register-resources expects one vector of string pairs".into()),
        },
        "provider/call" => {
            #[cfg(feature = "rich-hta")]
            {
                match args.as_slice() {
                    [Value::String(operation), Value::Vector(arguments)] => {
                        dispatch_rich_hta(kernel, task, operation, arguments.iter().cloned().collect())
                    }
                    _ => Err("hta provider/call expects an operation and argument vector".into()),
                }
            }
            #[cfg(not(feature = "rich-hta"))]
            {
                let _ = (kernel, task, args);
                Err("hta/rich-provider-unavailable".into())
            }
        }
        _ => {
            #[cfg(feature = "rich-hta")]
            {
                dispatch_rich_hta(kernel, task, target, args)
            }
            #[cfg(not(feature = "rich-hta"))]
            {
                let _ = (kernel, task, args);
                Err(format!("hta/target-unknown: {target}"))
            }
        }
    }
}

#[cfg(feature = "rich-hta")]
fn dispatch_rich_hta(
    kernel: &mut SessionKernel,
    task: u64,
    operation: &str,
    arguments: Vec<Value>,
) -> Result<(), String> {
    let source = format!(
        "(do\n{}\n(ns user)\n(hara.hta.provider/dispatch __hta_arg_0 __hta_arg_1))",
        rich_hta::SOURCE
    );
    dispatch_eval_values(
        kernel,
        task,
        "ROOT",
        &source,
        Some(vec![
            Value::String(operation.to_owned()),
            Value::Vector(arguments.into()),
        ]),
    )
}

fn dispatch_sandbox_eval(
    kernel: &mut SessionKernel,
    task: u64,
    source: &str,
) -> Result<(), String> {
    if source.is_empty() {
        return Err("sandbox/eval requires non-empty source".into());
    }
    if source.len() > MAX_SANDBOX_SOURCE_BYTES {
        return Err(format!(
            "sandbox/source-limit: source exceeds {MAX_SANDBOX_SOURCE_BYTES} bytes"
        ));
    }
    let name = format!("__hta_sandbox_{task}");
    if kernel.sessions.contains_key(&name) {
        return Err(format!("sandbox/task-exists: {task}"));
    }
    kernel
        .sessions
        .insert(name.clone(), Session::sandbox(&name, kernel.events.clone()));
    kernel.sandbox_sessions.insert(name.clone());
    kernel.task_sessions.insert(task, name.clone());
    kernel
        .session_mut(&name)?
        .enqueue_source(task, source, Vec::new());
    Ok(())
}

#[cfg(feature = "evaluation-journal")]
fn journal_value(journal: &journal::Journal) -> Value {
    fn key(name: &str) -> Value {
        Value::Keyword(name.into())
    }
    fn option_number(value: Option<u64>) -> Value {
        value
            .map(|value| Value::Number(value as i64))
            .unwrap_or(Value::Nil)
    }
    fn preview(value: &journal::ValuePreview) -> Value {
        Value::Map(
            vec![
                (key("value/type"), Value::String(value.type_name.clone())),
                (key("value/display"), Value::String(value.display.clone())),
                (key("value/truncated"), Value::Bool(value.truncated)),
            ]
            .into_iter()
            .collect(),
        )
    }
    fn kind(value: &journal::JournalEventKind) -> &'static str {
        match value {
            journal::JournalEventKind::EvaluationStart => "evaluation/start",
            journal::JournalEventKind::MacroExpand => "macro/expand",
            journal::JournalEventKind::OperationEnter => "operation/enter",
            journal::JournalEventKind::OperationReturn => "operation/return",
            journal::JournalEventKind::Error => "evaluation/error",
            journal::JournalEventKind::JournalTruncated => "journal/truncated",
        }
    }
    fn status(value: &journal::JournalStatus) -> &'static str {
        match value {
            journal::JournalStatus::Ok => "ok",
            journal::JournalStatus::Error => "error",
            journal::JournalStatus::Truncated => "truncated",
        }
    }
    let events = journal
        .events
        .iter()
        .map(|event| {
            let mut fields = vec![
                (key("event/id"), Value::Number(event.id.0 as i64)),
                (key("event/sequence"), Value::Number(event.sequence as i64)),
                (key("event/kind"), key(kind(&event.kind))),
            ];
            if matches!(
                event.kind,
                journal::JournalEventKind::OperationEnter
                    | journal::JournalEventKind::OperationReturn
            ) {
                fields.extend([
                    (
                        key("operation/id"),
                        option_number(event.operation.map(|id| id.0)),
                    ),
                    (
                        key("operation/parent"),
                        option_number(event.parent_operation.map(|id| id.0)),
                    ),
                    (key("operation/depth"), Value::Number(event.depth as i64)),
                    (
                        key("operation/name"),
                        event
                            .function
                            .clone()
                            .map(Value::String)
                            .unwrap_or(Value::Nil),
                    ),
                ]);
            }
            match event.kind {
                journal::JournalEventKind::OperationEnter => fields.push((
                    key("operation/arguments"),
                    Value::Vector(event.values.iter().map(preview).collect::<Vec<_>>().into()),
                )),
                journal::JournalEventKind::OperationReturn => fields.push((
                    key("operation/result"),
                    event.values.first().map(preview).unwrap_or(Value::Nil),
                )),
                journal::JournalEventKind::Error => fields.push((
                    key("error/message"),
                    event
                        .message
                        .clone()
                        .map(Value::String)
                        .unwrap_or(Value::Nil),
                )),
                journal::JournalEventKind::JournalTruncated => fields.push((
                    key("truncation/reason"),
                    event
                        .message
                        .clone()
                        .map(Value::String)
                        .unwrap_or(Value::Nil),
                )),
                journal::JournalEventKind::MacroExpand => {
                    fields.push((
                        key("macro/name"),
                        event
                            .function
                            .clone()
                            .map(Value::String)
                            .unwrap_or(Value::Nil),
                    ));
                    fields.push((
                        key("macro/values"),
                        Value::Vector(event.values.iter().map(preview).collect::<Vec<_>>().into()),
                    ));
                }
                journal::JournalEventKind::EvaluationStart => {}
            }
            Value::Map(fields.into_iter().collect())
        })
        .collect::<Vec<_>>();
    Value::Map(
        vec![
            (key("journal/schema"), Value::String(journal.schema.into())),
            (
                key("journal/id"),
                Value::String(journal.journal_id.to_string()),
            ),
            (key("journal/status"), key(status(&journal.status))),
            (key("journal/events"), Value::Vector(events.into())),
            (
                key("journal/result"),
                journal.result.as_ref().map(preview).unwrap_or(Value::Nil),
            ),
            (
                key("journal/error"),
                journal
                    .error
                    .clone()
                    .map(Value::String)
                    .unwrap_or(Value::Nil),
            ),
        ]
        .into_iter()
        .collect(),
    )
}

fn dispatch_eval(
    kernel: &mut SessionKernel,
    task: u64,
    session: &str,
    args: &[Value],
    bound: bool,
) -> Result<(), String> {
    if bound {
        match args {
            [Value::String(source), Value::Vector(bindings)] => dispatch_eval_values(
                kernel,
                task,
                session,
                source,
                Some(bindings.iter().cloned().collect()),
            ),
            _ => Err("hta eval-bound expects a source string and binding vector".into()),
        }
    } else {
        match args {
            [Value::String(source)] => dispatch_eval_values(kernel, task, session, source, None),
            _ => Err("hta eval expects one source string".into()),
        }
    }
}

fn dispatch_eval_values(
    kernel: &mut SessionKernel,
    task: u64,
    session: &str,
    source: &str,
    bindings: Option<Vec<Value>>,
) -> Result<(), String> {
    validate_session_name(session)?;
    kernel.session(session)?;
    kernel.task_sessions.insert(task, session.into());
    let runtime = kernel.session_mut(session)?;
    runtime.enqueue_source(task, source, bindings.unwrap_or_default());
    Ok(())
}

fn dispatch_eval_vm(
    kernel: &mut SessionKernel,
    task: u64,
    session: &str,
    args: &[Value],
) -> Result<(), String> {
    match args {
        [Value::String(source)] => dispatch_eval_vm_values(kernel, task, session, source),
        _ => Err("hta eval-vm expects one source string".into()),
    }
}

fn dispatch_eval_vm_values(
    kernel: &mut SessionKernel,
    task: u64,
    session: &str,
    source: &str,
) -> Result<(), String> {
    #[cfg(feature = "bytecode-vm")]
    {
        validate_session_name(session)?;
        kernel.session(session)?;
        kernel.task_sessions.insert(task, session.into());
        kernel.session_mut(session)?.enqueue_vm(task, source);
        Ok(())
    }
    #[cfg(not(feature = "bytecode-vm"))]
    {
        let _ = (kernel, task, session, source);
        Err("VM_UNAVAILABLE".into())
    }
}

fn dispatch_eval_halc(
    kernel: &mut SessionKernel,
    task: u64,
    session: &str,
    args: &[Value],
) -> Result<(), String> {
    let [Value::Bytes(bytes)] = args else {
        return Err("hta eval-halc expects one byte array".into());
    };
    dispatch_eval_halc_bytes(kernel, task, session, bytes)
}

fn dispatch_eval_halc_bytes(
    kernel: &mut SessionKernel,
    task: u64,
    session: &str,
    bytes: &[u8],
) -> Result<(), String> {
    validate_session_name(session)?;
    kernel.session(session)?;
    kernel.task_sessions.insert(task, session.into());
    kernel
        .session_mut(session)?
        .enqueue_halc(task, vec![bytes.to_vec()]);
    Ok(())
}

fn dispatch_eval_halc_bundle(
    kernel: &mut SessionKernel,
    task: u64,
    session: &str,
    args: &[Value],
) -> Result<(), String> {
    let [Value::Vector(modules)] = args else {
        return Err("hta eval-halc-bundle expects one vector of byte arrays".into());
    };
    let modules = modules.iter().cloned().collect::<Vec<_>>();
    dispatch_eval_halc_bundle_values(kernel, task, session, &modules)
}

fn dispatch_eval_halc_bundle_values(
    kernel: &mut SessionKernel,
    task: u64,
    session: &str,
    modules: &[Value],
) -> Result<(), String> {
    let bytes = modules
        .iter()
        .map(|module| match module {
            Value::Bytes(bytes) => Ok(bytes.clone()),
            _ => Err("hta eval-halc-bundle expects byte arrays".to_owned()),
        })
        .collect::<Result<Vec<_>, _>>()?;
    validate_session_name(session)?;
    kernel.session(session)?;
    kernel.task_sessions.insert(task, session.into());
    kernel.session_mut(session)?.enqueue_halc(task, bytes);
    Ok(())
}

fn dispatch_complete(
    kernel: &mut SessionKernel,
    task: u64,
    session: &str,
    args: &[Value],
) -> Result<(), String> {
    match args {
        [Value::String(prefix)] => dispatch_complete_values(kernel, task, session, prefix),
        _ => Err("hta complete expects one prefix string".into()),
    }
}

fn dispatch_complete_values(
    kernel: &mut SessionKernel,
    task: u64,
    session: &str,
    prefix: &str,
) -> Result<(), String> {
    let value = kernel.session(session)?.complete(prefix);
    enqueue_event(&kernel.events, event(0, task, value));
    Ok(())
}
fn output(bytes: Vec<u8>) -> i64 {
    let size = bytes.len();
    let pointer = alloc(size);
    if pointer.is_null() {
        return 0;
    }
    unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), pointer, size) };
    ((pointer as u64) << 32 | size as u64) as i64
}
#[no_mangle]
pub extern "C" fn hta_next_event() -> i64 {
    KERNEL.with(|cell| {
        let mut kernel = cell.borrow_mut();
        kernel.drain_ready();
        let Some(bytes) = kernel.events.borrow_mut().pop_front() else {
            return 0;
        };
        if let Some(task) = terminal_task(&bytes) {
            kernel.cleanup_task(task);
        }
        output(bytes)
    })
}
#[no_mangle]
pub extern "C" fn hta_poll() -> i32 {
    KERNEL.with(|cell| {
        let mut kernel = cell.borrow_mut();
        kernel.drain_ready();
        let count = kernel.events.borrow().len() as i32;
        count
    })
}
#[no_mangle]
pub extern "C" fn hta_deliver(pointer: *const u8, size: usize) -> i32 {
    let bytes = if pointer.is_null() {
        &[][..]
    } else {
        unsafe { std::slice::from_raw_parts(pointer, size) }
    };
    KERNEL.with(|cell| {
        let mut kernel = cell.borrow_mut();
        let values = match hta::decode(bytes) {
            Ok(Value::Vector(values)) if values.len() == 3 => values,
            _ => return 1,
        };
        let call = match values[0] {
            Value::Number(v) if v >= 0 => v as u64,
            _ => return 1,
        };
        let state = match values[1] {
            Value::Number(v) => v,
            _ => return 1,
        };
        let payload = values[2].clone();
        let Some(runtime) = kernel
            .sessions
            .values_mut()
            .find(|runtime| runtime.calls.contains_key(&call))
        else {
            return 2;
        };
        let Some((_task, promise)) = runtime.calls.remove(&call) else {
            return 2;
        };
        if state == 0 {
            promise.resolve(payload);
        } else {
            promise.reject(match payload {
                Value::String(v) => v,
                v => v.display(),
            });
        }
        kernel.drain_ready();
        0
    })
}
#[no_mangle]
pub extern "C" fn hta_cancel(task: i64) -> i32 {
    KERNEL.with(|cell| {
        let mut kernel = cell.borrow_mut();
        let task = task as u64;
        let Some(session) = kernel.task_sessions.get(&task).cloned() else {
            return 1;
        };
        let Some(runtime) = kernel.sessions.get_mut(&session) else {
            return 1;
        };
        if let Some(position) = runtime
            .evaluation_queue
            .iter()
            .position(|request| request.task() == task)
        {
            runtime.evaluation_queue.remove(position);
            runtime.event(event(1, task, PromiseRejection::cancelled().value()));
            return 0;
        }
        runtime.calls.retain(|_, (owner, _)| *owner != task);
        if let Some(mut fiber) = runtime.fibers.remove(&task) {
            fiber.cancel();
            runtime.event(event(1, task, PromiseRejection::cancelled().value()));
            runtime.finish_evaluation(task);
            return 0;
        }
        #[cfg(feature = "bytecode-vm")]
        if let Some(mut fiber) = runtime.vm_fibers.remove(&task) {
            fiber.cancel();
            runtime.event(event(1, task, PromiseRejection::cancelled().value()));
            runtime.finish_evaluation(task);
            return 0;
        }
        if let Some(promise) = runtime.tasks.remove(&task) {
            promise.cancel();
            return 0;
        }
        1
    })
}
#[no_mangle]
pub extern "C" fn hta_drop_task(task: i64) -> i32 {
    KERNEL.with(|kernel| {
        let mut kernel = kernel.borrow_mut();
        let task = task as u64;
        if let Some(session) = kernel.task_sessions.remove(&task) {
            let sandbox = kernel.sandbox_sessions.remove(&session);
            if let Some(runtime) = kernel.sessions.get_mut(&session) {
                if let Some(mut fiber) = runtime.fibers.remove(&task) {
                    fiber.cancel();
                }
                #[cfg(feature = "bytecode-vm")]
                if let Some(mut fiber) = runtime.vm_fibers.remove(&task) {
                    fiber.cancel();
                }
                runtime.tasks.remove(&task);
                runtime.calls.retain(|_, (owner, _)| *owner != task);
                runtime
                    .evaluation_queue
                    .retain(|request| request.task() != task);
                runtime.finish_evaluation(task);
            }
            if sandbox {
                kernel.sessions.remove(&session);
            }
        }
        0
    })
}
fn source_text(source_ptr: *const u8, source_len: usize) -> Result<&'static str, i32> {
    if source_ptr.is_null() {
        return Err(1);
    }
    let bytes = unsafe { std::slice::from_raw_parts(source_ptr, source_len) };
    std::str::from_utf8(bytes).map_err(|_| 1)
}

fn error_code(error: &str) -> i32 {
    let message = error.to_ascii_lowercase();
    if message.contains("division by zero") {
        return 5;
    }
    if message.contains("unbound symbol") || message.contains("unbound var") {
        return 2;
    }
    if message.contains("arity")
        || message.contains("at least")
        || message.contains("argument") && message.contains("expects")
    {
        return 3;
    }
    if message.contains("index") || message.contains("out of bounds") {
        return 6;
    }
    if message.contains("unknown") || message.contains("unsupported") {
        return 7;
    }
    if message.contains("parse") || message.contains("unexpected") || message.contains("unclosed") {
        return 1;
    }
    4
}

fn evaluate(source: &str) -> Result<i64, i32> {
    kernel::parse_forms(source).map_err(|_| 1)?;

    let mut runtime = Session::new();
    runtime
        .start_fiber(1, source)
        .map_err(|error| error_code(&error))?;
    let frame = runtime.events.borrow_mut().pop_front().ok_or(4)?;
    let value = hta::decode(&frame).map_err(|_| 4)?;
    let Value::Vector(values) = value else {
        return Err(4);
    };
    if values.len() != 3 {
        return Err(4);
    }
    match (&values[0], &values[2]) {
        (Value::Number(kind), Value::Number(value)) if *kind == 0 => Ok(*value),
        (Value::Number(kind), error) if *kind == 1 => Err(error_code(&error.display())),
        _ => Err(4),
    }
}

#[no_mangle]
pub extern "C" fn eval_i64(source_ptr: *const u8, source_len: usize) -> i64 {
    match source_text(source_ptr, source_len).and_then(evaluate) {
        Ok(value) => value,
        Err(_) => i64::MIN,
    }
}

/// Returns zero for a successful evaluation, otherwise a stable core.v1 error code.
#[no_mangle]
pub extern "C" fn eval_error_code(source_ptr: *const u8, source_len: usize) -> i32 {
    match source_text(source_ptr, source_len) {
        Ok(source) => evaluate(source).map(|_| 0).unwrap_or_else(|code| code),
        Err(code) => code,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        dispatch, emit_settlement, eval_error_code, evaluate, terminal_task, Session,
        SessionKernel, MAX_SANDBOX_SOURCE_BYTES,
    };
    use hara_runtime::core::{PromiseRejection, PromiseState, Value};
    use hara_runtime::lang::data::Symbol;
    use hara_runtime::lang::protocol::IDeref;
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::rc::Rc;

    fn result(kernel: &mut SessionKernel) -> Vec<Value> {
        kernel.drain_ready();
        let bytes = kernel
            .events
            .borrow_mut()
            .pop_front()
            .expect("result event");
        match hara_runtime::hta::decode(&bytes).expect("valid HTA event") {
            Value::Vector(values) => values.iter().cloned().collect(),
            value => panic!("expected result vector, got {}", value.display()),
        }
    }

    #[test]
    fn kernel_sessions_isolate_namespaces_in_one_runtime() {
        let mut kernel = SessionKernel::new();
        kernel.create_session("alpha").unwrap();
        kernel.create_session("beta").unwrap();

        dispatch(
            &mut kernel,
            1,
            "session/eval",
            vec![
                Value::String("alpha".into()),
                Value::String("(def answer 41) (+ answer 1)".into()),
            ],
        )
        .unwrap();
        assert!(matches!(
            result(&mut kernel).as_slice(),
            [Value::Number(0), Value::Number(1), Value::Number(42)]
        ));

        dispatch(
            &mut kernel,
            2,
            "session/eval",
            vec![
                Value::String("beta".into()),
                Value::String("(def answer 6) (* answer 7)".into()),
            ],
        )
        .unwrap();
        assert!(matches!(
            result(&mut kernel).as_slice(),
            [Value::Number(0), Value::Number(2), Value::Number(42)]
        ));

        dispatch(
            &mut kernel,
            3,
            "session/eval",
            vec![
                Value::String("alpha".into()),
                Value::String("answer".into()),
            ],
        )
        .unwrap();
        assert!(matches!(
            result(&mut kernel).as_slice(),
            [Value::Number(0), Value::Number(3), Value::Number(41)]
        ));
    }

    #[cfg(feature = "rich-hta")]
    #[test]
    fn rich_hta_fixture_crosses_the_host_boundary() {
        let mut kernel = SessionKernel::new();
        dispatch(&mut kernel, 1, "describe", Vec::new()).unwrap();
        let call_event = result(&mut kernel);
        let call = match call_event.as_slice() {
            [
                Value::Number(2),
                Value::Number(call),
                Value::Number(1),
                Value::String(session),
                Value::Nil,
                Value::String(service),
                Value::String(method),
                Value::Vector(arguments),
            ] => {
                assert_eq!(session, "ROOT");
                assert_eq!(service, "fixture.provider");
                assert_eq!(method, "describe");
                assert!(arguments.is_empty());
                *call as u64
            }
            other => panic!("expected rich HTA host call, got {other:?}"),
        };
        kernel
            .session_mut("ROOT")
            .unwrap()
            .calls
            .remove(&call)
            .unwrap()
            .1
            .resolve(Value::Map(
                vec![(
                    Value::Keyword("provider".into()),
                    Value::String("fixture".into()),
                )]
                .into_iter()
                .collect(),
            ));
        let response = result(&mut kernel);
        assert!(matches!(
            response.as_slice(),
            [Value::Number(0), Value::Number(1), Value::Map(values)]
                if values.get(&Value::Keyword("provider".into()))
                    == Some(&Value::String("fixture".into()))
        ));
    }

    #[cfg(feature = "bytecode-vm")]
    #[test]
    fn explicit_vm_target_matches_the_evaluator_without_fallback() {
        let cases = [
            "(+ 19 23)",
            "(let [x 6 y 7] (* x y))",
            "(loop [i 0 total 0] (if (< i 7) (recur (+ i 1) (+ total i)) total))",
        ];
        for (index, source) in cases.into_iter().enumerate() {
            let evaluator_task = (index * 2 + 1) as u64;
            let vm_task = evaluator_task + 1;
            let mut evaluator = SessionKernel::new();
            dispatch(
                &mut evaluator,
                evaluator_task,
                "eval",
                vec![Value::String(source.into())],
            )
            .unwrap();
            let expected = result(&mut evaluator)[2].clone();

            let mut vm = SessionKernel::new();
            dispatch(
                &mut vm,
                vm_task,
                "eval-vm",
                vec![Value::String(source.into())],
            )
            .unwrap();
            assert_eq!(result(&mut vm)[2], expected, "{source}");
        }

        let mut kernel = SessionKernel::new();
        dispatch(
            &mut kernel,
            20,
            "eval-vm",
            vec![Value::String("(require [unsupported.vm.module])".into())],
        )
        .unwrap();
        let failure = result(&mut kernel);
        assert_eq!(failure[0], Value::Number(1));
        assert_eq!(failure[1], Value::Number(20));
    }

    #[cfg(feature = "bytecode-vm")]
    #[test]
    fn prepared_vm_programs_compile_once_and_invoke_repeatedly() {
        let mut kernel = SessionKernel::new();
        kernel.create_session("prepared").unwrap();
        dispatch(
            &mut kernel,
            1,
            "session/prepare-vm",
            vec![
                Value::String("prepared".into()),
                Value::String(
                    "(loop [i 0 total 0] (if (< i 7) (recur (+ i 1) (+ total i)) total))".into(),
                ),
            ],
        )
        .unwrap();
        let prepared = result(&mut kernel);
        let Value::Number(program) = prepared[2] else {
            panic!("expected prepared program id")
        };
        for task in 2..=4 {
            dispatch(
                &mut kernel,
                task,
                "session/invoke-vm",
                vec![Value::String("prepared".into()), Value::Number(program)],
            )
            .unwrap();
            assert_eq!(result(&mut kernel)[2], Value::Number(21));
        }
    }

    #[test]
    fn filesystem_reattachment_preserves_idle_session_state() {
        let mut kernel = SessionKernel::new();
        kernel.create_session("example").unwrap();
        dispatch(
            &mut kernel,
            1,
            "session/eval",
            vec![
                Value::String("example".into()),
                Value::String("(def stale-value 42)".into()),
            ],
        )
        .unwrap();
        result(&mut kernel);

        let mount_id = kernel
            .create_filesystem(&Value::Map(
                vec![(
                    Value::Keyword("provider".into()),
                    Value::String("memory".into()),
                )]
                .into_iter()
                .collect(),
            ))
            .unwrap();
        kernel.attach_filesystem("example", mount_id).unwrap();
        assert_eq!(
            kernel
                .session("example")
                .unwrap()
                .complete("stale")
                .display(),
            "[\"stale-value\"]"
        );
        assert_eq!(kernel.session("example").unwrap().mount_id, Some(mount_id));
        assert_eq!(
            kernel.close_filesystem(mount_id).unwrap_err(),
            format!("FILESYSTEM_ATTACHED {mount_id}")
        );
        kernel.detach_filesystem("example").unwrap();
        kernel.close_filesystem(mount_id).unwrap();
    }

    #[test]
    fn filesystem_reattachment_rejects_busy_session() {
        let mut kernel = SessionKernel::new();
        kernel.create_session("busy").unwrap();
        dispatch(
            &mut kernel,
            1,
            "session/eval",
            vec![
                Value::String("busy".into()),
                Value::String("(deref (std.native.Host/call \"wait\" \"forever\" []))".into()),
            ],
        )
        .unwrap();
        let mount_id = kernel
            .create_filesystem(&Value::Map(
                vec![(
                    Value::Keyword("provider".into()),
                    Value::String("memory".into()),
                )]
                .into_iter()
                .collect(),
            ))
            .unwrap();
        assert_eq!(
            kernel.attach_filesystem("busy", mount_id).unwrap_err(),
            "SESSION_BUSY busy"
        );
    }

    #[test]
    fn session_evaluations_are_serialized_in_submission_order() {
        let mut kernel = SessionKernel::new();
        kernel.create_session("serial").unwrap();
        dispatch(
            &mut kernel,
            1,
            "session/eval",
            vec![
                Value::String("serial".into()),
                Value::String(
                    "(do (def queued-answer 41) \
                     (deref (std.native.Host/call \"wait\" \"once\" [])))"
                        .into(),
                ),
            ],
        )
        .unwrap();
        dispatch(
            &mut kernel,
            2,
            "session/eval",
            vec![
                Value::String("serial".into()),
                Value::String("(+ queued-answer 1)".into()),
            ],
        )
        .unwrap();

        assert_eq!(kernel.session("serial").unwrap().evaluation_queue.len(), 1);
        let call_event = result(&mut kernel);
        let call = match call_event.as_slice() {
            [Value::Number(2), Value::Number(call), ..] => *call as u64,
            other => panic!("expected host call, got {other:?}"),
        };
        let promise = kernel
            .session_mut("serial")
            .unwrap()
            .calls
            .remove(&call)
            .unwrap()
            .1;
        promise.resolve(Value::Nil);

        assert!(matches!(
            result(&mut kernel).as_slice(),
            [Value::Number(0), Value::Number(1), Value::Nil]
        ));
        assert!(matches!(
            result(&mut kernel).as_slice(),
            [Value::Number(0), Value::Number(2), Value::Number(42)]
        ));
    }

    #[test]
    fn mounted_file_calls_use_the_canonical_hal_module_and_v2_identity() {
        let mut kernel = SessionKernel::new();
        kernel.create_session("files").unwrap();
        let mount_id = kernel
            .create_filesystem(&Value::Map(
                vec![(
                    Value::Keyword("provider".into()),
                    Value::String("memory".into()),
                )]
                .into_iter()
                .collect(),
            ))
            .unwrap();
        kernel.attach_filesystem("files", mount_id).unwrap();
        dispatch(
            &mut kernel,
            1,
            "session/eval",
            vec![
                Value::String("files".into()),
                Value::String("(std.native.File/write \"/note.bin\" (bytes 1 2 3))".into()),
            ],
        )
        .unwrap();
        let call = result(&mut kernel);
        assert!(matches!(
            call.as_slice(),
            [
                Value::Number(2),
                Value::Number(_),
                Value::Number(1),
                Value::String(session),
                Value::Number(found_mount),
                Value::String(service),
                Value::String(method),
                Value::Vector(_)
            ] if session == "files"
                && *found_mount == mount_id as i64
                && service == "file"
                && method == "write"
        ));
    }

    #[test]
    fn parser_failures_have_the_stable_parse_code() {
        for source in [")", "[1", "123a", "\"unterminated"] {
            assert_eq!(evaluate(source), Err(1), "{source}");
            assert_eq!(
                eval_error_code(source.as_ptr(), source.len()),
                1,
                "{source}"
            );
        }
        assert_eq!(eval_error_code(b"(+ 1 2)".as_ptr(), 7), 0);
    }

    #[test]
    fn portable_type_descriptors_are_available_in_raw_wasm() {
        for source in [
            "(if (= (type nil) :std.native.Nil) 42 0)",
            "(if (= (type :key) :std.native.Keyword) 42 0)",
            "(if (= (type (symbol \"hara/name\")) :std.native.Symbol) 42 0)",
            "(if (= (type []) :std.native.Tuple) 42 0)",
            "(if (= (type (vector)) :std.native.Vector) 42 0)",
            "(if (= (type {}) :std.native.HashMap) 42 0)",
            "(if (= (type (ns-create (quote example))) :std.native.Namespace) 42 0)",
            "(if (= [(tuple? []) (tuple? [1 2 3 4 5 6 7 8 9])] [true false]) 42 0)",
            "(if (map? {}) 42 0)",
            "(if (set? #{}) 42 0)",
            "(if (atom? (atom nil)) 42 0)",
        ] {
            assert_eq!(evaluate(source), Ok(42), "{source}");
        }
    }

    #[test]
    fn iterator_lifecycle_matches_native_core_in_raw_wasm() {
        for source in [
            "(let (it (Iter/iter-cycle [1 2])) (do (iter-next it) (Iter/iter-close it) (if (iter-next? it) 0 42)))",
            "(let (it (Iter/iter-zip [1 2] [3 4])) (do (Iter/iter-close it) (if (iter-next? it) 0 42)))",
            "(let (it (Iter/iter-map (fn [x] x) [1 2])) (do (Iter/iter-close it) (if (iter-next? it) 0 42)))",
        ] {
            assert_eq!(evaluate(source), Ok(42), "{source}");
        }
        assert_eq!(
            evaluate("(iter-next (Iter/iter-map (fn [x] (+ x 1)) [0]))"),
            Ok(1)
        );
    }

    fn completion_value(runtime: &mut Session, task: u64) -> hara_runtime::core::Value {
        let frame = runtime
            .events
            .borrow_mut()
            .pop_front()
            .expect("completion event");
        match super::hta::decode(&frame).unwrap() {
            hara_runtime::core::Value::Vector(values) => {
                assert_eq!(
                    values[0],
                    hara_runtime::core::Value::Number(0),
                    "eval failed for task {}: {}",
                    values[1].display(),
                    values[2].display()
                );
                assert_eq!(values[1], hara_runtime::core::Value::Number(task as i64));
                values[2].clone()
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn require_loads_registered_resource_and_binds_alias() {
        let mut runtime = Session::new();
        runtime.resources.borrow_mut().insert(
            "chrome.api".to_string(),
            "(ns chrome.api) (defn answer [] 42)".to_string(),
        );
        runtime
            .start_fiber(1, "(require [chrome.api :as api]) (api/answer)")
            .unwrap();
        assert_eq!(
            completion_value(&mut runtime, 1),
            hara_runtime::core::Value::Number(42)
        );
    }

    #[test]
    fn raw_completion_preserves_public_priority_and_deterministic_helpers() {
        let mut runtime = Session::new();
        let source = "(def zebra-helper 1) ".to_owned()
            + "(def ^{:public true} recommended-api 2) "
            + "(def alpha-helper 3) "
            + "(def ^{:public true} advertised-api 4)";
        runtime.start_fiber(1, &source).unwrap();
        completion_value(&mut runtime, 1);
        let Value::Vector(symbols) = runtime.complete("") else {
            panic!("expected completion vector")
        };
        let symbols = symbols
            .iter()
            .map(|value| match value {
                Value::String(value) => value.clone(),
                other => other.display(),
            })
            .collect::<Vec<_>>();
        let position = |name: &str| symbols.iter().position(|value| value == name).unwrap();
        assert!(position("advertised-api") < position("recommended-api"));
        assert!(position("recommended-api") < position("alpha-helper"));
        assert!(position("alpha-helper") < position("zebra-helper"));
    }

    #[test]
    fn foundation_aliases_load_without_require_in_fresh_sessions() {
        let source_aliases = [
            "str/trim",
            "promise/from",
            "bytes/count",
            "co/create",
            "pretty/render",
        ];
        for (index, probe) in source_aliases.into_iter().enumerate() {
            let mut kernel = SessionKernel::new();
            let task = index as u64 + 1;
            dispatch(
                &mut kernel,
                task,
                "eval",
                vec![Value::String(format!("(nil? (resolve '{probe}))"))],
            )
            .unwrap();
            assert!(
                matches!(
                    result(&mut kernel).as_slice(),
                    [Value::Number(0), Value::Number(found_task), Value::Bool(false)] if *found_task == task as i64
                ),
                "{probe} should resolve through its built-in alias"
            );
        }
        for probe in [
            "socket/connect",
            "file/resolve",
            "edn/read",
            "json/read",
            "algo/deque",
            "host/call",
            "kernel/session-list",
            "os/platform",
            "crypto/sha256",
        ] {
            let mut kernel = SessionKernel::new();
            dispatch(
                &mut kernel,
                1,
                "eval",
                vec![Value::String(format!("(nil? (resolve '{probe}))"))],
            )
            .unwrap();
            assert!(
                matches!(
                    result(&mut kernel).as_slice(),
                    [Value::Number(0), Value::Number(1), Value::Bool(true)]
                ),
                "{probe} should not resolve through a runtime compatibility alias"
            );
        }
    }

    #[test]
    fn string_alias_evaluates_in_root_named_and_declared_namespaces() {
        for source in [
            "(str/trim \"  Hara  \")",
            "(ns docs.example) (str/trim \"  Hara  \")",
        ] {
            let mut kernel = SessionKernel::new();
            dispatch(&mut kernel, 1, "eval", vec![Value::String(source.into())]).unwrap();
            assert!(
                matches!(
                    result(&mut kernel).as_slice(),
                    [Value::Number(0), Value::Number(1), Value::String(value)] if value == "Hara"
                ),
                "{source}"
            );
        }

        let mut kernel = SessionKernel::new();
        kernel.create_session("lesson").unwrap();
        dispatch(
            &mut kernel,
            1,
            "session/eval",
            vec![
                Value::String("lesson".into()),
                Value::String("(str/trim \"  Hara  \")".into()),
            ],
        )
        .unwrap();
        assert!(matches!(
            result(&mut kernel).as_slice(),
            [Value::Number(0), Value::Number(1), Value::String(value)] if value == "Hara"
        ));
    }

    #[test]
    fn require_supports_ns_form_clauses_and_qualified_access() {
        let mut runtime = Session::new();
        runtime.resources.borrow_mut().insert(
            "acme.tools".to_string(),
            "(ns acme.tools) (defn seven [] 7)".to_string(),
        );
        runtime
            .start_fiber(
                2,
                "(ns demo (:require [acme.tools :as tools])) (tools/seven)",
            )
            .unwrap();
        assert_eq!(
            completion_value(&mut runtime, 2),
            hara_runtime::core::Value::Number(7)
        );
        runtime.start_fiber(3, "(acme.tools/seven)").unwrap();
        assert_eq!(
            completion_value(&mut runtime, 3),
            hara_runtime::core::Value::Number(7)
        );
    }

    #[test]
    fn fibers_preserve_guest_protocol_extensions() {
        let mut runtime = Session::new();
        runtime
            .start_fiber(
                1,
                "(defstruct Box [value]) (defprotocol ReadBox (read-box [self])) \
                 (extend-type Box ReadBox (read-box [self] (:value self))) :ok",
            )
            .unwrap();
        assert!(matches!(
            completion_value(&mut runtime, 1),
            Value::Keyword(_)
        ));

        runtime.start_fiber(2, "(read-box (Box 42))").unwrap();
        assert_eq!(completion_value(&mut runtime, 2), Value::Number(42));
    }

    #[test]
    fn bound_fibers_receive_hta_values_without_serializing_source() {
        let mut runtime = Session::new();
        runtime
            .start_fiber_with_bindings(
                1,
                "(get __hta_arg_0 :answer)",
                vec![Value::Map(
                    vec![(Value::Keyword("answer".into()), Value::Number(42))]
                        .into_iter()
                        .collect(),
                )],
            )
            .unwrap();
        assert_eq!(completion_value(&mut runtime, 1), Value::Number(42));
    }

    #[test]
    fn raw_fibers_preserve_exception_provenance() {
        let mut runtime = Session::new();
        runtime
            .start_fiber(
                1,
                "(let [error (ex :app/provenance {})] \
                   (try (throw error) \
                     (catch caught \
                       (let [provenance (ex-provenance caught)] \
                         (if (and (:ex/created-at provenance) \
                                  (= 1 (count (:ex/throws provenance)))) \
                           42 0)))))",
            )
            .unwrap();
        assert_eq!(completion_value(&mut runtime, 1), Value::Number(42));

        runtime
            .start_fiber(
                2,
                "(let [error (ex :app/provenance {})] \
                   (try (try (throw error) (catch caught (throw caught))) \
                     (catch caught \
                       (if (= 2 (count (:ex/throws (ex-provenance caught)))) 42 0))))",
            )
            .unwrap();
        assert_eq!(completion_value(&mut runtime, 2), Value::Number(42));
    }

    #[test]
    fn raw_kernels_expose_the_foundation_data_namespaces() {
        let declarations = hara_runtime::core::native_declarations();
        assert!(!declarations.is_empty());
        let mut runtime = Session::new();
        assert!(runtime.env.contains_key("std.native.Edn/write"));
        assert!(runtime
            .env
            .contains_key("std.protocol.icount.ICount"));
        for declaration in declarations {
            assert!(
                runtime
                    .env
                    .contains_key(&format!("std.native.{}", declaration.name)),
                "std.native.{}",
                declaration.name
            );
            for method in declaration.methods {
                assert!(
                    runtime.env.contains_key(&format!(
                        "std.native.{}/{}",
                        declaration.name, method
                    )),
                    "std.native.{}/{}",
                    declaration.name,
                    method
                );
            }
        }
        runtime
            .start_fiber(1,             "(ns example.json) (std.native.Json/write {\"answer\" 42})")
            .unwrap();
        assert_eq!(
            completion_value(&mut runtime, 1),
            Value::String("{\"answer\":42}".into())
        );
        runtime
            .start_fiber(2,             "(std.native.Edn/read \"{:answer 42}\")")
            .unwrap();
        assert_eq!(completion_value(&mut runtime, 2).display(), "{:answer 42}");
        runtime
            .start_fiber(3,             "(std.native.Json/pretty {\"answer\" 42} {})")
            .unwrap();
        assert_eq!(
            completion_value(&mut runtime, 3),
            Value::String("{\n  \"answer\": 42\n}".into())
        );
        runtime
            .start_fiber(4,             "(std.native.Edn/pretty {:answer 42} {})")
            .unwrap();
        assert_eq!(
            completion_value(&mut runtime, 4),
            Value::String("{:answer 42}".into())
        );
        runtime
            .start_fiber(
                5,
                "(try \
                   (throw (ex-info \"bad input\" {:kind :invalid})) \
                   (catch Throwable error \
                     [(ex-message error) \
                      (ex-data error)]))",
            )
            .unwrap();
        assert_eq!(
            completion_value(&mut runtime, 5).display(),
            "[\"bad input\" {:kind :invalid}]"
        );
        runtime
            .start_fiber(6, "(std.native.Edn/write {:answer 42})")
            .unwrap();
        assert_eq!(
            completion_value(&mut runtime, 6),
            Value::String("{:answer 42}".into())
        );
        runtime
            .start_fiber(7, "(= std.native.Maths std.native.Maths)")
            .unwrap();
        assert_eq!(completion_value(&mut runtime, 7), Value::Bool(true));
        runtime
            .start_fiber(
                8,
                "[ (= std.native.Edn std.native.Edn) \
                  (= std.native.Json std.native.Json) \
                  (= std.native.Maths std.native.Maths)]",
            )
            .unwrap();
        assert_eq!(
            completion_value(&mut runtime, 8).display(),
            "[true true true]"
        );
        runtime
            .start_fiber(9, "(std.protocol.icount.ICount/count [1 2 3])")
            .unwrap();
        assert_eq!(completion_value(&mut runtime, 9), Value::Number(3));
    }

    #[test]
    fn raw_kernel_keeps_the_three_bang_name_compatibility_operations() {
        let mut runtime = Session::new();
        runtime
            .start_fiber(
                1,
                "(let (reference (atom 1)) \
                   [(reset! reference 2) \
                    (cas! reference 2 3) \
                    (cas! reference 2 4) \
                    (swap! reference (fn [value amount] (+ value amount)) 39) \
                    (deref reference)])",
            )
            .unwrap();
        assert_eq!(
            completion_value(&mut runtime, 1).display(),
            "[2 true false 42 42]"
        );
    }

    #[test]
    fn raw_kernels_run_the_shared_substrate_frame_fixture() {
        let mut runtime = Session::new();
        runtime.resources.borrow_mut().extend(
            super::SUBSTRATE_RESOURCES
                .iter()
                .map(|(name, source)| ((*name).into(), (*source).into())),
        );
        runtime
            .start_fiber(
                1,
                include_str!("../../../lib/test-fixtures/std/substrate/frame_conformance.hal"),
            )
            .unwrap();
        assert_eq!(completion_value(&mut runtime, 1).display(),
            "[\"substrate.v1\" \"request\" \"req-1\" \"client/a\" \"server/b\" \"workspace/main\" {\"trace\" \"trace-1\"} \"math/add\" [19 23] nil nil nil nil nil nil]");
    }

    #[test]
    fn raw_kernels_run_atom_backed_substrate_request_stream_and_cancellation_lifecycle() {
        std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn(|| {
                let mut runtime = Session::new();
                runtime.resources.borrow_mut().extend(
                    super::SUBSTRATE_RESOURCES
                        .iter()
                        .map(|(name, source)| ((*name).into(), (*source).into())),
                );
                runtime
                    .start_fiber(
                        1,
                        include_str!(
                            "../../../lib/test-fixtures/std/substrate/node_lifecycle_conformance.hal"
                        ),
                    )
                    .unwrap();
                assert_eq!(
                    completion_value(&mut runtime, 1),
                    Value::Vector(
                        vec![
                            Value::Number(84),
                            Value::Number(42),
                            Value::Keyword("rejected".into()),
                        ]
                        .into()
                    ),
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn require_missing_namespace_is_a_clean_error() {
        let mut runtime = Session::new();
        runtime.start_fiber(4, "(require [no.such.ns])").unwrap();
        let frame = runtime
            .events
            .borrow_mut()
            .pop_front()
            .expect("error event");
        match super::hta::decode(&frame).unwrap() {
            hara_runtime::core::Value::Vector(values) => {
                assert_eq!(
                    values[0],
                    hara_runtime::core::Value::Number(1),
                    "expected failure"
                );
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn structured_promise_rejections_survive_hta_settlement() {
        let events = Rc::new(RefCell::new(VecDeque::new()));
        let rejection = Value::Map(
            vec![
                (
                    Value::Keyword("ex/code".into()),
                    Value::Keyword("host/unavailable".into()),
                ),
                (
                    Value::Keyword("ex/class".into()),
                    Value::Keyword("ex.class/host".into()),
                ),
            ]
            .into_iter()
            .collect(),
        );
        emit_settlement(
            &events,
            7,
            PromiseState::Rejected(PromiseRejection::Value(rejection.clone())),
        );
        let frame = events.borrow_mut().pop_front().expect("rejection event");
        let Value::Vector(values) = hara_runtime::hta::decode(&frame).expect("valid HTA event")
        else {
            panic!("expected rejection vector")
        };
        assert_eq!(values[0], Value::Number(1));
        assert_eq!(values[1], Value::Number(7));
        assert_eq!(values[2], rejection);
    }

    #[test]
    fn fibers_persist_namespace_selection_defs_and_var_identity() {
        let mut runtime = Session::new();
        runtime
            .start_fiber(1, "(ns example.lib) (def answer 42)")
            .unwrap();

        assert_eq!(runtime.namespaces.current().name().as_str(), "example.lib");
        let namespace = runtime.namespaces.find("example.lib").unwrap();
        let answer = namespace.resolve(&Symbol::parse("answer")).unwrap();
        assert_eq!(answer.symbol().as_str(), "example.lib/answer");
        assert_eq!(answer.deref(), Value::Number(42));
        assert!(
            matches!(runtime.env.get("answer"), Some(Value::Var(var)) if var.same_identity(&answer))
        );

        runtime.start_fiber(2, "(ns user) (def local 7)").unwrap();
        assert_eq!(runtime.namespaces.current().name().as_str(), "user");
        assert_eq!(answer.deref(), Value::Number(42));
        assert!(runtime
            .namespaces
            .find("example.lib")
            .unwrap()
            .resolve(&Symbol::parse("answer"))
            .unwrap()
            .same_identity(&answer));
    }

    #[test]
    fn production_foundation_halc_bootstraps_macros_and_builtin_aliases_when_available() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../java/target/classes/std/foundation.halc");
        let Ok(bytes) = std::fs::read(path) else {
            return;
        };
        let mut runtime = Session::new();
        runtime.start_halc_fiber(1, &bytes).unwrap();
        assert_eq!(completion_value(&mut runtime, 1), Value::Bool(true));
        runtime
            .start_fiber(2, "(ns user) (str/trim \"  hara  \" )")
            .unwrap();
        assert_eq!(
            completion_value(&mut runtime, 2),
            Value::String("hara".into())
        );
    }

    #[test]
    fn restricted_sandbox_fences_native_surfaces_and_host_calls() {
        let events = Rc::new(RefCell::new(VecDeque::new()));
        let mut runtime = Session::sandbox("sandbox", events);
        for native_type in super::SANDBOX_FORBIDDEN_NATIVE_TYPES {
            let qualified = format!("std.native.{native_type}");
            assert!(runtime.namespaces.find(&qualified).is_none(), "{qualified}");
            assert!(!runtime.env.contains_key(*native_type), "{native_type}");
            assert!(!runtime.env.contains_key(&qualified), "{qualified}");
            assert!(
                runtime
                    .namespaces
                    .find("user")
                    .unwrap()
                    .resolve(&Symbol::parse(native_type))
                    .is_none(),
                "{native_type}"
            );
            let declaration = hara_runtime::core::native_declarations()
                .iter()
                .find(|declaration| declaration.name == *native_type)
                .unwrap();
            for method in declaration.methods {
                for symbol in [
                    format!("{native_type}/{method}"),
                    format!("{qualified}/{method}"),
                ] {
                    assert!(
                        runtime
                            .namespaces
                            .find("user")
                            .unwrap()
                            .resolve(&Symbol::parse(&symbol))
                            .is_none(),
                        "{symbol}"
                    );
                }
            }
        }
        assert!(runtime.namespaces.find("std.native.String").is_some());
        runtime
            .start_fiber(
                1,
                "(try (Host/call \"service\" \"method\" []) (catch error :unreachable))",
            )
            .unwrap();
        assert!(matches!(
            completion_value(&mut runtime, 1),
            Value::Keyword(value) if value.as_str() == "unreachable"
        ));
        runtime.start_fiber(2, "(+ 40 2)").unwrap();
        assert_eq!(completion_value(&mut runtime, 2), Value::Number(42));
        runtime
            .start_fiber(
                3,
                "(and (nil? (resolve 'Runtime)) \
                      (nil? (resolve 'std.native.Runtime/resolve)) \
                      (nil? (resolve 'Host/call)) \
                      (nil? (Base/resolve 'File/read)))",
            )
            .unwrap();
        assert_eq!(completion_value(&mut runtime, 3), Value::Bool(true));
    }

    #[test]
    fn sandbox_eval_uses_one_private_session_per_task_and_cleans_it_up() {
        let mut kernel = SessionKernel::new();
        for (task, expected) in [(1_u64, 42_i64), (2, 7)] {
            dispatch(
                &mut kernel,
                task,
                "sandbox/eval",
                vec![Value::String(if task == 1 {
                    "(+ 40 2)".into()
                } else {
                    "(+ 3 4)".into()
                })],
            )
            .unwrap();
            let session = kernel.task_sessions.get(&task).cloned().unwrap();
            assert!(session.starts_with("__hta_sandbox_"));
            assert!(kernel.sandbox_sessions.contains(&session));
            let bytes = kernel.events.borrow_mut().pop_front().unwrap();
            let value = hara_runtime::hta::decode(&bytes).unwrap();
            let Value::Vector(values) = value else {
                panic!("expected sandbox result event");
            };
            assert_eq!(
                values.iter().cloned().collect::<Vec<_>>(),
                vec![
                    Value::Number(0),
                    Value::Number(task as i64),
                    Value::Number(expected)
                ]
            );
            assert!(terminal_task(&bytes).is_some());
            kernel.cleanup_task(task);
            assert!(!kernel.task_sessions.contains_key(&task));
            assert!(!kernel.sessions.contains_key(&session));
            assert!(!kernel.sandbox_sessions.contains(&session));
        }
        assert_eq!(
            dispatch(
                &mut kernel,
                3,
                "sandbox/call",
                vec![Value::String("not-source".into())]
            )
            .unwrap_err(),
            "hta/capability-unsupported: sandbox/call"
        );
        assert_eq!(
            dispatch(
                &mut kernel,
                4,
                "sandbox/eval",
                vec![Value::String("x".repeat(MAX_SANDBOX_SOURCE_BYTES + 1))]
            )
            .unwrap_err(),
            "sandbox/source-limit: source exceeds 1048576 bytes"
        );
    }
}

#[no_mangle]
pub extern "C" fn hta_release(pointer: *const u8, size: usize) -> i32 {
    let bytes = if pointer.is_null() {
        &[][..]
    } else {
        unsafe { std::slice::from_raw_parts(pointer, size) }
    };
    match hta::decode(bytes) {
        Ok(Value::Extension(_)) => 0,
        _ => 1,
    }
}
