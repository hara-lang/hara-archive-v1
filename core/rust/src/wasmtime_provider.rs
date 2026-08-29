#![cfg(not(target_arch = "wasm32"))]

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use wasmtime::{
    Caller, Config, Engine, Extern, Func, Instance, Linker, Memory, Module, Store, StoreLimits,
    StoreLimitsBuilder, Val, ValType,
};

use crate::core::{Promise, PromiseState, Value};
use crate::extension::{ExtensionExport, ExtensionManifest, WasmAbi, WasmExtensionProvider};
use crate::hta;
use crate::wasm_binding::{MemoryBindingPlan, WasmtimeMemoryExecutor};

struct Session {
    store: Store<StoreLimits>,
    instance: Instance,
}

/// Process-shareable compiled code. Hosts can store one of these per artifact
/// digest and creates a fresh provider/store for every session that loads it.
#[derive(Clone)]
pub struct CompiledWasmModule {
    engine: Engine,
    module: Module,
    exports: Vec<(String, ExtensionExport)>,
}

impl CompiledWasmModule {
    pub fn compile(bytes: &[u8]) -> Result<Self, String> {
        let exports = crate::direct_wasm::exports(bytes)?;
        let mut config = Config::new();
        config.consume_fuel(true);
        let engine = Engine::new(&config)
            .map_err(|error| format!("extension/engine-unavailable: {error}"))?;
        let module = Module::new(&engine, bytes)
            .map_err(|error| format!("extension/module-invalid: {error}"))?;
        if module.imports().next().is_some() {
            return Err("extension/module-invalid: extension modules must be import-free".into());
        }
        Ok(Self {
            engine,
            module,
            exports,
        })
    }

    pub fn provider(&self) -> WasmtimeExtensionProvider {
        WasmtimeExtensionProvider {
            mode: ProviderMode::Direct {
                engine: self.engine.clone(),
                module: self.module.clone(),
                session: RefCell::new(None),
            },
        }
    }

    pub fn direct_exports(&self) -> Result<Vec<(String, ExtensionExport)>, String> {
        Ok(self.exports.clone())
    }
}

/// Import-free Wasmtime host for the direct scalar core.v1 ABI.
pub struct WasmtimeExtensionProvider {
    mode: ProviderMode,
}

enum ProviderMode {
    Direct {
        engine: Engine,
        module: Module,
        session: RefCell<Option<Session>>,
    },
    Memory(WasmtimeMemoryExecutor),
    Hta(Rc<HtaProviderState>),
}

impl WasmtimeExtensionProvider {
    pub fn compile(bytes: &[u8]) -> Result<Self, String> {
        Ok(CompiledWasmModule::compile(bytes)?.provider())
    }

    pub fn compile_memory(bytes: &[u8], plan: MemoryBindingPlan) -> Result<Self, String> {
        Ok(Self {
            mode: ProviderMode::Memory(WasmtimeMemoryExecutor::compile(bytes, plan)?),
        })
    }

    pub fn compile_hta(bytes: &[u8]) -> Result<Self, String> {
        Self::compile_hta_with_host_handler(bytes, None)
    }

    pub fn drain_lifecycle_events(&self) -> Vec<HtaProviderEvent> {
        match &self.mode {
            ProviderMode::Hta(state) => state.trace.drain(),
            _ => Vec::new(),
        }
    }

    pub fn compile_hta_with_host_handler(
        bytes: &[u8],
        host_handler: Option<Rc<dyn Fn(String, String, Vec<Value>) -> Result<Value, String>>>,
    ) -> Result<Self, String> {
        Self::compile_hta_parts(bytes, None, host_handler)
    }

    pub fn compile_hta_with_library(
        bytes: &[u8],
        library_bytes: &[u8],
        host_handler: Option<Rc<dyn Fn(String, String, Vec<Value>) -> Result<Value, String>>>,
    ) -> Result<Self, String> {
        Self::compile_hta_parts(bytes, Some(library_bytes), host_handler)
    }

    fn compile_hta_parts(
        bytes: &[u8],
        library_bytes: Option<&[u8]>,
        host_handler: Option<Rc<dyn Fn(String, String, Vec<Value>) -> Result<Value, String>>>,
    ) -> Result<Self, String> {
        let (engine, module) = compile_hta_module(bytes, library_bytes.is_some())?;
        let library = library_bytes
            .map(|bytes| {
                Module::new(&engine, bytes)
                    .map_err(|error| format!("extension/module-invalid: {error}"))
            })
            .transpose()?;
        Ok(Self {
            mode: ProviderMode::Hta(Rc::new(HtaProviderState {
                engine,
                module,
                library,
                session: RefCell::new(None),
                host_handler,
                timeout: hta_timeout(),
                trace: HtaProviderTrace::new(),
            })),
        })
    }
}

impl WasmExtensionProvider for WasmtimeExtensionProvider {
    fn supports(&self, abi: WasmAbi) -> bool {
        matches!(
            (&self.mode, abi),
            (ProviderMode::Direct { .. }, WasmAbi::CoreV1)
                | (ProviderMode::Memory(_), WasmAbi::MemoryV1)
                | (ProviderMode::Hta(_), WasmAbi::HtaV1)
        )
    }

    fn capabilities(&self) -> Vec<String> {
        if matches!(&self.mode, ProviderMode::Hta(_)) {
            return hta_capabilities();
        }
        Vec::new()
    }

    fn start(&self, manifest: &ExtensionManifest) -> Result<(), String> {
        if let ProviderMode::Hta(state) = &self.mode {
            return state.start(manifest);
        }
        if !manifest.capabilities.is_empty() || !manifest.host_call_capabilities.is_empty() {
            return Err(format!(
                "extension/capability-denied: {:?} for {}",
                manifest.capabilities, manifest.namespace
            ));
        }
        if let ProviderMode::Memory(executor) = &self.mode {
            let plan = executor.plan();
            if manifest.exports.len() != plan.functions.len()
                || manifest.exports.iter().any(|(name, specification)| {
                    plan.functions
                        .iter()
                        .find(|function| function.name == *name)
                        .map_or(true, |function| {
                            specification.raw_name(name) != function.wasm_export
                        })
                })
            {
                return Err(format!(
                    "extension/manifest-mismatch: memory.v1 exports for {} do not match bindings.edn",
                    manifest.namespace
                ));
            }
            return Ok(());
        }
        let ProviderMode::Direct {
            engine,
            module,
            session,
        } = &self.mode
        else {
            unreachable!()
        };
        let limits = StoreLimitsBuilder::new()
            .memory_size(64 * 1024 * 1024)
            .instances(1)
            .memories(1)
            .tables(1)
            .build();
        let mut store = Store::new(engine, limits);
        store.limiter(|limits| limits);
        let instance = Instance::new(&mut store, module, &[])
            .map_err(|error| format!("extension/module-invalid: {error}"))?;
        for (name, specification) in &manifest.exports {
            let raw_name = specification.raw_name(name);
            let function = instance.get_func(&mut store, raw_name).ok_or_else(|| {
                format!(
                    "extension/malformed: module has no export {raw_name} for public name {name}"
                )
            })?;
            if function.ty(&store).results().len() > 1 {
                return Err(format!(
                    "extension/abi-type-unsupported: {name} has multiple results"
                ));
            }
        }
        *session.borrow_mut() = Some(Session { store, instance });
        Ok(())
    }

