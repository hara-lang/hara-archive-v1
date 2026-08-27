//! The synchronous stack machine.
//!
//! Validated programs reject impossible indexes and avoid per-instruction allocation.
//!
//! VM closures and static calls stay inside one machine. Call frames and
//! compact scalar slots avoid native callback recursion and boxed integer
//! traffic on the hot path; closures are converted to shared runtime values
//! only when they escape through the public value boundary.
//!
//! Exceptions (milestone 3): every failure routes through
//! [`Machine::raise`], which unwinds to the innermost covering try-table
//! entry. Catch dispatch and binding identity come from the shared
//! `core::catch_matches`/`core::caught_error` boundary, so thrown values
//! and runtime-error strings behave exactly as in the tree evaluator.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::rc::{Rc, Weak};

use super::error::VmError;
use super::fiber::{VmFiber, VmFiberState};
use super::frame::Frame;
use super::opcode::Instruction;
use super::program::{FunctionPrototype, Program};
use super::slot::{VmClosure, VmMultiArity, VmSlot};
use crate::core::{
    call_value, native_fiber_function, with_namespace_registry, Cont, Promise, PromiseState, Step,
    Value,
};
use crate::task::promise::settle_result;

#[path = "machine/async_runtime.rs"]
mod async_runtime;
#[path = "machine/constants.rs"]
mod constants;
#[path = "machine/coroutine_runtime.rs"]
mod coroutine_runtime;
use constants::{constant_named_fields, constant_string};
#[path = "machine/dispatch.rs"]
mod dispatch;
use dispatch::Dispatch;
#[path = "machine/globals.rs"]
mod globals;
use async_runtime::{async_result, async_result_from_outcome};
#[cfg(feature = "bytecode-instrumentation")]
#[path = "machine/instrumentation.rs"]
pub mod instrumentation;
#[cfg(feature = "bytecode-observation")]
#[path = "machine/observation.rs"]
pub mod observation;

/// Terminal state of a machine run. Suspension variants belong to the
/// later async milestone; adding them does not change instruction
/// dispatch, only the set of exit points.
pub enum VmOutcome {
    Returned(Value),
    Failed(VmError),
    Suspended(Promise),
    Yielded(Value),
}

/// A synchronous interpreter for one function of a validated [`Program`].
pub struct Machine {
    program: Rc<Program>,
    function: usize,
    frame: Frame,
    stack: Vec<VmSlot>,
    scratch: Vec<Value>,
    calls: Vec<SavedFrame>,
    free_locals: Vec<Vec<VmSlot>>,
    free_args: Vec<Vec<VmSlot>>,
    vm_globals: HashMap<usize, VmSlot>,
    next_closure_identity: u64,
    scheduler: Weak<RefCell<AsyncScheduler>>,
    scheduler_owner: Option<Rc<RefCell<AsyncScheduler>>>,
    ip: usize,
    #[cfg(feature = "tracing-jit")]
    jit: crate::jit::runtime::JitRuntime,
    #[cfg(feature = "tracing-jit")]
    jit_path: Vec<(usize, u32)>,
    #[cfg(feature = "tracing-jit")]
    jit_suppressed_range: Option<(usize, u32, u32)>,
    #[cfg(feature = "tracing-jit")]
    jit_loop_entries: HashMap<(usize, u32), Vec<crate::jit::TraceValue>>,
    #[cfg(feature = "tracing-jit")]
    jit_status_function: usize,
    #[cfg(feature = "tracing-jit")]
    jit_function_disabled: bool,
}

#[cfg(feature = "tracing-jit")]
struct CachedJit {
    program: Weak<Program>,
    runtime: crate::jit::runtime::JitRuntime,
}

#[cfg(feature = "tracing-jit")]
thread_local! {
    static PROGRAM_JITS: RefCell<HashMap<usize, CachedJit>> = RefCell::new(HashMap::new());
}

#[cfg(feature = "tracing-jit")]
const MAX_PROGRAM_JITS: usize = 128;

#[cfg(feature = "tracing-jit")]
fn program_key(program: &Rc<Program>) -> usize {
    Rc::as_ptr(program) as usize
}

