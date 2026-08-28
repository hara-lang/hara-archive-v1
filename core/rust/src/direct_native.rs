//! Opt-in native-substrate execution for validated Hara bytecode.
//!
//! The name of this compatibility module is retained for the public
//! `direct-native` feature, but it is deliberately not a second Hara compiler.
//! Hara source is compiled to [`crate::vm::Program`] and ordinary Hara
//! functions execute in the bytecode VM. This module only owns the native
//! execution boundary and its telemetry: canonical `std.native.*`,
//! `std.protocol.*`, and Rust-owned evaluator primitives are invoked by the VM
//! as Rust callouts. The optional Cranelift tracing tier remains attached to
//! the VM's real basic-loop path through `crate::jit`.

#![cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::core::Value;
use crate::vm::{Program, VmFiber};

/// A program which has crossed the native execution validation boundary.
///
/// Compiler output is already validated by the compiler's `finish` step and
/// artifact output is already validated by `decode_program`. Keeping that
/// fact in a separate type lets internal callers execute either form without
/// paying the structural validation cost again. Public entry points still
/// accept an ordinary `Rc<Program>` and validate it before constructing this
/// wrapper.
#[derive(Clone)]
pub(crate) struct ValidatedProgram {
    program: Rc<Program>,
}

impl ValidatedProgram {
    pub(crate) fn from_compiler(program: Rc<Program>) -> Self {
        Self { program }
    }

    pub(crate) fn from_artifact(program: Rc<Program>) -> Self {
        Self { program }
    }

    pub(crate) fn validate(program: Rc<Program>) -> Result<Self, String> {
        crate::vm::validate::validate(&program)
            .map_err(|error| format!("native backend received invalid bytecode: {error}"))?;
        Ok(Self { program })
    }

    pub(crate) fn program(&self) -> Rc<Program> {
        self.program.clone()
    }
}

/// The result of one native-substrate execution.
#[derive(Debug, Clone)]
pub struct NativeExecutionReport {
    /// The value returned by the VM entry function.
    pub value: Value,
    /// Number of Hara function prototypes validated for this bytecode unit.
    pub bytecode_functions: usize,
    /// Number of Hara VM instructions validated for this bytecode unit.
    pub bytecode_instructions: usize,
    /// Number of approved native/protocol/evaluator targets reached by the VM
    /// during this execution.
    pub native_target_calls: usize,
    /// Number of native-substrate VM entries represented by this report.
    pub invocations: usize,
}

/// Cumulative native-substrate counters for a reusable runtime owner.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NativeExecutionTelemetry {
    /// Cumulative number of validated Hara function prototypes encountered.
    pub bytecode_functions: usize,
    /// Cumulative number of validated Hara VM instructions encountered.
    pub bytecode_instructions: usize,
    /// Cumulative number of approved native/protocol/evaluator targets called.
    pub native_target_calls: usize,
    /// Cumulative number of native-substrate VM entries.
    pub invocations: usize,
}

impl NativeExecutionReport {
    /// Identifies the corrected two-stage backend in diagnostics and embedding
    /// telemetry. `Cranelift` is reserved for the VM's approved loop tier.
    pub const BACKEND: &'static str = "bytecode-vm-native-substrate";
}

#[derive(Default)]
struct NativeEngineState {
    bytecode_functions: Cell<usize>,
    bytecode_instructions: Cell<usize>,
    native_target_calls: Cell<usize>,
    invocations: Cell<usize>,
}

/// A captured native-substrate scope used by VM closures which outlive the
/// top-level entry call. It keeps the evaluator guard and target telemetry
/// active when a promise or callback resumes later on the same thread.
#[derive(Clone)]
pub(crate) struct NativeExecutionScope {
    state: Rc<NativeEngineState>,
}

thread_local! {
    static ACTIVE_NATIVE_ENGINE: RefCell<Option<Rc<NativeEngineState>>> = const { RefCell::new(None) };
}

/// A reusable native-substrate runtime boundary.
///
/// The engine owns the native execution boundary and cumulative telemetry.
/// Persistent source/artifact caching is configured by the Runtime owner so
/// namespace, provider, protocol, and mutable Var state remain isolated.
#[derive(Clone, Default)]
pub struct NativeEngine {
    state: Rc<NativeEngineState>,
}

