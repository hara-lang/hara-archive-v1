//! Benchmark driver for the experimental bytecode VM (issue #195).
//!
//! Same wire protocol as `hara-runtime-benchmark` plus a leading MODE:
//!
//! ```text
//! hara-bytecode-benchmark MODE ID SOURCE_HEX EXPECTED WINDOWS CALLS [RUNTIME]
//! ```
//!
//! Modes:
//!
//! - `existing`        — `Runtime::eval_native` baseline (parse + fiber
//!                       evaluation per call).
//! - `compile-execute` — parse + compile + validate + execute + display
//!                       per call through the isolated VM API used by issues
//!                       #195 and #202.
//! - `execute-only`    — compile once; execute + display per call.
//! - `execute-feature-enabled` — the same ordinary execution API, intended
//!                       to run in a build that also compiles instrumentation.
//! - `execute-instrumented-noop` — compile once; execute through `NoProbe`.
//! - `execute-counted` — compile once; collect aggregate counters.
//! - `execute-sampled` — compile once; sample instructions while retaining
//!                       all control-flow and terminal boundaries.
//! - `execute-events`  — compile once; retain a fixed-capacity event ring.
//! - `execute-observed` — compile once; execute through full snapshots.
//! - `runtime-compile-execute` — compile and execute through a `Runtime`,
//!                       including namespace compatibility synchronization.
//! - `runtime-execute` — compile once against a `Runtime`; execute through
//!                       the namespace-integrated compatibility path.
//! - `runtime-registry-execute` — compile once against a `Runtime`; execute
//!                       against its namespace registry without copying the
//!                       registry into the tree-walker environment per call.
//! - `halc-execute`    — encode as HALC and lower to typed HBC0 once, then
//!                       execute it against the module namespace.
//! - `whole-wasm`      — compile HBC0 to an HNW0 whole-function Wasm module
//!                       once, then call the generated entry through Wasmtime.
//! - `whole-wasm-value` — the dynamic-value whole-Wasm boundary, retaining
//!                       arbitrary integer and boolean results instead of
//!                       requiring the initial scalar i64 ABI.
//!
//! Every call checks the result against EXPECTED (the correctness
//! checksum); a mismatch aborts the run. Output is one JSON line with
//! `first_ns` and the per-window `samples_ns`.

use hara_wasm::Runtime;
use std::time::Instant;

fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if !(6..=7).contains(&args.len()) {
        eprintln!("benchmark expects MODE ID SOURCE_HEX EXPECTED WINDOWS CALLS [RUNTIME]");
        std::process::exit(2);
    }
    let mode = &args[0];
    let id = &args[1];
    let source = decode_hex(&args[2]).unwrap_or_else(|error| fail(id, &error));
    let expected = &args[3];
    let windows: usize = args[4]
        .parse()
        .unwrap_or_else(|_| fail(id, "invalid windows"));
    let calls: usize = args[5]
        .parse()
        .unwrap_or_else(|_| fail(id, "invalid calls"));
    let default_runtime_name = match mode.as_str() {
        "existing" => "hara-rust-existing",
        "compile-execute" => "hara-rust-bytecode-compile-execute",
        "execute-only" => "hara-rust-bytecode-execute-only",
        "execute-feature-enabled" => "hara-rust-bytecode-feature-enabled",
        "execute-instrumented-noop" => "hara-rust-bytecode-instrumented-noop",
        "execute-counted" => "hara-rust-bytecode-counted",
        "execute-sampled" => "hara-rust-bytecode-sampled",
        "execute-events" => "hara-rust-bytecode-events",
        "execute-observed" => "hara-rust-bytecode-observed",
        "runtime-compile-execute" => "hara-rust-bytecode-runtime-compile-execute",
        "runtime-execute" => "hara-rust-bytecode-runtime-execute",
        "runtime-registry-execute" => "hara-rust-bytecode-runtime-registry-execute",
        "halc-execute" => "hara-rust-bytecode-halc-execute",
        "whole-wasm" => "hara-rust-whole-wasm",
        "whole-wasm-value" => "hara-rust-whole-wasm-value",
        other => fail(id, &format!("unknown mode: {other}")),
    };
    let runtime_name = args
        .get(6)
        .map(String::as_str)
        .unwrap_or(default_runtime_name);

    let mut runtime = Runtime::new();
    let prepare_started = Instant::now();
    // Execute-only and observability modes compile once, outside the samples.
    let program = match mode.as_str() {
        "execute-only"
        | "execute-feature-enabled"
        | "execute-instrumented-noop"
        | "execute-counted"
        | "execute-sampled"
        | "execute-events"
        | "execute-observed" => {
            Some(hara_wasm::compile_bytecode(&source).unwrap_or_else(|error| fail(id, &error)))
        }
        "runtime-execute" | "runtime-registry-execute" => Some(
            runtime
                .compile_bytecode(&source)
                .unwrap_or_else(|error| fail(id, &error)),
        ),
        "halc-execute" => Some(compile_halc(&mut runtime, id, &source)),
        _ => None,
    };
    let artifact_bytes = program.as_ref().and_then(|program| {
        hara_wasm::vm::encode_program(program)
            .ok()
            .map(|artifact| artifact.len())
    });
    #[cfg(feature = "whole-wasm")]
    let mut artifact_bytes = artifact_bytes;
    #[cfg(feature = "whole-wasm")]
    let mut native_entry = None;
    #[cfg(not(feature = "whole-wasm"))]
    let native_entry: Option<bool> = None;
    #[cfg(feature = "whole-wasm")]
    let mut whole_unsupported: Option<String> = None;
    #[cfg(not(feature = "whole-wasm"))]
    let whole_unsupported: Option<String> = None;
    #[cfg(feature = "whole-wasm")]
    let mut native = if matches!(mode.as_str(), "whole-wasm" | "whole-wasm-value") {
        let mut program = (*runtime
            .compile_bytecode(&source)
            .unwrap_or_else(|error| fail(id, &error)))
        .clone();
        if mode == "whole-wasm-value" {
            declare_dynamic_entry(&mut program);
        }
        let artifact = hara_wasm::whole_wasm::compile_artifact(&program)
            .unwrap_or_else(|error| fail(id, &error));
        artifact_bytes = Some(artifact.len());
        let decoded = hara_wasm::whole_wasm::decode_artifact(&artifact)
            .unwrap_or_else(|error| fail(id, &error));
        native_entry = decoded
            .capabilities
            .get(usize::from(program.entry))
            .copied();
        if mode == "whole-wasm-value" && native_entry != Some(true) {
            whole_unsupported = Some("whole-Wasm entry is not native for dynamic values".into());
            None
        } else {
            Some(
                hara_wasm::whole_wasm::NativeModule::load(&artifact)
                    .unwrap_or_else(|error| fail(id, &error)),
            )
        }
    } else {
        None
    };
    let prepare_ns = match mode.as_str() {
        "execute-only"
        | "execute-feature-enabled"
        | "execute-instrumented-noop"
        | "execute-counted"
        | "execute-sampled"
        | "execute-events"
        | "execute-observed"
        | "runtime-execute"
        | "runtime-registry-execute"
        | "halc-execute"
        | "whole-wasm"
        | "whole-wasm-value" => Some(prepare_started.elapsed().as_nanos()),
        _ => None,
    };
    if let Some(reason) = whole_unsupported {
        println!(
            "{{\"runtime\":\"{}\",\"workload\":\"{}\",\"prepare_ns\":{},\"artifact_bytes\":{},\"native_entry\":{},\"status\":\"unsupported\",\"reason\":\"{}\"}}",
            json(runtime_name),
            json(id),
            prepare_ns.map_or_else(|| "null".to_string(), |value| value.to_string()),
            artifact_bytes.map_or_else(|| "null".to_string(), |value| value.to_string()),
            native_entry.map_or_else(|| "null".to_string(), |value| value.to_string()),
            json(&reason),
        );
        return;
    }
    let mut call = || {
        let value = match mode.as_str() {
            "existing" => runtime.eval_native(&source),
            "compile-execute" => hara_wasm::eval_bytecode_native(&source),
            "execute-only" | "execute-feature-enabled" => {
                hara_wasm::execute_bytecode(program.as_ref().expect("program"))
            }
            "execute-instrumented-noop"
            | "execute-counted"
            | "execute-sampled"
            | "execute-events" => execute_instrumented(program.as_ref().expect("program"), mode),
            "execute-observed" => execute_observed(program.as_ref().expect("program")),
            "runtime-compile-execute" => runtime.eval_bytecode_native(&source),
            "runtime-execute" => {
                runtime.execute_compiled_bytecode(program.as_ref().expect("program").clone())
            }
            "runtime-registry-execute" => runtime
                .execute_compiled_bytecode_registry_value(
                    program.as_ref().expect("program").clone(),
                )
                .map(|value| value.display()),
            "halc-execute" => {
                runtime.execute_compiled_bytecode(program.as_ref().expect("program").clone())
            }
            #[cfg(feature = "whole-wasm")]
            "whole-wasm" => native
                .as_mut()
                .expect("native module")
                .call_entry_i64()
                .map(|value| value.to_string()),
            #[cfg(feature = "whole-wasm")]
            "whole-wasm-value" => native
                .as_mut()
                .expect("native module")
                .call_entry_value()
                .map(|value| value.display()),
            #[cfg(not(feature = "whole-wasm"))]
            "whole-wasm" | "whole-wasm-value" => {
                fail(id, "whole-wasm mode requires the whole-wasm feature")
            }
            _ => unreachable!(),
        };
        value.unwrap_or_else(|error| fail(id, &error))
    };

    let started = Instant::now();
    let first = call();
    let first_ns = started.elapsed().as_nanos();
    assert_value(id, expected, &first);
    let mut samples = Vec::with_capacity(windows);
    for _ in 0..windows {
        let started = Instant::now();
        for _ in 0..calls {
            assert_value(id, expected, &call());
        }
        samples.push(started.elapsed().as_nanos() / calls as u128);
    }
    let samples = samples
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",");
    #[cfg(feature = "tracing-jit")]
    let telemetry = program.as_ref().map_or_else(String::new, |program| {
        let telemetry = hara_wasm::bytecode_jit_telemetry(program);
        format!(
            ",\"jit\":{{\"backedges\":{},\"compile_attempts\":{},\"compiled\":{},\"rejected\":{},\"entries\":{},\"completed_iterations\":{},\"side_exits\":{},\"recording_starts\":{},\"recording_completed\":{},\"recording_aborts\":{},\"trace_paths\":{},\"branch_exits\":{},\"type_exits\":{},\"error_exits\":{},\"disabled_loops\":{}}}",
            telemetry.backedges,
            telemetry.compile_attempts,
            telemetry.compiled,
            telemetry.rejected,
            telemetry.entries,
            telemetry.completed_iterations,
            telemetry.side_exits,
            telemetry.recording_starts,
            telemetry.recording_completed,
            telemetry.recording_aborts,
            telemetry.trace_paths,
            telemetry.branch_exits,
            telemetry.type_exits,
            telemetry.error_exits,
            telemetry.disabled_loops,
        )
    });
    #[cfg(not(feature = "tracing-jit"))]
    let telemetry = String::new();
    println!(
        "{{\"runtime\":\"{}\",\"workload\":\"{}\",\"prepare_ns\":{},\"first_ns\":{},\"samples_ns\":[{}],\"artifact_bytes\":{},\"native_entry\":{}{} }}",
        json(runtime_name),
        json(id),
        prepare_ns.map_or_else(|| "null".to_string(), |value| value.to_string()),
        first_ns,
        samples,
        artifact_bytes.map_or_else(|| "null".to_string(), |value| value.to_string()),
        native_entry.map_or_else(|| "null".to_string(), |value| value.to_string()),
        telemetry,
    );
}