#[cfg(feature = "tracing-jit")]
fn take_program_jit(program: &Rc<Program>) -> crate::jit::runtime::JitRuntime {
    PROGRAM_JITS.with(|cache| {
        cache
            .borrow_mut()
            .remove(&program_key(program))
            .filter(|cached| {
                cached
                    .program
                    .upgrade()
                    .is_some_and(|owner| Rc::ptr_eq(&owner, program))
            })
            .map(|cached| cached.runtime)
            .unwrap_or_default()
    })
}

#[cfg(feature = "tracing-jit")]
fn store_program_jit(program: &Rc<Program>, runtime: crate::jit::runtime::JitRuntime) {
    PROGRAM_JITS.with(|cache| {
        let mut cache = cache.borrow_mut();
        cache.retain(|_, cached| cached.program.strong_count() > 0);
        if cache.len() >= MAX_PROGRAM_JITS && !cache.contains_key(&program_key(program)) {
            // Program keys are pointer identities. Bound this TLS cache so a
            // long-lived embedder compiling many retained programs cannot grow
            // it indefinitely; all entries are merely optimization state.
            cache.clear();
        }
        cache.insert(
            program_key(program),
            CachedJit {
                program: Rc::downgrade(program),
                runtime,
            },
        );
    });
}

struct SavedFrame {
    function: usize,
    frame: Frame,
    call_ip: usize,
}

struct AsyncChild {
    machine: Machine,
    result: crate::task::promise::WeakPromise,
    pending: Promise,
}

#[derive(Default)]
struct AsyncScheduler {
    next_id: u64,
    children: HashMap<u64, AsyncChild>,
    ready: VecDeque<(u64, PromiseState)>,
    polling: bool,
}

impl Machine {
    #[cfg(feature = "tracing-jit")]
    pub(super) fn attach_cached_jit(&mut self) {
        self.jit = take_program_jit(&self.program);
    }

    #[cfg(feature = "tracing-jit")]
    pub(super) fn detach_cached_jit(&mut self) {
        store_program_jit(&self.program.clone(), std::mem::take(&mut self.jit));
    }

    /// The machine for the program's entry function.
    pub fn entry(program: Rc<Program>) -> Machine {
        let index = usize::from(program.entry);
        let local_count = usize::from(program.functions[index].local_count);
        let max_stack = usize::from(program.functions[index].max_stack);
        let scheduler = Rc::new(RefCell::new(AsyncScheduler::default()));
        Machine {
            program,
            function: index,
            frame: Frame::entry(local_count),
            stack: Vec::with_capacity(max_stack),
            scratch: Vec::new(),
            calls: Vec::new(),
            free_locals: Vec::new(),
            free_args: Vec::new(),
            vm_globals: HashMap::new(),
            next_closure_identity: 0,
            scheduler: Rc::downgrade(&scheduler),
            scheduler_owner: Some(scheduler),
            ip: 0,
            #[cfg(feature = "tracing-jit")]
            jit: crate::jit::runtime::JitRuntime::default(),
            #[cfg(feature = "tracing-jit")]
            jit_path: Vec::new(),
            #[cfg(feature = "tracing-jit")]
            jit_suppressed_range: None,
            #[cfg(feature = "tracing-jit")]
            jit_loop_entries: HashMap::new(),
            #[cfg(feature = "tracing-jit")]
            jit_status_function: usize::MAX,
            #[cfg(feature = "tracing-jit")]
            jit_function_disabled: false,
        }
    }

    /// The machine for a function call: `args` fill the parameter slots,
    /// `captures` the capture slots directly above them.
    pub fn call(
        program: Rc<Program>,
        prototype: u16,
        args: Vec<Value>,
        captures: Vec<Value>,
    ) -> Machine {
        Machine::call_slots(
            program,
            prototype,
            args.into_iter().map(VmSlot::from).collect(),
            captures.into_iter().map(VmSlot::from).collect(),
        )
    }

    fn call_slots(
        program: Rc<Program>,
        prototype: u16,
        args: Vec<VmSlot>,
        captures: Vec<VmSlot>,
    ) -> Machine {
        let scheduler = Rc::new(RefCell::new(AsyncScheduler::default()));
        Self::call_slots_with_scheduler(
            program,
            prototype,
            args,
            captures,
            Rc::downgrade(&scheduler),
            Some(scheduler),
        )
    }

