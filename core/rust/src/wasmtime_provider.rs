#![cfg(not(target_arch = "wasm32"))]

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
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

    pub fn compile_hta_with_host_handler(
        bytes: &[u8],
        host_handler: Option<
            Rc<dyn Fn(String, String, Vec<Value>) -> Result<Value, String>>,
        >,
    ) -> Result<Self, String> {
        let (engine, module) = compile_hta_module(bytes)?;
        Ok(Self {
            mode: ProviderMode::Hta(Rc::new(HtaProviderState {
                engine,
                module,
                session: RefCell::new(None),
                host_handler,
                timeout: hta_timeout(),
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

    fn start(&self, manifest: &ExtensionManifest) -> Result<(), String> {
        if let ProviderMode::Hta(state) = &self.mode {
            return state.start(manifest);
        }
        if !manifest.capabilities.is_empty() {
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

struct HtaPending {
    promise: Promise,
    deadline: Option<Instant>,
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
    pending: HashMap<u64, HtaPending>,
    host_promises: HashMap<u64, Promise>,
    deliveries: VecDeque<(u64, bool, Value)>,
}

struct HtaProviderState {
    engine: Engine,
    module: Module,
    session: RefCell<Option<HtaSession>>,
    host_handler: Option<Rc<dyn Fn(String, String, Vec<Value>) -> Result<Value, String>>>,
    timeout: Option<Duration>,
}

impl HtaProviderState {
    fn start(&self, manifest: &ExtensionManifest) -> Result<(), String> {
        if manifest.provider != "wasm" || manifest.abi != WasmAbi::HtaV1 {
            return Err("extension/manifest-mismatch: HTA Wasm provider requires :wasm/:hta.v1".into());
        }
        if !manifest.capabilities.is_empty() {
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
            return Err(format!("extension/start: session already exists for {}", manifest.namespace));
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
            .instances(1)
            .memories(1)
            .tables(1)
            .build();
        let mut store = Store::new(&self.engine, limits);
        store.limiter(|limits| limits);
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
            return Err(format!("extension/abi-version-unsupported: {}", manifest.namespace));
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
            pending: HashMap::new(),
            host_promises: HashMap::new(),
            deliveries: VecDeque::new(),
        });
        Ok(())
    }

    fn invoke(
        self: &Rc<Self>,
        manifest: &ExtensionManifest,
        export: &str,
        arguments: &[Value],
    ) -> Result<Value, String> {
        let promise = Promise::new();
        let task = {
            let mut session_ref = self.session.borrow_mut();
            let session = session_ref
                .as_mut()
                .ok_or_else(|| "hta/session-closed".to_owned())?;
            let request = hta::encode(&Value::Vector(
                vec![
                    Value::String(export.to_owned()),
                    Value::Vector(arguments.to_vec().into()),
                ]
                .into(),
            ))?;
            let task = execute_start(session, &request)?;
            if task <= 0 {
                return Err(format!("hta/start-failed: {}", manifest.namespace));
            }
            session.pending.insert(
                task as u64,
                HtaPending {
                    promise: promise.clone(),
                    deadline: self.timeout.map(|timeout| Instant::now() + timeout),
                },
            );
            task as u64
        };
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
        self.pump(manifest)?;
        Ok(Value::Promise(promise))
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
        let packed = call_i64(&mut session.store, &session.next_event, &[], "hta_next_event")?;
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
        hta::decode(&bytes)
            .map(Some)
            .map_err(|error| format!("hta/event-malformed: {error}"))
    }

    fn handle_event(self: &Rc<Self>, manifest: &ExtensionManifest, event: Value) -> Result<(), String> {
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
                let pending = self
                    .session
                    .borrow_mut()
                    .as_mut()
                    .and_then(|session| session.pending.remove(&task));
                if let Some(pending) = pending {
                    self.drop_task(task)?;
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
        let service_index = if values.len() == 8 { 5 } else { 3 };
        let service = string_value(values, service_index, "service")?;
        let method = string_value(values, service_index + 1, "method")?;
        let arguments = match values.get(service_index + 2) {
            Some(Value::Vector(arguments)) => arguments.iter().cloned().collect::<Vec<_>>(),
            Some(Value::List(arguments)) => arguments.iter().cloned().collect::<Vec<_>>(),
            _ => return Err("hta/host-call-malformed: arguments".into()),
        };
        if !manifest.permits_host_call(&service, &method) {
            self.queue_delivery(call, false, host_error("hta/host-call-denied", &service, &method));
            return Ok(());
        }
        let Some(handler) = self.host_handler.clone() else {
            self.queue_delivery(call, false, host_error("host/unavailable", &service, &method));
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
                                Value::String(error.message()),
                            ),
                            PromiseState::Pending => {}
                        }
                    }
                }));
            }
            Ok(value) => self.queue_delivery(call, true, value),
            Err(error) => self.queue_delivery(call, false, Value::String(error)),
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
                    value,
                ]
                .into(),
            ))?;
            let mut session_ref = self.session.borrow_mut();
            let session = session_ref
                .as_mut()
                .ok_or_else(|| "hta/session-closed".to_owned())?;
            execute_deliver(session, &frame)?;
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
        if !self.is_pending(task) {
            return Ok(());
        }
        self.cancel_task(task)?;
        self.session
            .borrow_mut()
            .as_mut()
            .ok_or_else(|| "hta/session-closed".to_owned())?
            .pending
            .remove(&task);
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
        let _ = call_i32(
            &mut session.store,
            &session.drop_task,
            &[Val::I64(task as i64)],
            "hta_drop_task",
        )?;
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
            let _ = self.cancel_task(task);
            let pending = self
                .session
                .borrow_mut()
                .as_mut()
                .and_then(|session| session.pending.remove(&task));
            if let Some(pending) = pending {
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
                    .map(|(_, pending)| pending.promise)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for promise in pending {
            promise.reject(error.clone());
        }
    }

    fn shutdown(&self, _manifest: &ExtensionManifest) {
        let pending = self
            .session
            .borrow_mut()
            .take()
            .map(|session| session.pending.into_values().map(|pending| pending.promise).collect::<Vec<_>>())
            .unwrap_or_default();
        for promise in pending {
            promise.reject("hta/session-closed");
        }
    }
}