impl NativeEngine {
    pub fn new() -> Self {
        Self::default()
    }

    /// Resets all cumulative counters. The operation is idempotent and does
    /// not affect any Runtime namespace or any already-created VM closure.
    pub fn reset(&self) {
        self.state.bytecode_functions.set(0);
        self.state.bytecode_instructions.set(0);
        self.state.native_target_calls.set(0);
        self.state.invocations.set(0);
    }

    pub fn telemetry(&self) -> NativeExecutionTelemetry {
        let bytecode_functions = self.state.bytecode_functions.get();
        let bytecode_instructions = self.state.bytecode_instructions.get();
        NativeExecutionTelemetry {
            bytecode_functions,
            bytecode_instructions,
            native_target_calls: self.state.native_target_calls.get(),
            invocations: self.state.invocations.get(),
        }
    }

    /// Executes a validated Hara program through the VM's synchronous fiber
    /// boundary. No tree-evaluator fallback is available from this path.
    pub fn execute(&self, program: Rc<Program>) -> Result<NativeExecutionReport, String> {
        self.execute_vm(ValidatedProgram::validate(program)?)
    }

    /// Executes a validated Hara program and drives settled VM promises to the
    /// same blocking boundary used by `Runtime::eval_native`.
    pub fn execute_blocking(&self, program: Rc<Program>) -> Result<NativeExecutionReport, String> {
        self.execute_vm(ValidatedProgram::validate(program)?)
    }

    pub(crate) fn execute_blocking_validated_with_multimethods(
        &self,
        program: ValidatedProgram,
        multimethods: crate::core::MultiMethodRegistry,
    ) -> Result<NativeExecutionReport, String> {
        let context = crate::core::DirectNativeContext::capture_with_multimethods(multimethods);
        context.with(|| self.execute_validated(program))
    }

    fn execute_validated(
        &self,
        validated: ValidatedProgram,
    ) -> Result<NativeExecutionReport, String> {
        self.execute_vm(validated)
    }

    fn execute_vm(&self, validated: ValidatedProgram) -> Result<NativeExecutionReport, String> {
        let program = validated.program();
        let bytecode_functions = program.functions.len();
        let bytecode_instructions = program
            .functions
            .iter()
            .map(|function| function.code.len())
            .sum();
        self.record_bytecode_program(&program);
        self.state
            .invocations
            .set(self.state.invocations.get().saturating_add(1));
        let before_targets = self.state.native_target_calls.get();
        let state = self.state.clone();
        let run = || {
            let mut fiber = VmFiber::start(program);
            fiber.drive_sync().map_err(|error| error.to_string())
        };
        let result = with_active_engine(state, || crate::core::with_direct_native_execution(run));
        let native_target_calls = self
            .state
            .native_target_calls
            .get()
            .saturating_sub(before_targets);
        let value = result?;
        Ok(NativeExecutionReport {
            value,
            bytecode_functions,
            bytecode_instructions,
            native_target_calls,
            invocations: 1,
        })
    }

    fn record_bytecode_program(&self, program: &Program) {
        self.state.bytecode_functions.set(
            self.state
                .bytecode_functions
                .get()
                .saturating_add(program.functions.len()),
        );
        let instructions = program
            .functions
            .iter()
            .map(|function| function.code.len())
            .sum::<usize>();
        self.state.bytecode_instructions.set(
            self.state
                .bytecode_instructions
                .get()
                .saturating_add(instructions),
        );
    }
}

fn with_active_engine<R>(state: Rc<NativeEngineState>, action: impl FnOnce() -> R) -> R {
    ACTIVE_NATIVE_ENGINE.with(|active| {
        let previous = active.borrow_mut().replace(state);
        let result = action();
        *active.borrow_mut() = previous;
        result
    })
}

pub(crate) fn capture_execution_scope() -> Option<NativeExecutionScope> {
    ACTIVE_NATIVE_ENGINE.with(|active| {
        active
            .borrow()
            .as_ref()
            .cloned()
            .map(|state| NativeExecutionScope { state })
    })
}