    fn invoke(
        &self,
        manifest: &ExtensionManifest,
        export: &str,
        arguments: &[Value],
    ) -> Result<Value, String> {
        if let ProviderMode::Memory(executor) = &self.mode {
            return executor.invoke(export, arguments);
        }
        if let ProviderMode::Hta(state) = &self.mode {
            return state.invoke(manifest, export, arguments);
        }
        let ProviderMode::Direct { session, .. } = &self.mode else {
            unreachable!()
        };
        let specification = manifest
            .exports
            .iter()
            .find(|(name, _)| name == export)
            .map(|(_, specification)| specification)
            .ok_or_else(|| format!("extension/export-missing: {export}"))?;
        let raw_name = specification.raw_name(export);
        let mut session = session.borrow_mut();
        let session = session
            .as_mut()
            .ok_or_else(|| format!("extension/not-started: {}", manifest.namespace))?;
        let function = session
            .instance
            .get_func(&mut session.store, raw_name)
            .ok_or_else(|| format!("extension/export-missing: {export} -> {raw_name}"))?;
        let values = specification
            .arguments
            .iter()
            .zip(arguments)
            .map(|(wire_type, value)| argument(export, wire_type, value))
            .collect::<Result<Vec<_>, _>>()?;
        let mut results = if specification.returns == "void" {
            Vec::new()
        } else {
            vec![default_result(&specification.returns)?]
        };
        session
            .store
            .set_fuel(10_000_000)
            .map_err(|error| format!("extension/execution-limit: {error}"))?;
        function
            .call(&mut session.store, &values, &mut results)
            .map_err(|error| {
                format!(
                    "extension/invoke-failed: {}/{} ({error})",
                    manifest.namespace, export
                )
            })?;
        result(export, &specification.returns, results.into_iter().next())
    }

    fn cancel(&self, _manifest: &ExtensionManifest, _request: u64) -> Result<(), String> {
        if let ProviderMode::Hta(state) = &self.mode {
            return state.cancel(_request);
        }
        Err("extension/cancel-unsupported: core.v1 calls are synchronous".into())
    }

    fn release(&self, manifest: &ExtensionManifest, handle: &Value) -> Result<(), String> {
        if let ProviderMode::Hta(state) = &self.mode {
            return state.release(manifest, handle);
        }
        Err("extension/release-unsupported: provider has no HTA handle boundary".into())
    }

    fn shutdown(&self, manifest: &ExtensionManifest) {
        match &self.mode {
            ProviderMode::Direct { session, .. } => {
                session.borrow_mut().take();
            }
            ProviderMode::Hta(state) => state.shutdown(manifest),
            ProviderMode::Memory(_) => {}
        }
    }
}

const MAX_HTA_FRAME_BYTES: usize = 64 * 1024 * 1024;
const DEFAULT_HTA_TIMEOUT: Duration = Duration::from_secs(120);

pub const HTA_PROVIDER_EVENT_SCHEMA: &str = "hara.hta.provider.event/0-alpha";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HtaProviderEvent {
    pub schema: &'static str,
    pub sequence: u64,
    pub origin: &'static str,
    pub event: &'static str,
    pub request: Option<u64>,
    pub operation: Option<String>,
    pub status: Option<String>,
    pub code: Option<String>,
}

struct HtaProviderTrace {
    sequence: Cell<u64>,
    shutdown: Cell<bool>,
    events: RefCell<Vec<HtaProviderEvent>>,
}

impl HtaProviderTrace {
    fn new() -> Self {
        Self {
            sequence: Cell::new(0),
            shutdown: Cell::new(false),
            events: RefCell::new(Vec::new()),
        }
    }

    fn emit(
        &self,
        event: &'static str,
        request: Option<u64>,
        operation: Option<String>,
        status: Option<&str>,
        code: Option<String>,
    ) {
        let sequence = self.sequence.get() + 1;
        self.sequence.set(sequence);
        self.events.borrow_mut().push(HtaProviderEvent {
            schema: HTA_PROVIDER_EVENT_SCHEMA,
            sequence,
            origin: "wasmtime",
            event,
            operation,
            request,
            status: status.map(str::to_owned),
            code,
        });
    }

    fn emit_shutdown(&self, status: Option<&str>, code: Option<String>) {
        if self.shutdown.replace(true) {
            return;
        }
        self.emit("shutdown", None, None, status, code);
    }

    fn drain(&self) -> Vec<HtaProviderEvent> {
        std::mem::take(&mut *self.events.borrow_mut())
    }
}

fn hta_capabilities() -> Vec<String> {
    std::env::var("HARA_HTA_CAPABILITIES")
        .unwrap_or_default()
        .split([',', ' ', '\n', '\t'])
        .filter(|capability| !capability.is_empty())
        .map(str::to_owned)
        .collect()
}

struct HtaPending {
    promise: Promise,
    deadline: Option<Instant>,
    operation: String,
}

struct HtaSession {
    store: Store<StoreLimits>,
    memory: Memory,
    allocator: Func,
    deallocator: Func,
    start: Func,
    next_event: Func,
    deliver: Func,
    cancel: Func,
    drop_task: Func,
    release: Func,
    pending: HashMap<u64, HtaPending>,
    host_promises: HashMap<u64, Promise>,
    host_calls_seen: HashSet<u64>,
    handles: HashSet<(String, String, u64)>,
    deliveries: VecDeque<(u64, bool, Value)>,
}

struct HtaProviderState {
    engine: Engine,
    module: Module,
    library: Option<Module>,
    session: RefCell<Option<HtaSession>>,
    host_handler: Option<Rc<dyn Fn(String, String, Vec<Value>) -> Result<Value, String>>>,
    timeout: Option<Duration>,
    trace: HtaProviderTrace,
}

