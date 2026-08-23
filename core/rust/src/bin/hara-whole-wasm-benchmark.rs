use std::rc::Rc;
use std::time::Instant;

use hara_wasm::core::Value;
use hara_wasm::vm::{compile_source, execute_program};
use hara_wasm::whole_wasm::{compile_artifact, NativeModule};
use serde_json::{json, Value as JsonValue};

const CORPUS: &str = include_str!("../../assets/whole-wasm-workloads.json");

fn elapsed_ns(started: Instant) -> u128 {
    started.elapsed().as_nanos()
}

fn measure<F>(mut call: F) -> Result<(u128, u128), String>
where
    F: FnMut() -> Result<i64, String>,
{
    for _ in 0..2 {
        call()?;
    }
    let first_started = Instant::now();
    call()?;
    let first_ns = elapsed_ns(first_started);
    let calls = 5;
    let steady_started = Instant::now();
    for _ in 0..calls {
        call()?;
    }
    let steady_ns = elapsed_ns(steady_started) / calls;
    Ok((first_ns, steady_ns))
}

fn number(value: Value) -> Result<i64, String> {
    match value {
        Value::Number(value) => Ok(value),
        other => Err(format!("expected an i64 workload result, got {other:?}")),
    }
}

fn main() -> Result<(), String> {
    let corpus: JsonValue = serde_json::from_str(CORPUS).map_err(|error| error.to_string())?;
    let workloads = corpus["workloads"]
        .as_array()
        .ok_or("whole-Wasm corpus is missing workloads")?;
    let mut measurements = Vec::with_capacity(workloads.len());

    for workload in workloads {
        let id = workload["id"].as_str().ok_or("workload id is missing")?;
        let source = workload["hara_source"]
            .as_str()
            .ok_or("workload source is missing")?;
        let expected = workload["expected"]
            .as_str()
            .ok_or("workload expected value is missing")?
            .parse::<i64>()
            .map_err(|error| error.to_string())?;
        let compile_started = Instant::now();
        let program = compile_source(source).map_err(|error| error.to_string())?;
        let artifact = compile_artifact(&program)?;
        let prepare_ns = elapsed_ns(compile_started);

        let hbc_program = Rc::new(program.clone());
        let hbc_call = || {
            let value = execute_program(hbc_program.clone()).map_err(|error| error.to_string())?;
            number(value)
        };
        if hbc_call()? != expected {
            return Err(format!("{id}: HBC checksum mismatch"));
        }
        let (hbc_first_ns, hbc_steady_ns) = measure(hbc_call)?;

        let mut native = NativeModule::load(&artifact)?;
        let mut native_call = || native.call_entry_i64();
        if native_call()? != expected {
            return Err(format!("{id}: whole-Wasm checksum mismatch"));
        }
        let native_first_started = Instant::now();
        native_call()?;
        let native_first_ns = elapsed_ns(native_first_started);
        let native_steady_started = Instant::now();
        for _ in 0..5 {
            native_call()?;
        }
        let native_steady_ns = elapsed_ns(native_steady_started) / 5;

        measurements.push(json!({
            "id": id,
            "expected": expected.to_string(),
            "prepare_ns": prepare_ns,
            "hbc_first_ns": hbc_first_ns,
            "hbc_steady_ns": hbc_steady_ns,
            "whole_wasm_first_ns": native_first_ns,
            "whole_wasm_steady_ns": native_steady_ns,
            "status": "ok"
        }));
    }

    let report = json!({
        "schema": "hara.whole-wasm.performance/0-alpha",
        "corpus": "whole-wasm-workloads/1",
        "measurements": measurements
    });
    let output = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "core/target/whole-wasm-native-performance.json".into());
    if let Some(parent) = std::path::Path::new(&output).parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    std::fs::write(
        &output,
        format!("{}\n", serde_json::to_string_pretty(&report).unwrap()),
    )
    .map_err(|error| error.to_string())?;
    println!("{output}");
    Ok(())
}