fn compile_hta_module(bytes: &[u8]) -> Result<(Engine, Module), String> {
    let mut config = Config::new();
    config.consume_fuel(true);
    let engine =
        Engine::new(&config).map_err(|error| format!("extension/engine-unavailable: {error}"))?;
    let module =
        Module::new(&engine, bytes).map_err(|error| format!("extension/module-invalid: {error}"))?;
    for import in module.imports() {
        if import.module() != "env"
            || !matches!(
                import.name(),
                "hara_random_fill" | "hara_time_ms" | "hara_time_ns"
            )
        {
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
        return Err(format!("extension/abi-type-unsupported: {name} has an invalid signature"));
    }
    Ok(())
}

fn call_i32(
    store: &mut Store<StoreLimits>,
    function: &Func,
    arguments: &[Val],
    name: &str,
) -> Result<i32, String> {
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
    Value::Map(
        [
            (
                Value::Keyword("code".into()),
                Value::Keyword(code.into()),
            ),
            (
                Value::Keyword("message".into()),
                Value::String(format!("{service}/{method}")),
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
    let type_error = || format!("extension/type-error: {export} expects {wire_type}");
    match (wire_type, value) {
        ("i32", Value::Number(value)) => i32::try_from(*value)
            .map(Val::I32)
            .map_err(|_| type_error()),
        ("i64", Value::Number(value)) => Ok(Val::I64(*value)),
        ("f32", Value::Float(value)) => Ok(Val::F32((*value as f32).to_bits())),
        ("f32", Value::Number(value)) => Ok(Val::F32((*value as f32).to_bits())),
        ("f64", Value::Float(value)) => Ok(Val::F64(value.to_bits())),
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
        ("f32", Some(Val::F32(value))) => Ok(Value::Float(f32::from_bits(value) as f64)),
        ("f64", Some(Val::F64(value))) => Ok(Value::Float(f64::from_bits(value))),
        ("boolean", Some(Val::I32(value))) => Ok(Value::Bool(value != 0)),
        _ => Err(format!(
            "extension/abi-type-unsupported: {export} -> {wire_type}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use crate::extension::{ExtensionManifest, Value, WasmExtension};

    use super::WasmtimeExtensionProvider;

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
}