impl HtaProviderState {
    fn start(&self, manifest: &ExtensionManifest) -> Result<(), String> {
        if manifest.provider != "wasm" || manifest.abi != WasmAbi::HtaV1 {
            return Err(
                "extension/manifest-mismatch: HTA Wasm provider requires :wasm/:hta.v1".into(),
            );
        }
        let capabilities = hta_capabilities();
        if manifest
            .capabilities
            .iter()
            .chain(manifest.host_call_capabilities.values().flatten())
            .any(|capability| !capabilities.contains(capability))
        {
            return Err(format!(
                "extension/capability-denied: {:?} for {}",
                manifest.capabilities, manifest.namespace
            ));
        }
        if !manifest.host_calls.is_empty() && self.host_handler.is_none() {
            return Err(format!(
                "extension/host-unavailable: {} declares host calls",
                manifest.namespace
            ));
        }
        if self.session.borrow().is_some() {
            return Err(format!(
                "extension/start: session already exists for {}",
                manifest.namespace
            ));
        }

        let mut linker = Linker::new(&self.engine);
        linker
            .func_wrap(
                "env",
                "hara_random_fill",
                |mut caller: Caller<'_, StoreLimits>, pointer: i32, length: i32| -> i32 {
                    if pointer < 0 || length < 0 {
                        return 1;
                    }
                    let Some(Extern::Memory(memory)) = caller.get_export("memory") else {
                        return 1;
                    };
                    let mut bytes = vec![0_u8; length as usize];
                    if getrandom::getrandom(&mut bytes).is_err()
                        || memory.write(&mut caller, pointer as usize, &bytes).is_err()
                    {
                        return 1;
                    }
                    0
                },
            )
            .map_err(|error| format!("extension/engine-unavailable: {error}"))?;
        linker
            .func_wrap(
                "env",
                "hara_time_ms",
                |_caller: Caller<'_, StoreLimits>| -> i64 {
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map(|value| value.as_millis() as i64)
                        .unwrap_or_default()
                },
            )
            .map_err(|error| format!("extension/engine-unavailable: {error}"))?;
        linker
            .func_wrap(
                "env",
                "hara_time_ns",
                |_caller: Caller<'_, StoreLimits>| -> i64 {
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map(|value| value.as_nanos() as i64)
                        .unwrap_or_default()
                },
            )
            .map_err(|error| format!("extension/engine-unavailable: {error}"))?;

        let limits = StoreLimitsBuilder::new()
            .memory_size(64 * 1024 * 1024)
            .instances(if self.library.is_some() { 2 } else { 1 })
            .memories(1)
            .tables(1)
            .build();
        let mut store = Store::new(&self.engine, limits);
        store.limiter(|limits| limits);
        if let Some(library) = &self.library {
            let library_instance = Instance::new(&mut store, library, &[]).map_err(|error| {
                format!("extension/module-invalid: wrapped library cannot instantiate: {error}")
            })?;
            for import in self.module.imports() {
                if import.module() != "hara/library" {
                    continue;
                }
                let function = library_instance
                    .get_func(&mut store, import.name())
                    .ok_or_else(|| {
                        format!(
                            "extension/module-invalid: wrapped library has no export {}",
                            import.name()
                        )
                    })?;
                linker
                    .define(&mut store, import.module(), import.name(), function)
                    .map_err(|error| format!("extension/module-invalid: {error}"))?;
            }
        }
        let instance = linker
            .instantiate(&mut store, &self.module)
            .map_err(|error| format!("extension/module-invalid: {error}"))?;
        let memory = instance
            .get_memory(&mut store, "memory")
            .ok_or_else(|| "extension/malformed: module has no export memory".to_owned())?;
        let allocator = require_export(&instance, &mut store, "hta_alloc")?;
        let deallocator = require_export(&instance, &mut store, "hta_dealloc")?;
        let abi_version = require_export(&instance, &mut store, "hta_abi_version")?;
        let start = require_export(&instance, &mut store, "hta_start")?;
        let next_event = require_export(&instance, &mut store, "hta_next_event")?;
        let deliver = require_export(&instance, &mut store, "hta_deliver")?;
        let cancel = require_export(&instance, &mut store, "hta_cancel")?;
        let drop_task = require_export(&instance, &mut store, "hta_drop_task")?;
        expect_signature(
            &mut store,
            &allocator,
            &[ValType::I32],
            &[ValType::I32],
            "hta_alloc",
        )?;
        expect_signature(
            &mut store,
            &deallocator,
            &[ValType::I32, ValType::I32],
            &[],
            "hta_dealloc",
        )?;
        expect_signature(
            &mut store,
            &abi_version,
            &[],
            &[ValType::I32],
            "hta_abi_version",
        )?;
        expect_signature(
            &mut store,
            &start,
            &[ValType::I32, ValType::I32],
            &[ValType::I64],
            "hta_start",
        )?;
        expect_signature(
            &mut store,
            &next_event,
            &[],
            &[ValType::I64],
            "hta_next_event",
        )?;
        expect_signature(
            &mut store,
            &deliver,
            &[ValType::I32, ValType::I32],
            &[ValType::I32],
            "hta_deliver",
        )?;
        expect_signature(
            &mut store,
            &cancel,
            &[ValType::I64],
            &[ValType::I32],
            "hta_cancel",
        )?;
        expect_signature(
            &mut store,
            &drop_task,
            &[ValType::I64],
            &[ValType::I32],
            "hta_drop_task",
        )?;
        let release = require_export(&instance, &mut store, "hta_release")?;
        expect_signature(
            &mut store,
            &release,
            &[ValType::I32, ValType::I32],
            &[ValType::I32],
            "hta_release",
        )?;
        let version = call_i32(&mut store, &abi_version, &[], "hta_abi_version")?;
        if !(1..=4).contains(&version) {
            return Err(format!(
                "extension/abi-version-unsupported: {}",
                manifest.namespace
            ));
        }
        *self.session.borrow_mut() = Some(HtaSession {
            store,
            memory,
            allocator,
            deallocator,
            start,
            next_event,
            deliver,
            cancel,
            drop_task,
            release,
            pending: HashMap::new(),
            host_promises: HashMap::new(),
            host_calls_seen: HashSet::new(),
            handles: HashSet::new(),
            deliveries: VecDeque::new(),
        });
        self.trace.emit("start", None, None, Some("ok"), None);
        Ok(())
    }

    fn invoke(
        self: &Rc<Self>,
        manifest: &ExtensionManifest,
        export: &str,
        arguments: &[Value],
    ) -> Result<Value, String> {
        let promise = Promise::new();
        let (task, operation) = {
            let mut session_ref = self.session.borrow_mut();
            let session = session_ref
                .as_mut()
                .ok_or_else(|| "hta/session-closed".to_owned())?;
            let operation = manifest
                .operations
                .get(export)
                .cloned()
                .unwrap_or_else(|| export.to_owned());
            let arguments_value = Value::Vector(arguments.to_vec().into());
            validate_handles_in_value(&arguments_value, manifest)?;
            validate_live_handles(&arguments_value, &session.handles)?;
            let request = hta::encode(&Value::Vector(
                vec![
                    Value::String(operation.clone()),
                    Value::Vector(arguments.to_vec().into()),
                ]
                .into(),
            ))?;
            let task = execute_start(session, &request)?;
            if task <= 0 {
                return Err(format!("hta/start-failed: {}", manifest.namespace));
            }
            if session.pending.contains_key(&(task as u64)) {
                let _ = cancel_task_on_session(session, task as u64);
                let _ = drop_task_on_session(session, task as u64);
                return Err(format!("hta/task-duplicate: {}", task));
            }
            session.pending.insert(
                task as u64,
                HtaPending {
                    promise: promise.clone(),
                    deadline: self.timeout.map(|timeout| Instant::now() + timeout),
                    operation: operation.clone(),
                },
            );
            (task as u64, operation)
        };
        self.trace
            .emit("call-enter", Some(task), Some(operation), None, None);
        let weak = Rc::downgrade(self);
        let manifest_for_poll = manifest.clone();
        promise.set_poller(Rc::new(move || {
            if let Some(state) = weak.upgrade() {
                if let Err(error) = state.pump(&manifest_for_poll) {
                    state.fail_all(error);
                }
            }
        }));
        let weak = Rc::downgrade(self);
        let manifest_for_wait = manifest.clone();
        let waiting = promise.clone();
        promise.set_waiter(Rc::new(move || {
            if let Some(state) = weak.upgrade() {
                loop {
                    if !state.is_pending(task) {
                        break;
                    }
                    if let Err(error) = state.pump(&manifest_for_wait) {
                        state.fail_all(error);
                        break;
                    }
                    if !state.is_pending(task) {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
                if matches!(waiting.state(), PromiseState::Pending) && state.is_expired(task) {
                    state.timeout(task);
                }
            }
        }));
        let weak = Rc::downgrade(self);
        promise.set_cancel_hook(Rc::new(move || {
            if let Some(state) = weak.upgrade() {
                let _ = state.cancel(task);
            }
        }));
        if let Err(error) = self.pump(manifest) {
            self.fail_all(error.clone());
            return Err(error);
        }
        Ok(Value::Promise(promise))
    }

    fn release(&self, manifest: &ExtensionManifest, handle: &Value) -> Result<(), String> {
        validate_handles_in_value(handle, manifest)?;
        let Value::Extension(handle_value) = handle else {
            return Err("hta/handle-invalid: release expects an opaque handle".into());
        };
        let key = (
            handle_value.provider.clone(),
            handle_value.type_name.clone(),
            handle_value.handle,
        );
        let frame = hta::encode(handle)?;
        let mut session_ref = self.session.borrow_mut();
        let session = session_ref
            .as_mut()
            .ok_or_else(|| "hta/session-closed".to_owned())?;
        if !session.handles.remove(&key) {
            self.trace.emit(
                "release",
                None,
                None,
                Some("error"),
                Some("hta/handle-stale".into()),
            );
            return Err(format!(
                "hta/handle-stale: {}:{}",
                handle_value.type_name, handle_value.handle
            ));
        }
        if let Err(error) = execute_release(session, &frame) {
            session.handles.insert(key);
            self.trace
                .emit("release", None, None, Some("error"), Some(error.clone()));
            return Err(error);
        }
        self.trace.emit("release", None, None, Some("ok"), None);
        Ok(())
    }

    fn pump(self: &Rc<Self>, manifest: &ExtensionManifest) -> Result<(), String> {
        self.poll_host_promises();
        self.deliver_pending()?;
        loop {
            let event = self.next_event()?;
            let Some(event) = event else {
                self.expire_pending();
                return Ok(());
            };
            self.handle_event(manifest, event)?;
            self.poll_host_promises();
            self.deliver_pending()?;
        }
    }

    fn next_event(&self) -> Result<Option<Value>, String> {
        let mut session_ref = self.session.borrow_mut();
        let session = session_ref
            .as_mut()
            .ok_or_else(|| "hta/session-closed".to_owned())?;
        let packed = call_i64(
            &mut session.store,
            &session.next_event,
            &[],
            "hta_next_event",
        )?;
        if packed == 0 {
            return Ok(None);
        }
        if packed < 0 {
            return Err("hta/event-pointer-invalid".into());
        }
        let packed = packed as u64;
        let pointer = (packed >> 32) as usize;
        let size = (packed & u64::from(u32::MAX)) as usize;
        if size == 0 || size > MAX_HTA_FRAME_BYTES {
            return Err("hta/event-size-invalid".into());
        }
        let mut bytes = vec![0_u8; size];
        session
            .memory
            .read(&session.store, pointer, &mut bytes)
            .map_err(|error| format!("hta/event-memory-invalid: {error}"))?;
        call_void(
            &mut session.store,
            &session.deallocator,
            &[Val::I32(pointer as i32), Val::I32(size as i32)],
            "hta_dealloc",
        )?;
        hta::decode_canonical(&bytes)
            .map(Some)
            .map_err(|error| format!("hta/event-malformed: {error}"))
    }

    fn handle_event(
        self: &Rc<Self>,
        manifest: &ExtensionManifest,
        event: Value,
    ) -> Result<(), String> {
        let values = match event {
            Value::Vector(values) => values.iter().cloned().collect::<Vec<_>>(),
            Value::List(values) => values.iter().cloned().collect::<Vec<_>>(),
            _ => return Err("hta/event-malformed".into()),
        };
        let kind = number(&values, 0, "kind")?;
        match kind {
            0 | 1 => {
                let task = number(&values, 1, "task")?;
                let payload = values
                    .get(2)
                    .cloned()
                    .ok_or_else(|| "hta/event-malformed: payload".to_owned())?;
                validate_handles_in_value(&payload, manifest)?;
                {
                    let session_ref = self.session.borrow();
                    let session = session_ref
                        .as_ref()
                        .ok_or_else(|| "hta/session-closed".to_owned())?;
                    if kind == 1 {
                        validate_live_handles(&payload, &session.handles)?;
                    }
                }
                if self.is_pending(task) {
                    self.drop_task(task)?;
                    let pending = self
                        .session
                        .borrow_mut()
                        .as_mut()
                        .and_then(|session| session.pending.remove(&task));
                    let Some(pending) = pending else {
                        return Ok(());
                    };
                    if kind == 0 {
                        if let Some(session) = self.session.borrow_mut().as_mut() {
                            collect_handles(&payload, &mut session.handles);
                        }
                    }
                    self.trace.emit(
                        if kind == 0 {
                            "call-return"
                        } else {
                            "call-error"
                        },
                        Some(task),
                        Some(pending.operation.clone()),
                        Some(if kind == 0 { "ok" } else { "error" }),
                        None,
                    );
                    if kind == 0 {
                        pending.promise.resolve(payload);
                    } else {
                        pending.promise.reject_value(payload);
                    }
                }
                Ok(())
            }
            2 => self.handle_host_event(manifest, &values),
            _ => Err(format!("hta/event-unknown: {kind}")),
        }
    }

    fn handle_host_event(
        self: &Rc<Self>,
        manifest: &ExtensionManifest,
        values: &[Value],
    ) -> Result<(), String> {
        if values.len() != 6 && values.len() != 8 {
            return Err("hta/host-call-malformed".into());
        }
        let call = number(values, 1, "call")?;
        let task = number(values, 2, "task")?;
        if !self.is_pending(task) {
            return Ok(());
        }
        if !self
            .session
            .borrow_mut()
            .as_mut()
            .ok_or_else(|| "hta/session-closed".to_owned())?
            .host_calls_seen
            .insert(call)
        {
            return Ok(());
        }
        let service_index = if values.len() == 8 { 5 } else { 3 };
        let service = string_value(values, service_index, "service")?;
        let method = string_value(values, service_index + 1, "method")?;
        let arguments = match values.get(service_index + 2) {
            Some(Value::Vector(arguments)) => arguments.iter().cloned().collect::<Vec<_>>(),
            Some(Value::List(arguments)) => arguments.iter().cloned().collect::<Vec<_>>(),
            _ => return Err("hta/host-call-malformed: arguments".into()),
        };
        validate_handles_in_value(&Value::Vector(arguments.clone().into()), manifest)?;
        if let Some(session) = self.session.borrow().as_ref() {
            validate_live_handles(&Value::Vector(arguments.clone().into()), &session.handles)?;
        }
        if !manifest.permits_host_call(&service, &method) {
            self.queue_delivery(
                call,
                false,
                host_error("hta/host-call-denied", &service, &method),
            );
            return Ok(());
        }
        if manifest
            .host_call_capabilities(&service, &method)
            .iter()
            .any(|capability| !hta_capabilities().contains(capability))
        {
            self.queue_delivery(
                call,
                false,
                host_error("hta/capability-denied", &service, &method),
            );
            return Ok(());
        }
        let Some(handler) = self.host_handler.clone() else {
            self.queue_delivery(
                call,
                false,
                host_error("host/unavailable", &service, &method),
            );
            return Ok(());
        };
        match handler(service.clone(), method.clone(), arguments) {
            Ok(Value::Promise(promise)) => {
                self.session
                    .borrow_mut()
                    .as_mut()
                    .ok_or_else(|| "hta/session-closed".to_owned())?
                    .host_promises
                    .insert(call, promise.clone());
                let weak = Rc::downgrade(self);
                promise.on_settle(Rc::new(move |state| {
                    if let Some(state_owner) = weak.upgrade() {
                        match state {
                            PromiseState::Fulfilled(value) => {
                                state_owner.queue_delivery(call, true, value)
                            }
                            PromiseState::Rejected(error) => state_owner.queue_delivery(
                                call,
                                false,
                                host_failure("hta/host-call-failed", &error.message()),
                            ),
                            PromiseState::Pending => {}
                        }
                    }
                }));
            }
            Ok(value) => self.queue_delivery(call, true, value),
            Err(error) => {
                self.queue_delivery(call, false, host_failure("hta/host-call-failed", &error))
            }
        }
        Ok(())
    }

    fn poll_host_promises(&self) {
        let promises = self
            .session
            .borrow()
            .as_ref()
            .map(|session| session.host_promises.values().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        for promise in promises {
            let _ = promise.state();
        }
    }

    fn queue_delivery(&self, call: u64, fulfilled: bool, value: Value) {
        if let Some(session) = self.session.borrow_mut().as_mut() {
            session.deliveries.push_back((call, fulfilled, value));
        }
    }

    fn deliver_pending(&self) -> Result<(), String> {
        loop {
            let delivery = self
                .session
                .borrow_mut()
                .as_mut()
                .and_then(|session| session.deliveries.pop_front());
            let Some((call, fulfilled, value)) = delivery else {
                return Ok(());
            };
            let frame = hta::encode(&Value::Vector(
                vec![
                    Value::Number(call as i64),
                    Value::Number(if fulfilled { 0 } else { 1 }),
                    value.clone(),
                ]
                .into(),
            ))?;
            let mut session_ref = self.session.borrow_mut();
            let session = session_ref
                .as_mut()
                .ok_or_else(|| "hta/session-closed".to_owned())?;
            execute_deliver(session, &frame)?;
            if fulfilled {
                collect_handles(&value, &mut session.handles);
            }
            session.host_promises.remove(&call);
        }
    }

    fn drop_task(&self, task: u64) -> Result<(), String> {
        let mut session_ref = self.session.borrow_mut();
        let session = session_ref
            .as_mut()
            .ok_or_else(|| "hta/session-closed".to_owned())?;
        let status = call_i32(
            &mut session.store,
            &session.drop_task,
            &[Val::I64(task as i64)],
            "hta_drop_task",
        )?;
        if status != 0 {
            return Err(format!("hta/drop-task-failed: {status}"));
        }
        Ok(())
    }

    fn cancel(&self, task: u64) -> Result<(), String> {
        let pending = self
            .session
            .borrow_mut()
            .as_mut()
            .ok_or_else(|| "hta/session-closed".to_owned())?
            .pending
            .remove(&task);
        let Some(pending) = pending else {
            return Ok(());
        };
        if let Err(error) = self.cancel_task(task) {
            let _ = self.drop_task(task);
            self.trace.emit(
                "cancel",
                Some(task),
                Some(pending.operation),
                Some("error"),
                Some(error.clone()),
            );
            return Err(error);
        }
        self.trace.emit(
            "cancel",
            Some(task),
            Some(pending.operation),
            Some("ok"),
            None,
        );
        Ok(())
    }

    fn cancel_task(&self, task: u64) -> Result<(), String> {
        let mut session_ref = self.session.borrow_mut();
        let session = session_ref
            .as_mut()
            .ok_or_else(|| "hta/session-closed".to_owned())?;
        let status = call_i32(
            &mut session.store,
            &session.cancel,
            &[Val::I64(task as i64)],
            "hta_cancel",
        )?;
        if status != 0 {
            return Err(format!("hta/cancel-failed: {status}"));
        }
        let drop_status = call_i32(
            &mut session.store,
            &session.drop_task,
            &[Val::I64(task as i64)],
            "hta_drop_task",
        )?;
        if drop_status != 0 {
            return Err(format!("hta/drop-task-failed: {drop_status}"));
        }
        Ok(())
    }

    fn is_pending(&self, task: u64) -> bool {
        self.session
            .borrow()
            .as_ref()
            .is_some_and(|session| session.pending.contains_key(&task))
    }

    fn is_expired(&self, task: u64) -> bool {
        self.session
            .borrow()
            .as_ref()
            .and_then(|session| session.pending.get(&task))
            .and_then(|pending| pending.deadline)
            .is_some_and(|deadline| deadline <= Instant::now())
    }

    fn expire_pending(&self) {
        let expired = self
            .session
            .borrow()
            .as_ref()
            .map(|session| {
                session
                    .pending
                    .iter()
                    .filter_map(|(task, pending)| {
                        pending
                            .deadline
                            .filter(|deadline| *deadline <= Instant::now())
                            .map(|_| *task)
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for task in expired {
            self.timeout(task);
        }
    }

    fn timeout(&self, task: u64) {
        if self.is_pending(task) {
            if self.cancel_task(task).is_err() {
                let _ = self.drop_task(task);
            }
            let pending = self
                .session
                .borrow_mut()
                .as_mut()
                .and_then(|session| session.pending.remove(&task));
            if let Some(pending) = pending {
                self.trace.emit(
                    "call-error",
                    Some(task),
                    Some(pending.operation),
                    Some("error"),
                    Some("hta/timeout".into()),
                );
                pending.promise.notify_cancel();
                pending.promise.reject("hta/timeout");
            }
        }
    }

    fn fail_all(&self, error: String) {
        let pending = self
            .session
            .borrow_mut()
            .as_mut()
            .map(|session| {
                session
                    .pending
                    .drain()
                    .map(|(task, pending)| (task, pending.promise))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for (task, promise) in pending {
            if let Some(session) = self.session.borrow_mut().as_mut() {
                let _ = cancel_task_on_session(session, task);
                let _ = drop_task_on_session(session, task);
            }
            promise.reject(error.clone());
        }
        if let Some(session) = self.session.borrow_mut().as_mut() {
            session.host_promises.clear();
            session.deliveries.clear();
        }
    }

    fn shutdown(&self, _manifest: &ExtensionManifest) {
        let Some(mut session) = self.session.borrow_mut().take() else {
            self.trace.emit_shutdown(Some("ok"), None);
            return;
        };
        let pending = session
            .pending
            .drain()
            .map(|(task, pending)| (task, pending.operation, pending.promise))
            .collect::<Vec<_>>();
        for (task, operation, promise) in pending {
            let _ = cancel_task_on_session(&mut session, task);
            let _ = drop_task_on_session(&mut session, task);
            self.trace.emit(
                "call-error",
                Some(task),
                Some(operation),
                Some("error"),
                Some("hta/session-closed".into()),
            );
            promise.reject("hta/session-closed");
        }
        session.host_promises.clear();
        session.deliveries.clear();
        self.trace.emit_shutdown(Some("ok"), None);
    }
}

fn validate_handles_in_value(value: &Value, manifest: &ExtensionManifest) -> Result<(), String> {
    match value {
        Value::Extension(handle) => {
            if manifest.handle_tags.is_empty() {
                return Ok(());
            }
            let Some(owner) = manifest.handle_tags.get(&handle.type_name) else {
                return Err(format!("hta/handle-type-denied: {}", handle.type_name));
            };
            if handle.provider != manifest.namespace
                && manifest.identity.as_deref() != Some(handle.provider.as_str())
                && handle.provider != *owner
            {
                return Err(format!(
                    "hta/handle-owner-mismatch: {}:{}",
                    handle.provider, handle.handle
                ));
            }
        }
        Value::Tagged(value) => validate_handles_in_value(value.form(), manifest)?,
        Value::Vector(values) => validate_handles_iter(values.iter(), manifest)?,
        Value::List(values) => validate_handles_iter(values.iter(), manifest)?,
        Value::Tuple(values) => validate_handles_iter(values.iter(), manifest)?,
        Value::MapEntry(entry) => {
            validate_handles_in_value(entry.key(), manifest)?;
            validate_handles_in_value(entry.value(), manifest)?;
        }
        Value::Map(values) => validate_handles_map(values.iter(), manifest)?,
        Value::SortedMap(values) => validate_handles_map(values.iter(), manifest)?,
        Value::OrderedMap(values) => {
            for (key, value) in values.iter() {
                validate_handles_in_value(key, manifest)?;
                validate_handles_in_value(value, manifest)?;
            }
        }
        Value::PriorityMap(values) => {
            for (key, value) in values.iter() {
                validate_handles_in_value(&key, manifest)?;
                validate_handles_in_value(&value, manifest)?;
            }
        }
        Value::Set(values) => validate_handles_iter(values.iter(), manifest)?,
        Value::OrderedSet(values) => validate_handles_iter(values.iter(), manifest)?,
        Value::SortedSet(values) => validate_handles_iter(values.iter(), manifest)?,
        Value::Struct(value) => {
            for value in value.ordered_values() {
                validate_handles_in_value(value, manifest)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn validate_handles_iter<'a>(
    values: impl Iterator<Item = &'a Value>,
    manifest: &ExtensionManifest,
) -> Result<(), String> {
    for value in values {
        validate_handles_in_value(value, manifest)?;
    }
    Ok(())
}

fn validate_handles_map<'a>(
    values: impl Iterator<Item = (&'a Value, &'a Value)>,
    manifest: &ExtensionManifest,
) -> Result<(), String> {
    for (key, value) in values {
        validate_handles_in_value(key, manifest)?;
        validate_handles_in_value(value, manifest)?;
    }
    Ok(())
}

fn validate_live_handles(
    value: &Value,
    handles: &HashSet<(String, String, u64)>,
) -> Result<(), String> {
    match value {
        Value::Extension(handle)
            if !handles.contains(&(
                handle.provider.clone(),
                handle.type_name.clone(),
                handle.handle,
            )) =>
        {
            return Err(format!(
                "hta/handle-stale: {}:{}",
                handle.type_name, handle.handle
            ));
        }
        Value::Vector(values) => validate_live_iter(values.iter(), handles)?,
        Value::List(values) => validate_live_iter(values.iter(), handles)?,
        Value::Tuple(values) => validate_live_iter(values.iter(), handles)?,
        Value::MapEntry(entry) => {
            validate_live_handles(entry.key(), handles)?;
            validate_live_handles(entry.value(), handles)?;
        }
        Value::Map(values) => validate_live_map(values.iter(), handles)?,
        Value::SortedMap(values) => validate_live_map(values.iter(), handles)?,
        Value::OrderedMap(values) => {
            for (key, value) in values.iter() {
                validate_live_handles(key, handles)?;
                validate_live_handles(value, handles)?;
            }
        }
        Value::PriorityMap(values) => {
            for (key, value) in values.iter() {
                validate_live_handles(&key, handles)?;
                validate_live_handles(&value, handles)?;
            }
        }
        Value::Set(values) => validate_live_iter(values.iter(), handles)?,
        Value::OrderedSet(values) => validate_live_iter(values.iter(), handles)?,
        Value::SortedSet(values) => validate_live_iter(values.iter(), handles)?,
        Value::Tagged(value) => validate_live_handles(value.form(), handles)?,
        Value::Struct(value) => {
            for value in value.ordered_values() {
                validate_live_handles(value, handles)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn validate_live_iter<'a>(
    values: impl Iterator<Item = &'a Value>,
    handles: &HashSet<(String, String, u64)>,
) -> Result<(), String> {
    for value in values {
        validate_live_handles(value, handles)?;
    }
    Ok(())
}

fn validate_live_map<'a>(
    values: impl Iterator<Item = (&'a Value, &'a Value)>,
    handles: &HashSet<(String, String, u64)>,
) -> Result<(), String> {
    for (key, value) in values {
        validate_live_handles(key, handles)?;
        validate_live_handles(value, handles)?;
    }
    Ok(())
}

fn collect_handles(value: &Value, handles: &mut HashSet<(String, String, u64)>) {
    match value {
        Value::Extension(handle) => {
            handles.insert((
                handle.provider.clone(),
                handle.type_name.clone(),
                handle.handle,
            ));
        }
        Value::Vector(values) => collect_iter(values.iter(), handles),
        Value::List(values) => collect_iter(values.iter(), handles),
        Value::Tuple(values) => collect_iter(values.iter(), handles),
        Value::MapEntry(entry) => {
            collect_handles(entry.key(), handles);
            collect_handles(entry.value(), handles);
        }
        Value::Map(values) => collect_map(values.iter(), handles),
        Value::SortedMap(values) => collect_map(values.iter(), handles),
        Value::OrderedMap(values) => {
            for (key, value) in values.iter() {
                collect_handles(key, handles);
                collect_handles(value, handles);
            }
        }
        Value::PriorityMap(values) => {
            for (key, value) in values.iter() {
                collect_handles(&key, handles);
                collect_handles(&value, handles);
            }
        }
        Value::Set(values) => collect_iter(values.iter(), handles),
        Value::OrderedSet(values) => collect_iter(values.iter(), handles),
        Value::SortedSet(values) => collect_iter(values.iter(), handles),
        Value::Tagged(value) => collect_handles(value.form(), handles),
        Value::Struct(value) => {
            for value in value.ordered_values() {
                collect_handles(value, handles);
            }
        }
        _ => {}
    }
}

fn collect_iter<'a>(
    values: impl Iterator<Item = &'a Value>,
    handles: &mut HashSet<(String, String, u64)>,
) {
    for value in values {
        collect_handles(value, handles);
    }
}

fn collect_map<'a>(
    values: impl Iterator<Item = (&'a Value, &'a Value)>,
    handles: &mut HashSet<(String, String, u64)>,
) {
    for (key, value) in values {
        collect_handles(key, handles);
        collect_handles(value, handles);
    }
}

fn compile_hta_module(bytes: &[u8], allow_library: bool) -> Result<(Engine, Module), String> {
    let mut config = Config::new();
    config.consume_fuel(true);
    let engine =
        Engine::new(&config).map_err(|error| format!("extension/engine-unavailable: {error}"))?;
    let module = Module::new(&engine, bytes)
        .map_err(|error| format!("extension/module-invalid: {error}"))?;
    for import in module.imports() {
        let supported_env = import.module() == "env"
            && matches!(
                import.name(),
                "hara_random_fill" | "hara_time_ms" | "hara_time_ns"
            );
        let supported_library = allow_library && import.module() == "hara/library";
        if !supported_env && !supported_library {
            return Err(format!(
                "extension/module-invalid: unsupported import {}::{}",
                import.module(),
                import.name()
            ));
        }
    }
    Ok((engine, module))
}

fn require_export(
    instance: &Instance,
    store: &mut Store<StoreLimits>,
    name: &str,
) -> Result<Func, String> {
    instance
        .get_func(&mut *store, name)
        .ok_or_else(|| format!("extension/malformed: module has no export {name}"))
}

fn expect_signature(
    store: &mut Store<StoreLimits>,
    function: &Func,
    parameters: &[ValType],
    results: &[ValType],
    name: &str,
) -> Result<(), String> {
    let ty = function.ty(&mut *store);
    let actual_parameters = ty.params().collect::<Vec<_>>();
    let actual_results = ty.results().collect::<Vec<_>>();
    if actual_parameters != parameters || actual_results != results {
        return Err(format!(
            "extension/abi-type-unsupported: {name} has an invalid signature"
        ));
    }
    Ok(())
}

fn call_i32(
    store: &mut Store<StoreLimits>,
    function: &Func,
    arguments: &[Val],
    name: &str,
) -> Result<i32, String> {
    store
        .set_fuel(10_000_000)
        .map_err(|error| format!("extension/execution-limit: {error}"))?;
    let mut results = [Val::I32(0)];
    function
        .call(store, arguments, &mut results)
        .map_err(|error| format!("extension/{name}-failed: {error}"))?;
    match results[0] {
        Val::I32(value) => Ok(value),
        _ => Err(format!("extension/abi-type-unsupported: {name}")),
    }
}

fn call_i64(
    store: &mut Store<StoreLimits>,
    function: &Func,
    arguments: &[Val],
    name: &str,
) -> Result<i64, String> {
    store
        .set_fuel(10_000_000)
        .map_err(|error| format!("extension/execution-limit: {error}"))?;
    let mut results = [Val::I64(0)];
    function
        .call(store, arguments, &mut results)
        .map_err(|error| format!("extension/{name}-failed: {error}"))?;
    match results[0] {
        Val::I64(value) => Ok(value),
        _ => Err(format!("extension/abi-type-unsupported: {name}")),
    }
}

fn call_void(
    store: &mut Store<StoreLimits>,
    function: &Func,
    arguments: &[Val],
    name: &str,
) -> Result<(), String> {
    store
        .set_fuel(10_000_000)
        .map_err(|error| format!("extension/execution-limit: {error}"))?;
    function
        .call(store, arguments, &mut [])
        .map_err(|error| format!("extension/{name}-failed: {error}"))
}

fn execute_start(session: &mut HtaSession, frame: &[u8]) -> Result<i64, String> {
    let pointer = call_i32(
        &mut session.store,
        &session.allocator,
        &[Val::I32(frame.len() as i32)],
        "hta_alloc",
    )?;
    if pointer < 0 {
        return Err("hta/memory-unavailable".into());
    }
    session
        .memory
        .write(&mut session.store, pointer as usize, frame)
        .map_err(|error| format!("hta/memory-write-failed: {error}"))?;
    let result = call_i64(
        &mut session.store,
        &session.start,
        &[Val::I32(pointer), Val::I32(frame.len() as i32)],
        "hta_start",
    );
    call_void(
        &mut session.store,
        &session.deallocator,
        &[Val::I32(pointer), Val::I32(frame.len() as i32)],
        "hta_dealloc",
    )?;
    result
}

fn execute_deliver(session: &mut HtaSession, frame: &[u8]) -> Result<(), String> {
    let pointer = call_i32(
        &mut session.store,
        &session.allocator,
        &[Val::I32(frame.len() as i32)],
        "hta_alloc",
    )?;
    if pointer < 0 {
        return Err("hta/memory-unavailable".into());
    }
    session
        .memory
        .write(&mut session.store, pointer as usize, frame)
        .map_err(|error| format!("hta/memory-write-failed: {error}"))?;
    let status = call_i32(
        &mut session.store,
        &session.deliver,
        &[Val::I32(pointer), Val::I32(frame.len() as i32)],
        "hta_deliver",
    );
    call_void(
        &mut session.store,
        &session.deallocator,
        &[Val::I32(pointer), Val::I32(frame.len() as i32)],
        "hta_dealloc",
    )?;
    let status = status?;
    if status != 0 {
        return Err(format!("hta/deliver-failed: {status}"));
    }
    Ok(())
}

fn execute_release(session: &mut HtaSession, frame: &[u8]) -> Result<(), String> {
    let pointer = call_i32(
        &mut session.store,
        &session.allocator,
        &[Val::I32(frame.len() as i32)],
        "hta_alloc",
    )?;
    if pointer < 0 {
        return Err("hta/memory-unavailable".into());
    }
    session
        .memory
        .write(&mut session.store, pointer as usize, frame)
        .map_err(|error| format!("hta/memory-write-failed: {error}"))?;
    let status = call_i32(
        &mut session.store,
        &session.release,
        &[Val::I32(pointer), Val::I32(frame.len() as i32)],
        "hta_release",
    );
    call_void(
        &mut session.store,
        &session.deallocator,
        &[Val::I32(pointer), Val::I32(frame.len() as i32)],
        "hta_dealloc",
    )?;
    let status = status?;
    if status != 0 {
        return Err(format!("hta/handle-release-failed: {status}"));
    }
    Ok(())
}

fn drop_task_on_session(session: &mut HtaSession, task: u64) -> Result<(), String> {
    let status = call_i32(
        &mut session.store,
        &session.drop_task,
        &[Val::I64(task as i64)],
        "hta_drop_task",
    )?;
    if status != 0 {
        return Err(format!("hta/drop-task-failed: {status}"));
    }
    Ok(())
}

fn cancel_task_on_session(session: &mut HtaSession, task: u64) -> Result<(), String> {
    let status = call_i32(
        &mut session.store,
        &session.cancel,
        &[Val::I64(task as i64)],
        "hta_cancel",
    )?;
    if status != 0 {
        return Err(format!("hta/cancel-failed: {status}"));
    }
    Ok(())
}

fn hta_timeout() -> Option<Duration> {
    match std::env::var("HARA_HTA_TIMEOUT_MS") {
        Ok(value) => match value.parse::<u64>() {
            Ok(0) => None,
            Ok(milliseconds) => Some(Duration::from_millis(milliseconds)),
            Err(_) => Some(DEFAULT_HTA_TIMEOUT),
        },
        Err(_) => Some(DEFAULT_HTA_TIMEOUT),
    }
}

fn number(values: &[Value], index: usize, field: &str) -> Result<u64, String> {
    match values.get(index) {
        Some(Value::Number(value)) if *value >= 0 => Ok(*value as u64),
        _ => Err(format!("hta/event-malformed: {field}")),
    }
}

fn string_value(values: &[Value], index: usize, field: &str) -> Result<String, String> {
    match values.get(index) {
        Some(Value::String(value)) => Ok(value.clone()),
        _ => Err(format!("hta/event-malformed: {field}")),
    }
}

fn host_error(code: &str, service: &str, method: &str) -> Value {
    host_failure(code, &format!("{service}/{method}"))
}

fn host_failure(code: &str, message: &str) -> Value {
    Value::Map(
        [
            (Value::Keyword("code".into()), Value::Keyword(code.into())),
            (
                Value::Keyword("message".into()),
                Value::String(message.into()),
            ),
            (
                Value::Keyword("origin".into()),
                Value::Keyword("host".into()),
            ),
            (Value::Keyword("retryable".into()), Value::Bool(false)),
        ]
        .into_iter()
        .collect(),
    )
}

fn argument(export: &str, wire_type: &str, value: &Value) -> Result<Val, String> {
    fn finite_f32(value: f64) -> Result<f32, String> {
        let value = value as f32;
        if value.is_finite() {
            Ok(value)
        } else {
            Err("non-finite number".into())
        }
    }
    let type_error = || format!("extension/type-error: {export} expects {wire_type}");
    match (wire_type, value) {
        ("i32", Value::Number(value)) => i32::try_from(*value)
            .map(Val::I32)
            .map_err(|_| type_error()),
        ("i64", Value::Number(value)) => Ok(Val::I64(*value)),
        ("f32", Value::Float(value)) => Ok(Val::F32(finite_f32(*value)?.to_bits())),
        ("f32", Value::Number(value)) => Ok(Val::F32(finite_f32(*value as f64)?.to_bits())),
        ("f64", Value::Float(value)) => {
            Ok(Val::F64(crate::numeric::finite_float(*value)?.to_bits()))
        }
        ("f64", Value::Number(value)) => Ok(Val::F64((*value as f64).to_bits())),
        ("boolean", Value::Bool(value)) => Ok(Val::I32(i32::from(*value))),
        _ => Err(type_error()),
    }
}

fn default_result(wire_type: &str) -> Result<Val, String> {
    match wire_type {
        "i32" | "boolean" => Ok(Val::I32(0)),
        "i64" => Ok(Val::I64(0)),
        "f32" => Ok(Val::F32(0)),
        "f64" => Ok(Val::F64(0)),
        _ => Err(format!("extension/abi-type-unsupported: {wire_type}")),
    }
}

fn result(export: &str, wire_type: &str, value: Option<Val>) -> Result<Value, String> {
    match (wire_type, value) {
        ("void", None) => Ok(Value::Nil),
        ("i32", Some(Val::I32(value))) => Ok(Value::Number(i64::from(value))),
        ("i64", Some(Val::I64(value))) => Ok(Value::Number(value)),
        ("f32", Some(Val::F32(value))) => Ok(Value::Float(crate::numeric::finite_float(
            f32::from_bits(value) as f64,
        )?)),
        ("f64", Some(Val::F64(value))) => Ok(Value::Float(crate::numeric::finite_float(
            f64::from_bits(value),
        )?)),
        ("boolean", Some(Val::I32(value))) => Ok(Value::Bool(value != 0)),
        _ => Err(format!(
            "extension/abi-type-unsupported: {export} -> {wire_type}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use crate::extension::{ExtensionManifest, Value, WasmExtension};

    use super::{HtaProviderTrace, WasmtimeExtensionProvider, HTA_PROVIDER_EVENT_SCHEMA};

    const ADD: &[u8] = b"\0asm\x01\0\0\0\x01\x07\x01\x60\x02\x7e\x7e\x01\x7e\x03\x02\x01\0\x07\x07\x01\x03add\0\0\x0a\x09\x01\x07\0\x20\0\x20\x01\x7c\x0b";
    const ALIASED_MANIFEST: &str = r#"
      {:namespace "math.scalar"
       :version "0.1.0"
       :provider :wasm
       :module "math.wasm"
       :abi :core.v1
       :exports {"sum" {:wasm/export "add"
                         :args [:i64 :i64]
                         :returns :i64}}
       :capabilities []}"#;

    #[test]
    fn invokes_a_raw_wasm_export_through_a_public_hara_name() {
        let manifest = ExtensionManifest::parse(ALIASED_MANIFEST, "fixture").unwrap();
        let provider = WasmtimeExtensionProvider::compile(ADD).unwrap();
        let mut extension = WasmExtension::new(manifest, provider).unwrap();
        let bindings = extension.require().unwrap();
        assert_eq!(bindings[0].name, "sum");
        assert_eq!(
            bindings[0]
                .invoke(&[Value::Number(19), Value::Number(23)])
                .unwrap(),
            Value::Number(42)
        );
    }

    #[test]
    fn hta_lifecycle_trace_is_stable_and_shutdown_is_idempotent() {
        let trace = HtaProviderTrace::new();
        trace.emit("start", None, None, Some("ok"), None);
        trace.emit("call-enter", Some(7), Some("demo/echo".into()), None, None);
        trace.emit_shutdown(Some("ok"), None);
        trace.emit_shutdown(Some("error"), Some("late".into()));

        let events = trace.drain();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].schema, HTA_PROVIDER_EVENT_SCHEMA);
        assert_eq!(events[0].origin, "wasmtime");
        assert_eq!(events[1].sequence, 2);
        assert_eq!(events[1].request, Some(7));
        assert_eq!(events[1].operation.as_deref(), Some("demo/echo"));
        assert_eq!(events[2].event, "shutdown");
        assert_eq!(events[2].sequence, 3);
    }
}