    fn call_slots_with_scheduler(
        program: Rc<Program>,
        prototype: u16,
        mut args: Vec<VmSlot>,
        captures: Vec<VmSlot>,
        scheduler: Weak<RefCell<AsyncScheduler>>,
        scheduler_owner: Option<Rc<RefCell<AsyncScheduler>>>,
    ) -> Machine {
        let index = usize::from(prototype);
        let proto = &program.functions[index];
        let mut arity = usize::from(proto.arity);
        if proto.variadic {
            // The rest parameter occupies the slot directly above the
            // fixed parameters (captures sit above it): pack the
            // remaining arguments into a list there, exactly like
            // `call_function` binds `& rest`.
            let fixed = arity.min(args.len());
            let rest = args
                .split_off(fixed)
                .into_iter()
                .map(|value| Machine::into_value(program.clone(), value))
                .collect();
            args.push(Value::List(rest).into());
            arity = fixed + 1;
        }
        Machine {
            frame: Frame::call(usize::from(proto.local_count), arity, args, captures, 0),
            stack: Vec::with_capacity(usize::from(proto.max_stack)),
            program,
            function: index,
            scratch: Vec::new(),
            calls: Vec::new(),
            free_locals: Vec::new(),
            free_args: Vec::new(),
            vm_globals: HashMap::new(),
            next_closure_identity: 0,
            scheduler,
            scheduler_owner,
            ip: 0,
            #[cfg(feature = "tracing-jit")]
            jit: crate::jit::runtime::JitRuntime::default(),
            #[cfg(feature = "tracing-jit")]
            jit_path: Vec::new(),
            #[cfg(feature = "tracing-jit")]
            jit_suppressed_range: None,
            #[cfg(feature = "tracing-jit")]
            jit_loop_entries: HashMap::new(),
            #[cfg(feature = "tracing-jit")]
            jit_status_function: usize::MAX,
            #[cfg(feature = "tracing-jit")]
            jit_function_disabled: false,
        }
    }

    fn into_value(program: Rc<Program>, slot: VmSlot) -> Value {
        match slot {
            VmSlot::Number(value) => Value::Number(value),
            VmSlot::Bool(value) => Value::Bool(value),
            VmSlot::Nil => Value::Nil,
            VmSlot::Value(value) => Rc::try_unwrap(value).unwrap_or_else(|value| (*value).clone()),
            VmSlot::InlineClosure { prototype, .. } => Self::closure_value(
                program,
                Rc::new(VmClosure {
                    prototype,
                    captures: Vec::new(),
                }),
            ),
            VmSlot::Closure(closure) => Self::closure_value(program, closure),
            VmSlot::MultiArity(dispatch) => {
                let functions = dispatch
                    .clauses
                    .iter()
                    .cloned()
                    .map(
                        |closure| match Self::closure_value(program.clone(), closure) {
                            Value::Function(function) => function,
                            _ => unreachable!(),
                        },
                    )
                    .collect();
                crate::core::arity_dispatcher(&dispatch.name, functions, false)
            }
        }
    }

    fn callable_key(value: &Value) -> Option<usize> {
        match value {
            Value::Function(function) => Some(Rc::as_ptr(function) as usize),
            _ => None,
        }
    }

    fn remember_vm_global(&mut self, value: &Value, slot: VmSlot) {
        if let Some(key) = Self::callable_key(value) {
            self.vm_globals.insert(key, slot);
        }
    }

