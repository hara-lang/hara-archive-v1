use hara_wasm::extension::Value;
use hara_wasm::wasmtime_provider::WasmtimeExtensionProvider;
use hara_wasm::Runtime;
use std::hint::black_box;
use std::path::PathBuf;
use std::time::Instant;

const MANIFEST: &str = r#"{:namespace "bench.wasm"
 :version "1" :provider :wasm :module "hara.wasm" :abi :core.v1
 :exports {"hta_abi_version" {:args [] :returns :i32}} :capabilities []}"#;

fn main() -> Result<(), String> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let path = args.first().map_or_else(
        || {
            PathBuf::from(
                "core/rust/raw/target/wasm32-unknown-unknown/browser-release/hara-wasm-vm.wasm",
            )
        },
        PathBuf::from,
    );
    let iterations = args
        .get(1)
        .and_then(|value| value.parse().ok())
        .unwrap_or(10_000usize);
    let bytes = std::fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))?;

    let compile_started = Instant::now();
    let provider = WasmtimeExtensionProvider::compile(&bytes)?;
    let compile_ns = compile_started.elapsed().as_nanos();

    let mut runtime = Runtime::new();
    let install_started = Instant::now();
    runtime.install_wasm_extension(MANIFEST, "benchmark", provider)?;
    let first = runtime.invoke_wasm_extension("bench.wasm", "hta_abi_version", &[])?;
    let first_ns = install_started.elapsed().as_nanos();
    if !matches!(first, Value::Number(2)) {
        return Err(format!("unexpected ABI result: {}", first.display()));
    }

    let started = Instant::now();
    for _ in 0..iterations {
        black_box(runtime.invoke_wasm_extension("bench.wasm", "hta_abi_version", &[])?);
    }
    let warm_ns = started.elapsed().as_nanos() / iterations as u128;
    println!(
        "{{\"boundary\":\"hara-to-wasmtime-core-v1\",\"module_bytes\":{},\"compile_ns\":{},\"first_ns\":{},\"warm_ns\":{},\"iterations\":{}}}",
        bytes.len(), compile_ns, first_ns, warm_ns, iterations
    );
    Ok(())
}