#[cfg(feature = "whole-wasm")]
fn declare_dynamic_entry(program: &mut hara_wasm::vm::Program) {
    use hara_wasm::kernel::{FunctionSchema, SchemaType};

    const NAMESPACE: &str = "hara.integer-representation-benchmark";
    let name = format!("{NAMESPACE}/entry");
    let index = usize::from(program.entry);
    program.namespace = Some(NAMESPACE.into());
    program.functions[index].name = Some(name.clone());
    program.function_types.insert(
        name,
        SchemaType::Function(vec![FunctionSchema {
            fixed: Vec::new(),
            rest: None,
            output: Box::new(SchemaType::Primitive("any".into())),
        }]),
    );
}

#[cfg(feature = "bytecode-instrumentation")]
fn execute_instrumented(
    program: &std::rc::Rc<hara_wasm::vm::Program>,
    mode: &str,
) -> Result<String, String> {
    use hara_wasm::vm::{CounterProbe, EventRing, Machine, NoProbe, SampledProbe};
    use std::hint::black_box;

    let mut machine = Machine::entry(program.clone());
    let outcome = match mode {
        "execute-instrumented-noop" => {
            let mut probe = NoProbe;
            machine.run_instrumented(&mut probe)
        }
        "execute-counted" => {
            let mut probe = CounterProbe::default();
            let outcome = machine.run_instrumented(&mut probe);
            black_box((
                probe.metrics().instructions,
                probe.metrics().max_stack_depth,
                probe.metrics().max_call_depth,
            ));
            outcome
        }
        "execute-sampled" => {
            let mut probe = SampledProbe::new(EventRing::with_capacity(256), 64);
            let outcome = machine.run_instrumented(&mut probe);
            black_box((probe.inner().len(), probe.inner().dropped()));
            outcome
        }
        "execute-events" => {
            let mut probe = EventRing::with_capacity(256);
            let outcome = machine.run_instrumented(&mut probe);
            black_box((probe.len(), probe.dropped()));
            outcome
        }
        _ => return Err(format!("unknown instrumented mode: {mode}")),
    };
    display_outcome(outcome)
}