    fn enter_callable(
        &mut self,
        program: &Rc<Program>,
        callee: VmSlot,
        mut args: Vec<VmSlot>,
    ) -> Result<(), String> {
        match callee {
            VmSlot::InlineClosure { prototype, .. } => {
                self.check_arity(program, prototype, args.len())?;
                self.enter_or_spawn(program, prototype, args, Vec::new());
                Ok(())
            }
            VmSlot::Closure(closure) => {
                self.check_arity(program, closure.prototype, args.len())?;
                self.enter_or_spawn(program, closure.prototype, args, closure.captures.clone());
                Ok(())
            }
            VmSlot::MultiArity(dispatch) => {
                let closure = dispatch
                    .clauses
                    .iter()
                    .find(|closure| {
                        let proto = &program.functions[usize::from(closure.prototype)];
                        (!proto.variadic && usize::from(proto.arity) == args.len())
                            || (proto.variadic && args.len() >= usize::from(proto.arity))
                    })
                    .cloned()
                    .ok_or_else(|| format!("{} has no arity {}", dispatch.name, args.len()))?;
                self.enter_or_spawn(program, closure.prototype, args, closure.captures.clone());
                Ok(())
            }
            value => {
                let callee = Self::into_value(program.clone(), value);
                let runtime_args = args
                    .drain(..)
                    .map(|value| Self::into_value(program.clone(), value))
                    .collect();
                self.free_args.push(args);
                let value = if let Some(position) = program.functions[self.function]
                    .source_map
                    .position(self.ip)
                {
                    crate::core::with_exception_site(
                        crate::core::ExceptionSite {
                            namespace: program.namespace.clone(),
                            resource: None,
                            line: position.line,
                            column: position.column,
                        },
                        || call_value(callee, runtime_args),
                    )?
                } else {
                    call_value(callee, runtime_args)?
                };
                self.stack.push(value.into());
                self.ip += 1;
                Ok(())
            }
        }
    }

    fn enter_or_spawn(
        &mut self,
        program: &Rc<Program>,
        prototype: u16,
        args: Vec<VmSlot>,
        captures: Vec<VmSlot>,
    ) {
        if program.functions[usize::from(prototype)].async_function {
            let mut child = Machine::call_slots(program.clone(), prototype, args, captures);
            child.vm_globals = self.vm_globals.clone();
            child.next_closure_identity = self.next_closure_identity;
            self.stack
                .push(Value::Promise(self.spawn_async(child)).into());
            self.ip += 1;
        } else {
            self.enter_prototype(program, prototype, args, captures);
        }
    }

    fn check_arity(&self, program: &Program, prototype: u16, argc: usize) -> Result<(), String> {
        let proto = &program.functions[usize::from(prototype)];
        let arity = usize::from(proto.arity);
        if (!proto.variadic && argc != arity) || (proto.variadic && argc < arity) {
            let expectation = if proto.variadic {
                format!("at least {arity}")
            } else {
                arity.to_string()
            };
            return Err(format!("function expects {expectation} arguments"));
        }
        Ok(())
    }

    fn enter_prototype(
        &mut self,
        program: &Program,
        prototype: u16,
        mut args: Vec<VmSlot>,
        captures: Vec<VmSlot>,
    ) {
        let proto = &program.functions[usize::from(prototype)];
        let mut frame_arity = usize::from(proto.arity);
        if proto.variadic {
            let fixed = frame_arity.min(args.len());
            let rest = args
                .split_off(fixed)
                .into_iter()
                .map(|value| Self::into_value(self.program.clone(), value))
                .collect();
            args.push(Value::List(rest).into());
            frame_arity = fixed + 1;
        }
        let locals = self.free_locals.pop().unwrap_or_default();
        let frame = Frame::call_reusing(
            locals,
            usize::from(proto.local_count),
            frame_arity,
            &mut args,
            captures,
            self.stack.len(),
        );
        self.free_args.push(args);
        let caller = std::mem::replace(&mut self.frame, frame);
        self.calls.push(SavedFrame {
            function: self.function,
            frame: caller,
            call_ip: self.ip,
        });
        self.function = usize::from(prototype);
        self.ip = 0;
    }

    /// Enters a synchronous, capture-free, fixed-arity static target by
    /// transferring its operands straight into a recycled local frame.
    fn enter_static_direct(&mut self, program: &Program, prototype: u16, argc: u8) {
        let proto = &program.functions[usize::from(prototype)];
        debug_assert_eq!(proto.capture_count, 0);
        debug_assert!(!proto.async_function);
        debug_assert!(!proto.variadic);
        debug_assert_eq!(usize::from(proto.arity), usize::from(argc));
        let locals = self.free_locals.pop().unwrap_or_default();
        let frame = Frame::call_static_reusing(
            locals,
            usize::from(proto.local_count),
            &mut self.stack,
            usize::from(argc),
        );
        let caller = std::mem::replace(&mut self.frame, frame);
        self.calls.push(SavedFrame {
            function: self.function,
            frame: caller,
            call_ip: self.ip,
        });
        self.function = usize::from(prototype);
        self.ip = 0;
    }

