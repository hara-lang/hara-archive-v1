//! Experimental staged bytecode VM for the Rust runtime (issue #195).
//!
//! Milestone 4 compiles literals, lexical locals, arithmetic,
//! comparisons, `if`, `do`, `let`, `loop`/`recur`, `fn` closures
//! (including variadic), exceptions, and the registry-direct global
//! forms (`def`, `defn`/`defn-` single- and multi-arity, `var`, `set!`,
//! `declare`, `defstruct`, `field`, `instance?`) into a typed
//! instruction program and executes it on a stack machine (issue #223).
//! See `notes/rust-bytecode-vm.md` for the design.
//!
//! The main `hara-wasm` crate enables `bytecode-vm` in its default feature
//! set. VM entry points remain feature-gated for compiler-free and minimal
//! builds, while live snapshots and one-boundary stepping are separately
//! opt-in through `bytecode-observation`. The VM never falls back to the
//! tree-walking evaluator: unsupported forms are typed compile errors.

#[path = "vm/artifact.rs"]
pub mod artifact;
#[path = "vm/bundle.rs"]
pub mod bundle;
#[path = "vm/compiler.rs"]
pub mod compiler;
#[cfg(feature = "code-vm-conformance")]
#[path = "vm/conformance.rs"]
pub mod conformance;
#[path = "vm/disassemble.rs"]
pub mod disassemble;
#[path = "vm/error.rs"]
pub mod error;
#[path = "vm/fiber.rs"]
pub mod fiber;
#[path = "vm/frame.rs"]
pub mod frame;
#[path = "vm/machine.rs"]
pub mod machine;
#[path = "vm/opcode.rs"]
pub mod opcode;
#[path = "vm/prepared.rs"]
pub mod prepared;
#[path = "vm/program.rs"]
pub mod program;
#[cfg(feature = "bytecode-observation")]
#[path = "vm/session.rs"]
pub mod session;
#[path = "vm/slot.rs"]
mod slot;
#[path = "vm/source_map.rs"]
pub mod source_map;
#[path = "vm/validate.rs"]
pub mod validate;

#[cfg(test)]
#[path = "vm/conformance_tests.rs"]
mod conformance_tests;
#[cfg(test)]
#[path = "vm/differential_tests.rs"]
mod differential_tests;
#[cfg(test)]
#[path = "vm/execution_tests.rs"]
mod execution_tests;
#[cfg(all(test, feature = "bytecode-vm"))]
mod numeric_predicate_tests;
#[cfg(test)]
#[path = "vm/tests.rs"]
mod tests;

/// Normalizes an error message to a coarse category for comparison. The
/// fiber and the synchronous fallback phrase some shape errors
/// differently ("let expects bindings" vs "let expects a binding list or
/// vector"); each bucket covers every phrasing of one failure class.
/// Shared by the differential tests and the corpus-driven conformance
/// tests; the bucket names are pinned by
/// `specs/01-lang/010-bytecode/draft/conformance/bytecode-vm.edn`.
#[cfg(test)]
pub(crate) fn error_category(message: &str) -> &'static str {
    let buckets: &[(&[&str], &str)] = &[
        (&["division by zero"], "division by zero"),
        (&["integer overflow"], "integer overflow"),
        (&["expects numbers"], "expects numbers"),
        (
            &["expects at least", "expects arguments"],
            "primitive arity",
        ),
        (&["expects 2 or 3 arguments"], "if arity"),
        (
            &["expects bindings and a body", "expects bindings and body"],
            "binding body shape",
        ),
        (
            &["expects a binding list or vector", "expects bindings"],
            "binding bindings shape",
        ),
        (&["require name/value pairs"], "binding pairs"),
        (&["function expects"], "function arity"),
        (&["value is not callable"], "not callable"),
        (
            &[
                "function parameters must be a vector",
                "defn arity must contain parameters and a body",
            ],
            "fn params shape",
        ),
        (&["conj expects a collection"], "conj receiver"),
        (&["throw expects one value"], "throw arity"),
        (&["thrown: "], "thrown"),
        // "unbound var" is checked first: its message contains "unbound
        // var", not "unbound symbol", so order is safe either way.
        (&["unbound var"], "unbound var"),
        (&["unbound symbol"], "unbound symbol"),
        (
            &[
                "ns+ does not accept",
                "ns accepts only one",
                "ns clause",
                ":config",
                "Namespace alias",
            ],
            "namespace config",
        ),
        (&["recur"], "recur"),
        (
            &[
                "Invalid number",
                "Legacy numeric suffixes",
                "EOF while reading",
            ],
            "reader",
        ),
    ];
    for (markers, bucket) in buckets {
        if markers.iter().any(|marker| message.contains(marker)) {
            return bucket;
        }
    }
    panic!("unclassified error message: {message}")
}

pub use artifact::{decode_program, encode_program};
pub use bundle::{
    compile_bytecode_bundle, compile_embedded_foundation_bootstrap_bundle,
    compile_embedded_standard_library_bundle, compile_package_bytecode_bundle,
    embedded_foundation_bootstrap_sources, encode_bytecode_bundle, eval_bytecode_bundle,
    eval_eager_bytecode_bundle_with_registries, BytecodeBundleModule, ModuleSource,
};
pub use compiler::{
    compile_form_with_config_allow_unbound_globals, compile_halc_module, compile_source,
    compile_source_with,
    compile_source_with_allow_unbound_globals, compile_source_with_config,
    compile_source_with_config_allow_unbound_globals, source_namespace_config,
    source_uses_dynamic_evaluation,
};
pub use disassemble::disassemble;
pub use error::{CompileError, CompileErrorKind, ValidationError, VmError};
pub use fiber::{VmFiber, VmFiberState};
#[cfg(feature = "bytecode-instrumentation")]
pub use machine::instrumentation::{
    BytecodeMetrics, CounterProbe, EventRing, InstructionEvent, NoProbe, Opcode, OpcodeCount,
    SampledProbe, TerminalEvent, TerminalKind, TransitionEvent, TransitionKind, VmEvent, VmProbe,
    BYTECODE_EVENTS_SCHEMA, BYTECODE_METRICS_SCHEMA,
};
#[cfg(feature = "bytecode-observation")]
pub use machine::observation::{
    CallFrameSnapshot, HandlerSnapshot, InstructionOperand, InstructionSnapshot,
    MachineObservationStatus, MachineSnapshot, ObservationEventKind, ObservationEventStatus,
    ObservationLimits, ObservedStep, ObservedStepOutcome, ProgramSnapshot, SourcePositionSnapshot,
    ValueSnapshot, BYTECODE_TRACE_SCHEMA,
};
pub use machine::{execute_program, execute_program_with_globals, Machine, VmOutcome};
pub use opcode::Instruction;
pub use prepared::{prepare_call, PreparedCall};
pub use program::{FunctionId, FunctionPrototype, Program};
#[cfg(feature = "bytecode-observation")]
pub use session::{
    BytecodeObservationSession, BytecodeSessionError, BytecodeSessionStatus, SessionRetentionLimits,
};
pub use validate::validate;

/// Compiles, validates, and executes a closed source string in one step.
/// Errors from either stage flatten to their display form (which carries
/// source positions). No fallback to the tree-walking evaluator.
pub fn eval_source(source: &str) -> Result<crate::core::Value, String> {
    let program = compile_source(source).map_err(|error| error.to_string())?;
    execute_program(std::rc::Rc::new(program)).map_err(|error| error.to_string())
}