#[cfg(not(feature = "bytecode-instrumentation"))]
fn execute_instrumented(
    _program: &std::rc::Rc<hara_wasm::vm::Program>,
    mode: &str,
) -> Result<String, String> {
    Err(format!(
        "{mode} requires the bytecode-instrumentation feature"
    ))
}

#[cfg(feature = "bytecode-observation")]
fn execute_observed(program: &std::rc::Rc<hara_wasm::vm::Program>) -> Result<String, String> {
    use hara_wasm::vm::{Machine, ObservedStepOutcome};

    let mut machine = Machine::entry(program.clone());
    loop {
        match machine.step_observed().outcome {
            ObservedStepOutcome::Continue => {}
            ObservedStepOutcome::Returned(value) => return Ok(value.display()),
            ObservedStepOutcome::Failed(error) => return Err(error.to_string()),
            ObservedStepOutcome::Suspended(_) => return Err("observed benchmark suspended".into()),
            ObservedStepOutcome::Yielded(_) => return Err("observed benchmark yielded".into()),
        }
    }
}

#[cfg(not(feature = "bytecode-observation"))]
fn execute_observed(_program: &std::rc::Rc<hara_wasm::vm::Program>) -> Result<String, String> {
    Err("execute-observed requires the bytecode-observation feature".into())
}

#[cfg(feature = "bytecode-instrumentation")]
fn display_outcome(outcome: hara_wasm::vm::VmOutcome) -> Result<String, String> {
    match outcome {
        hara_wasm::vm::VmOutcome::Returned(value) => Ok(value.display()),
        hara_wasm::vm::VmOutcome::Failed(error) => Err(error.to_string()),
        hara_wasm::vm::VmOutcome::Suspended(_) => Err("instrumented benchmark suspended".into()),
        hara_wasm::vm::VmOutcome::Yielded(_) => Err("instrumented benchmark yielded".into()),
    }
}

#[cfg(feature = "halc-encoder")]
fn compile_halc(
    runtime: &mut Runtime,
    id: &str,
    source: &str,
) -> std::rc::Rc<hara_wasm::vm::Program> {
    let forms = hara_wasm::kernel::parse_forms(source).unwrap_or_else(|error| fail(id, &error));
    let artifact = hara_wasm::kernel::halc::encode_halc_module(
        "benchmark.typed",
        "benchmark/typed.hal",
        source,
        forms,
    )
    .unwrap_or_else(|error| fail(id, &error));
    let bytecode = runtime
        .compile_halc_bytecode_artifact(&artifact)
        .unwrap_or_else(|error| fail(id, &error));
    hara_wasm::vm::decode_program(&bytecode)
        .map(std::rc::Rc::new)
        .unwrap_or_else(|error| fail(id, &error))
}

#[cfg(not(feature = "halc-encoder"))]
fn compile_halc(
    _runtime: &mut Runtime,
    id: &str,
    _source: &str,
) -> std::rc::Rc<hara_wasm::vm::Program> {
    fail(id, "halc-execute requires the halc-encoder feature")
}

fn decode_hex(value: &str) -> Result<String, String> {
    if value.len() % 2 != 0 {
        return Err("invalid source hex".into());
    }
    let bytes = (0..value.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&value[index..index + 2], 16))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "invalid source hex")?;
    String::from_utf8(bytes).map_err(|_| "source is not UTF-8".into())
}

fn assert_value(id: &str, expected: &str, actual: &str) {
    if expected != actual {
        fail(id, &format!("expected {expected}, got {actual}"));
    }
}

fn fail(id: &str, message: &str) -> ! {
    eprintln!("{id}: {message}");
    std::process::exit(1);
}

fn json(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}