    /// Runs the function to completion or failure.
    pub fn run(&mut self) -> VmOutcome {
        let program = self.program.clone();
        // Guest calls use the explicit frame stack below, so instruction
        // dispatch may be inlined without increasing native recursion depth.
        loop {
            let Some(function) = program.functions.get(self.function) else {
                return VmOutcome::Failed(VmError::new("function index out of range", 0, None));
            };
            let Some(instruction) = function.code.get(self.ip) else {
                return VmOutcome::Failed(self.error(function, "instruction pointer out of range"));
            };
            #[cfg(feature = "tracing-jit")]
            {
                // A cached fully-disabled function takes the shortest fallback
                // path: one predictable flag check per instruction. Recompute
                // only if control moved to another function.
                if !self.jit_function_disabled || self.jit_status_function != self.function {
                    if self.jit_status_function != self.function {
                        self.jit_function_disabled = self
                            .jit
                            .function_is_fully_disabled(&program, self.function as u16);
                        self.jit_status_function = self.function;
                    }
                    if !self.jit_function_disabled {
                        let instruction = self.ip as u32;
                        let suppressed = self.jit_suppressed_range.is_some_and(
                            |(function, header, backedge)| {
                                function == self.function
                                    && instruction >= header
                                    && instruction <= backedge
                            },
                        );
                        if !suppressed {
                            self.jit_suppressed_range = None;
                            self.jit_path.push((self.function, instruction));
                        }
                    }
                }
            }
            match self.dispatch(&program, function, instruction) {
                Dispatch::Next(ip) | Dispatch::Unwound(ip) => {
                    #[cfg(feature = "tracing-jit")]
                    let mut next_ip = ip;
                    #[cfg(not(feature = "tracing-jit"))]
                    let next_ip = ip;
                    #[cfg(feature = "tracing-jit")]
                    if !self.jit_function_disabled && ip <= self.ip {
                        let header = ip as u32;
                        if self.jit.is_disabled(self.function as u16, header) {
                            self.jit_suppressed_range =
                                Some((self.function, header, self.ip as u32));
                        } else {
                            let (mut locals, writable) = self.frame.trace_locals();
                            let recording_locals = self
                                .jit_loop_entries
                                .get(&(self.function, header))
                                .cloned()
                                .unwrap_or_else(|| locals.clone());
                            let path_start = self
                                .jit_path
                                .iter()
                                .rposition(|entry| *entry == (self.function, header));
                            let path = path_start.map_or_else(Vec::new, |start| {
                                self.jit_path[start..]
                                    .iter()
                                    .map(|(_, instruction)| *instruction)
                                    .collect()
                            });
                            if let Some(snapshot) = self.jit.backedge(
                                &program,
                                self.function as u16,
                                self.ip as u32,
                                header,
                                &path,
                                &recording_locals,
                                &mut locals,
                            ) {
                                self.frame.apply_trace_locals(&snapshot.locals, &writable);
                                locals = snapshot.locals;
                                next_ip = snapshot.instruction as usize;
                            }
                            self.jit_loop_entries
                                .insert((self.function, header), locals);
                            if self.jit.is_disabled(self.function as u16, header) {
                                self.jit_suppressed_range =
                                    Some((self.function, header, self.ip as u32));
                                self.jit_status_function = usize::MAX;
                            }
                        }
                        self.jit_path.clear();
                    }
                    self.ip = next_ip;
                }
                Dispatch::Call { callee, args } => {
                    #[cfg(feature = "tracing-jit")]
                    {
                        self.jit_path.clear();
                        self.jit_loop_entries.clear();
                    }
                    if let Err(message) = self.enter_callable(&program, callee, args) {
                        match self.raise(function, message) {
                            Ok(target) => self.ip = target,
                            Err(error) => return VmOutcome::Failed(error),
                        }
                    }
                }
                Dispatch::CallStatic {
                    prototype,
                    args,
                    captures,
                } => {
                    #[cfg(feature = "tracing-jit")]
                    {
                        self.jit_path.clear();
                        self.jit_loop_entries.clear();
                    }
                    self.enter_or_spawn(&program, prototype, args, captures)
                }
                Dispatch::CallStaticDirect { prototype, argc } => {
                    #[cfg(feature = "tracing-jit")]
                    {
                        self.jit_path.clear();
                        self.jit_loop_entries.clear();
                    }
                    self.enter_static_direct(&program, prototype, argc)
                }
                Dispatch::Returned(value) => {
                    #[cfg(feature = "tracing-jit")]
                    {
                        self.jit_path.clear();
                        self.jit_loop_entries.clear();
                    }
                    self.stack.truncate(self.frame.base());
                    if let Some(caller) = self.calls.pop() {
                        self.function = caller.function;
                        let completed = std::mem::replace(&mut self.frame, caller.frame);
                        self.free_locals.push(completed.into_locals());
                        self.ip = caller.call_ip + 1;
                        self.stack.push(value);
                    } else {
                        return VmOutcome::Returned(Self::into_value(program.clone(), value));
                    }
                }
                Dispatch::Suspended(promise) => return VmOutcome::Suspended(promise),
                Dispatch::Yielded(value) => return VmOutcome::Yielded(value),
                Dispatch::Failed(error) => return VmOutcome::Failed(error),
            }
        }
    }