/// Adds a nested VM program to the active engine's bytecode telemetry. Nested
/// programs are produced by native `Runtime/eval`, declaration adapters, and
/// dynamic namespace seams; they are still validated Hara bytecode units.
pub(crate) fn record_bytecode_program(program: &Program) {
    ACTIVE_NATIVE_ENGINE.with(|active| {
        if let Some(state) = active.borrow().as_ref() {
            state.bytecode_functions.set(
                state
                    .bytecode_functions
                    .get()
                    .saturating_add(program.functions.len()),
            );
            let instructions = program
                .functions
                .iter()
                .map(|function| function.code.len())
                .sum::<usize>();
            state.bytecode_instructions.set(
                state
                    .bytecode_instructions
                    .get()
                    .saturating_add(instructions),
            );
        }
    });
}

impl NativeExecutionScope {
    pub(crate) fn with<R>(&self, action: impl FnOnce() -> R) -> R {
        with_active_engine(self.state.clone(), || {
            crate::core::with_direct_native_execution(action)
        })
    }
}

/// Re-enters both the native target scope and the captured runtime context for
/// a VM callback which may run after its creating VM entry has returned.
pub(crate) fn with_captured_context<R>(
    scope: Option<&NativeExecutionScope>,
    context: Option<&crate::core::DirectNativeContext>,
    action: impl FnOnce() -> R,
) -> R {
    if let Some(scope) = scope {
        scope.with(|| {
            if let Some(context) = context {
                context.with(action)
            } else {
                action()
            }
        })
    } else if let Some(context) = context {
        context.with(action)
    } else {
        action()
    }
}

/// Returns whether a symbol belongs to the closed native target inventory.
/// Ordinary Hara namespace Vars intentionally do not match this predicate.
pub(crate) fn is_native_target_symbol(name: &str) -> bool {
    crate::core::IntrinsicOp::from_symbol(name).is_some()
        || matches!(name, "disj" | "quot" | "rem" | "mod")
        || crate::core::canonical_intrinsic_callable_symbol(name).is_some()
}

/// Records one approved target call for the active native-substrate entry.
pub(crate) fn record_native_target(name: &str) {
    if !is_native_target_symbol(name) {
        return;
    }
    ACTIVE_NATIVE_ENGINE.with(|active| {
        if let Some(state) = active.borrow().as_ref() {
            state
                .native_target_calls
                .set(state.native_target_calls.get().saturating_add(1));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn program(source: &str) -> Rc<Program> {
        Rc::new(crate::vm::compile_source(source).expect("test program compiles"))
    }

    #[test]
    fn native_engine_executes_hara_in_the_vm_and_counts_targets() {
        let engine = NativeEngine::new();
        let report = engine
            .execute(program("(let [value 20] (+ value 22))"))
            .expect("native-substrate execution");
        assert_eq!(report.value, Value::Number(42));
        assert_eq!(report.bytecode_functions, 1);
        assert!(report.bytecode_instructions > 0);
        assert!(report.native_target_calls > 0);
        assert_eq!(report.invocations, 1);
        let telemetry = engine.telemetry();
        assert_eq!(telemetry.bytecode_functions, 1);
        assert_eq!(telemetry.invocations, 1);
        assert!(telemetry.native_target_calls > 0);
    }

    #[test]
    fn native_engine_reset_is_idempotent_and_does_not_discard_programs() {
        let engine = NativeEngine::new();
        let program = program("(+ 20 22)");
        engine.execute(program.clone()).expect("first execution");
        engine.reset();
        engine.reset();
        assert_eq!(engine.telemetry(), NativeExecutionTelemetry::default());
        assert_eq!(engine.execute(program).unwrap().value, Value::Number(42));
    }

    #[test]
    fn only_closed_native_target_names_are_classified() {
        assert!(is_native_target_symbol("+"));
        assert!(is_native_target_symbol("std.native.String/length"));
        assert!(is_native_target_symbol(
            "std.protocol.ilookup.ILookup/lookup"
        ));
        assert!(is_native_target_symbol("quot"));
        assert!(!is_native_target_symbol("std.foundation.core/map"));
        assert!(!is_native_target_symbol("example.application/start"));
    }
}