    /// Pops the callee and arguments for a Call instruction.
    fn collect_call(&mut self, argc: u8) -> Result<(VmSlot, Vec<VmSlot>), String> {
        let argc = usize::from(argc);
        if self.stack.len() < argc + 1 {
            return Err("stack underflow".to_string());
        }
        let mut args = self.free_args.pop().unwrap_or_default();
        args.extend(self.stack.drain(self.stack.len() - argc..));
        let callee = self.stack.pop().expect("callee checked above");
        Ok((callee, args))
    }

    /// Collects the arguments and capture slots for a CallStatic
    /// instruction; the nested machine runs from the thin `run` loop.
    fn collect_call_static(
        &mut self,
        program: &Program,
        function: &FunctionPrototype,
        prototype: u16,
        argc: u8,
    ) -> Result<(u16, Vec<VmSlot>, Vec<VmSlot>), String> {
        let argc = usize::from(argc);
        if self.stack.len() < argc {
            return Err("stack underflow".to_string());
        }
        let Some(proto) = program.functions.get(usize::from(prototype)) else {
            return Err(format!("callstatic target {prototype} out of range"));
        };
        let capture_count = usize::from(proto.capture_count);
        let mut args = self.free_args.pop().unwrap_or_default();
        args.extend(self.stack.drain(self.stack.len() - argc..));
        let capture_base = usize::from(function.arity) + usize::from(function.variadic);
        let Some(captures) = self.frame.slot_range(capture_base, capture_count) else {
            return Err("capture slots out of range".to_string());
        };
        Ok((prototype, args, captures))
    }

    /// Executes the Closure instruction: builds a plain
    /// `core::Value::Function` whose native callback re-enters
    /// [`Machine::call`]. Kept out of the dispatch loop
    /// (`#[inline(never)]`) so the hot `run` frame stays small — guest
    /// recursion maps onto native stack depth through Call/CallStatic,
    /// and this arm carries large transient locals.
    #[inline(never)]
    fn exec_closure(
        &mut self,
        program: &Rc<Program>,
        prototype: u16,
        captures: u8,
    ) -> Result<(), String> {
        let captures = usize::from(captures);
        if self.stack.len() < captures {
            return Err("stack underflow".to_string());
        }
        let Some(_proto) = program.functions.get(usize::from(prototype)) else {
            return Err(format!("closure prototype {prototype} out of range"));
        };
        if captures == 0 {
            let identity = self.next_closure_identity;
            self.next_closure_identity = self.next_closure_identity.wrapping_add(1);
            self.stack.push(VmSlot::InlineClosure {
                prototype,
                identity,
            });
        } else {
            let captured = self.stack.split_off(self.stack.len() - captures);
            self.stack.push(VmSlot::Closure(Rc::new(VmClosure {
                prototype,
                captures: captured,
            })));
        }
        Ok(())
    }

    /// Routes a failure through the static handler table: the innermost
    /// entry covering the failing instruction gets first catch dispatch,
    /// then outer entries. Returns the instruction to continue at, or the
    /// terminal error when no entry handles it.
    #[cold]
    #[inline(never)]
    fn raise(
        &mut self,
        _function: &FunctionPrototype,
        message: impl Into<String>,
    ) -> Result<usize, VmError> {
        let message = message.into();
        loop {
            let function = &self.program.functions[self.function];
            let error_ip = self.ip;
            for entry in function.handlers.iter().rev() {
                let (start, end) = (entry.start as usize, entry.end as usize);
                if error_ip < start || error_ip >= end {
                    continue;
                }
                let depth = self.frame.base() + usize::from(entry.depth);
                if self.stack.len() < depth {
                    return Err(self.error(function, "handler stack depth out of range"));
                }
                for catch in &entry.catches {
                    if crate::core::catch_matches(&message, &catch.class) {
                        self.stack.truncate(depth);
                        let value = crate::core::caught_error(&message);
                        if !self.frame.store(catch.binding, value.into()) {
                            return Err(self.error(function, "catch binding slot out of range"));
                        }
                        return Ok(catch.target as usize);
                    }
                }
                if let Some(finally) = entry.finally {
                    let (Some(value_slot), Some(flag_slot)) =
                        (entry.pending_value, entry.pending_error)
                    else {
                        return Err(self.error(function, "handler pending slots missing"));
                    };
                    self.stack.truncate(depth);
                    if !self
                        .frame
                        .store(value_slot, crate::core::caught_error(&message).into())
                        || !self.frame.store(flag_slot, Value::Bool(true).into())
                    {
                        return Err(self.error(function, "pending slot out of range"));
                    }
                    return Ok(finally as usize);
                }
            }

            let Some(caller) = self.calls.pop() else {
                return Err(VmError::new(
                    message,
                    error_ip as u32,
                    function.source_map.position(error_ip),
                ));
            };
            self.stack.truncate(self.frame.base());
            self.function = caller.function;
            let completed = std::mem::replace(&mut self.frame, caller.frame);
            self.free_locals.push(completed.into_locals());
            self.ip = caller.call_ip;
        }
    }

    fn error(&self, function: &FunctionPrototype, message: impl Into<String>) -> VmError {
        VmError::new(
            message,
            self.ip as u32,
            function.source_map.position(self.ip),
        )
    }
}

impl Machine {
    /// Continues a machine stopped at `Await`. The settlement is applied at
    /// the suspended instruction so ordinary exception handlers see a
    /// rejected promise exactly like a guest throw.
    pub fn resume(&mut self, state: PromiseState) -> VmOutcome {
        let Some(function) = self.program.functions.get(self.function).cloned() else {
            return VmOutcome::Failed(VmError::new("function index out of range", 0, None));
        };
        let protocol_deref = match function.code.get(self.ip) {
            Some(Instruction::ProtocolCall { target, argc }) if *argc == 1 => {
                constant_string(&self.program, *target)
                    .is_some_and(|name| name == "std.protocol.ideref.IDeref/deref")
            }
            _ => false,
        };
        if !protocol_deref && !matches!(function.code.get(self.ip), Some(Instruction::Await)) {
            return VmOutcome::Failed(
                self.error(&function, "VM is not suspended at await or deref"),
            );
        }
        match state {
            PromiseState::Pending => {
                let promise = match self.stack.last().and_then(VmSlot::runtime_value) {
                    Some(Value::Promise(promise)) => promise,
                    _ => {
                        return VmOutcome::Failed(self.error(
                            &function,
                            if protocol_deref {
                                "deref expects a promise"
                            } else {
                                "await expects a promise"
                            },
                        ))
                    }
                };
                return VmOutcome::Suspended(promise);
            }
            PromiseState::Fulfilled(value) => {
                self.stack.pop();
                self.stack.push(value.into());
                self.ip += 1;
            }
            PromiseState::Rejected(error) => {
                self.stack.pop();
                let message = if protocol_deref {
                    crate::core::promise_rejection_error(error)
                } else {
                    error.message()
                };
                match self.raise(&function, message) {
                    Ok(target) => self.ip = target,
                    Err(error) => return VmOutcome::Failed(error),
                }
            }
        }
        self.run()
    }

    pub fn resume_yield(&mut self, value: Value) -> VmOutcome {
        let Some(function) = self.program.functions.get(self.function).cloned() else {
            return VmOutcome::Failed(VmError::new("function index out of range", 0, None));
        };
        if !matches!(function.code.get(self.ip), Some(Instruction::Yield)) {
            return VmOutcome::Failed(self.error(&function, "VM is not suspended at yield"));
        }
        self.stack.push(value.into());
        self.ip += 1;
        self.run()
    }
}

fn run_entry(program: Rc<Program>) -> Result<Value, VmError> {
    let mut machine = Machine::entry(program.clone());
    #[cfg(feature = "tracing-jit")]
    {
        machine.jit = take_program_jit(&program);
    }
    let outcome = machine.run();
    #[cfg(feature = "tracing-jit")]
    store_program_jit(&program, machine.jit);
    match outcome {
        VmOutcome::Returned(value) => Ok(value),
        VmOutcome::Failed(error) => Err(error),
        VmOutcome::Suspended(_) => Err(VmError::new(
            "VM fiber suspended on an unresolved promise",
            0,
            None,
        )),
        VmOutcome::Yielded(_) => Err(VmError::new(
            "coroutine/yield used outside of a coroutine",
            0,
            None,
        )),
    }
}

#[cfg(feature = "tracing-jit")]
fn cached_jit_runtime<R>(
    program: &Rc<Program>,
    access: impl FnOnce(&crate::jit::runtime::JitRuntime) -> R,
) -> Option<R> {
    PROGRAM_JITS.with(|cache| {
        cache
            .borrow()
            .get(&program_key(program))
            .and_then(|cached| {
                cached
                    .program
                    .upgrade()
                    .map(|owner| (owner, &cached.runtime))
            })
            .filter(|(owner, _)| Rc::ptr_eq(owner, program))
            .map(|(_, runtime)| access(runtime))
    })
}

#[cfg(all(test, feature = "tracing-jit"))]
pub(crate) fn cached_trace_count(program: &Rc<Program>) -> usize {
    cached_jit_runtime(program, crate::jit::runtime::JitRuntime::compiled_count).unwrap_or(0)
}

#[cfg(all(test, feature = "tracing-jit"))]
pub(crate) fn active_compiled_trace_count() -> usize {
    PROGRAM_JITS.with(|cache| {
        cache
            .borrow()
            .values()
            .filter(|cached| cached.program.strong_count() > 0)
            .map(|cached| cached.runtime.compiled_count())
            .sum()
    })
}

#[cfg(all(test, feature = "tracing-jit"))]
pub(crate) fn active_jit_telemetry() -> Vec<crate::jit::JitTelemetry> {
    PROGRAM_JITS.with(|cache| {
        cache
            .borrow()
            .values()
            .filter(|cached| cached.program.strong_count() > 0)
            .map(|cached| cached.runtime.telemetry())
            .collect()
    })
}

#[cfg(feature = "tracing-jit")]
pub(crate) fn cached_jit_telemetry(program: &Rc<Program>) -> crate::jit::JitTelemetry {
    cached_jit_runtime(program, crate::jit::runtime::JitRuntime::telemetry).unwrap_or_default()
}

/// Executes a validated program's entry function.
///
/// Programs produced by [`crate::vm::compile_source`] are already
/// validated; callers constructing programs by hand must run
/// [`crate::vm::validate`] first. Either way the machine reports
/// [`VmError`] rather than panicking on malformed state. When no
/// namespace registry is active the program runs against a throwaway
/// `user` registry, so same-program `def`/`defn`/`defstruct` can intern
/// without touching caller state (issue #223).
pub fn execute_program(program: Rc<Program>) -> Result<Value, VmError> {
    if crate::core::namespace_registry().is_ok() {
        return run_entry(program);
    }
    let registry = crate::kernel::NamespaceRegistry::new("user");
    with_namespace_registry(&registry, || run_entry(program))
}

/// Executes a program against a caller's namespace registry: globals
/// intern into it and resolve from it, with no env bridge, snapshot, or
/// refresh (issue #223).
pub fn execute_program_with_globals(
    program: Rc<Program>,
    globals: &crate::kernel::NamespaceRegistry<Value>,
) -> Result<Value, VmError> {
    with_namespace_registry(globals, || run_entry(program))
}
