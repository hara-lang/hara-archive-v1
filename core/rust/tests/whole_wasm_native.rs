#![cfg(feature = "whole-wasm")]

use std::rc::Rc;

use hara_wasm::core::Value;
use hara_wasm::vm::{compile_source, execute_program};
use hara_wasm::whole_wasm::{
    compile_artifact, compile_artifact_from_hbc, decode_artifact, NativeModule, HNW_ABI_VERSION,
};
use sha2::{Digest, Sha256};

fn compile_module(source: &str) -> (hara_wasm::vm::Program, NativeModule, Vec<u8>) {
    let program = compile_source(source).expect("source must compile to HBC0");
    let artifact = compile_artifact(&program).expect("source must compile to HNW0");
    let module = NativeModule::load(&artifact).expect("HNW0 must instantiate in Wasmtime");
    (program, module, artifact)
}

#[test]
fn whole_wasm_artifact_is_deterministic_and_preserves_hbc_parity() {
    let source = "(loop [i 0 acc 0]
                    (if (< i 5000)
                      (recur (+ i 1) (+ acc i))
                      acc))";
    let program = compile_source(source).expect("source must compile to HBC0");
    let first = compile_artifact(&program).expect("source must compile to HNW0");
    let second = compile_artifact(&program).expect("the same program must recompile");
    assert_eq!(first, second, "HNW0 compilation must be byte deterministic");

    let decoded = decode_artifact(&first).expect("HNW0 must decode");
    assert_eq!(decoded.abi_version, HNW_ABI_VERSION);
    assert_eq!(decoded.program.entry, program.entry);
    assert_eq!(decoded.program.functions.len(), program.functions.len());
    assert!(decoded.wasm.starts_with(b"\0asm"));
    assert_eq!(
        decoded
            .targets
            .iter()
            .map(|target| (target.id, target.symbol.as_str(), target.arity))
            .collect::<Vec<_>>(),
        vec![
            (0, "hara.whole-wasm/map", None),
            (1, "hara.whole-wasm/vector", None),
            (2, "std.native.Base/number?", Some(1)),
            (3, "std.protocol.iassoc.IAssoc/assoc", Some(3)),
            (4, "std.protocol.icount.ICount/count", Some(1)),
            (5, "std.protocol.ilookup.ILookup/lookup", Some(2)),
            (6, "std.protocol.inth.INth/nth", Some(2)),
        ]
    );

    let expected = execute_program(Rc::new(program)).expect("HBC0 must execute");
    let Value::Number(expected) = expected else {
        panic!("the scalar parity fixture must return an i64");
    };
    let mut native = NativeModule::load(&first).expect("decoded HNW0 must instantiate");
    assert_eq!(native.call_entry_i64(), Ok(expected));
}

#[test]
fn whole_wasm_operation_registry_keeps_the_hnw0_contract_digest() {
    let program = compile_source("(+ 1 2)").expect("source must compile to HBC0");
    let artifact = compile_artifact(&program).expect("source must compile to HNW0");
    assert_eq!(
        decode_artifact(&artifact)
            .expect("HNW0 must decode")
            .operation_registry_digest,
        [
            0xd8, 0xb2, 0xcd, 0x60, 0x97, 0xd1, 0x76, 0x00, 0xd5, 0xa5, 0x34, 0x18, 0x6d, 0x27,
            0xea, 0x27, 0x44, 0xf4, 0xc8, 0x05, 0x7b, 0x77, 0x9b, 0x2c, 0x6d, 0x0b, 0x7f, 0x97,
            0x27, 0x62, 0x3e, 0x2a,
        ]
    );
}

#[test]
fn hnw0_rejects_unknown_abi_versions() {
    let program = compile_source("(+ 19 23)").expect("source must compile to HBC0");
    let artifact = compile_artifact(&program).expect("source must compile to HNW0");
    let payload_end = 8 + u32::from_be_bytes(artifact[4..8].try_into().unwrap()) as usize;
    let mut corrupt = artifact;
    corrupt[8..10].copy_from_slice(&1u16.to_be_bytes());
    let digest = Sha256::digest(&corrupt[8..payload_end]);
    corrupt[payload_end..].copy_from_slice(&digest);

    assert_eq!(
        decode_artifact(&corrupt).unwrap_err(),
        "unsupported HNW ABI version 1"
    );
}

#[test]
fn canonical_hbc_artifact_is_the_hnw0_compiler_input() {
    let source = "(+ 19 23)";
    let program = compile_source(source).expect("source must compile to HBC0");
    let hbc = hara_wasm::vm::encode_program(&program).expect("HBC0 must encode");
    let artifact = compile_artifact_from_hbc(&hbc).expect("HBC0 must lower to HNW0");
    let decoded = decode_artifact(&artifact).expect("HNW0 must decode");
    let retained =
        hara_wasm::vm::encode_program(&decoded.program).expect("retained HBC0 must encode");
    assert_eq!(retained, hbc, "HNW0 must retain the canonical HBC0 input");
}

#[test]
fn whole_wasm_uses_only_the_generic_target_bridge_imports() {
    let program = compile_source("(+ 19 23)").expect("source must compile to HBC0");
    let artifact = compile_artifact(&program).expect("source must compile to HNW0");
    let decoded = decode_artifact(&artifact).expect("HNW0 must decode");
    let engine = wasmtime::Engine::default();
    let module = wasmtime::Module::new(&engine, &decoded.wasm).expect("Wasm must validate");
    let imports = module
        .imports()
        .map(|import| format!("{}::{}", import.module(), import.name()))
        .collect::<Vec<_>>();
    assert_eq!(
        imports,
        vec![
            "hara::constant_handle",
            "hara::box_i64",
            "hara::unbox_i64",
            "hara::value_construct",
            "hara::target_call",
        ]
    );
}

#[test]
fn whole_wasm_prepared_calls_are_repeatable_after_call_reset() {
    let source = "(let [a (std.native.Arr/new 1 2 3)]
                    (std.native.Arr/set a 1 40)
                    (+ (std.native.Arr/get a 0)
                       (+ (std.native.Arr/get a 1) 1)))";
    let (_, mut native, _) = compile_module(source);

    for _ in 0..8 {
        assert_eq!(native.call_entry_i64(), Ok(42));
    }
}

#[test]
fn whole_wasm_falls_back_to_hbc_for_values_outside_i64() {
    let source = "(+ 9223372036854775807 1)";
    let program = compile_source(source).expect("source must compile to HBC0");
    let expected = execute_program(Rc::new(program.clone())).expect("HBC0 must execute");
    assert_eq!(
        expected,
        Value::BigInteger("9223372036854775808".parse().unwrap())
    );

    let artifact = compile_artifact(&program).expect("source must compile to HNW0");
    let mut native = NativeModule::load(&artifact).expect("HNW0 must instantiate");
    let decoded = decode_artifact(&artifact).expect("HNW0 must decode");
    assert!(!decoded.capabilities[usize::from(decoded.program.entry)]);
    assert_eq!(native.call_entry_value(), Ok(expected));
}

#[test]
fn runtime_whole_wasm_product_has_stable_alpha_metadata() {
    let runtime = hara_wasm::Runtime::core();
    let source = "(+ 19 23)";
    let first = runtime
        .compile_whole_wasm_artifact_js(source)
        .expect("Runtime must produce an HNW0 product");
    let second = runtime
        .compile_whole_wasm_artifact_js(source)
        .expect("the cached HNW0 product must remain loadable");
    assert_eq!(first, second);

    let manifest = runtime
        .compile_whole_wasm_manifest_js(source)
        .expect("Runtime must publish the HNW0 manifest");
    let manifest: serde_json::Value = serde_json::from_str(&manifest).unwrap();
    assert_eq!(manifest["schema"], "hara.compiled-product.manifest/0-alpha");
    assert_eq!(manifest["product"], "whole-wasm");
    assert_eq!(manifest["format"], "HNW0");
    assert_eq!(manifest["abi-version"], "hnw0/0");
    assert_eq!(manifest["artifact-bytes"], first.len());
    assert_eq!(manifest["entrypoint"], "hara_entry");
    assert_eq!(manifest["error-global"], "hara_error");
    assert_eq!(manifest["heap-global"], "hara_heap");
    assert_eq!(manifest["import-module"], "hara");
}

#[test]
fn unsupported_function_uses_the_validated_hbc_fallback() {
    let source = "(throw \"native fallback\")";
    let program = compile_source(source).expect("source must compile to HBC0");
    let expected = execute_program(Rc::new(program.clone()))
        .expect_err("the HBC0 fixture must throw")
        .to_string();
    let artifact = compile_artifact(&program).expect("HNW0 must retain unsupported HBC0 code");
    let decoded = decode_artifact(&artifact).expect("fallback HNW0 must decode");
    assert!(!decoded.capabilities[usize::from(decoded.program.entry)]);

    let mut native = NativeModule::load(&artifact).expect("fallback HNW0 must instantiate");
    assert_eq!(native.call_entry_value(), Err(expected));
}

#[test]
fn native_and_fallback_functions_share_one_hnw0_module() {
    let source = "(+ 41 1)";
    let program = compile_source(source).expect("mixed source must compile to HBC0");
    let unsupported_program =
        compile_source("(throw nil)").expect("the fallback function must compile to HBC0");
    let mut program = program;
    let mut unsupported_function =
        unsupported_program.functions[unsupported_program.entry as usize].clone();
    unsupported_function.name = Some("unsupported".into());
    program.functions.push(unsupported_function);
    let artifact = compile_artifact(&program).expect("mixed source must compile to HNW0");
    let decoded = decode_artifact(&artifact).expect("mixed HNW0 must decode");
    let unsupported = decoded
        .program
        .functions
        .iter()
        .position(|function| function.name.as_deref() == Some("unsupported"))
        .expect("unsupported prototype must be retained") as u16;
    let entry = decoded.program.entry;
    assert!(decoded.capabilities[usize::from(entry)]);
    assert!(!decoded.capabilities[usize::from(unsupported)]);

    let expected = execute_program(Rc::new(unsupported_program))
        .expect_err("the fallback function must throw")
        .to_string();
    assert!(
        expected.starts_with("throw expects an Exception value created by ex"),
        "invalid throw values must retain the canonical Exception-only diagnostic: {expected}"
    );
    let mut native = NativeModule::load(&artifact).expect("mixed HNW0 must instantiate");
    assert_eq!(native.call_entry_i64(), Ok(42));
    assert_eq!(native.call_value(unsupported, &[]), Err(expected));
}

#[test]
fn hnw0_rejects_noncanonical_capability_metadata() {
    let program = compile_source("(+ 19 23)").expect("source must compile to HBC0");
    let artifact = compile_artifact(&program).expect("source must compile to HNW0");
    let payload_end = 8 + u32::from_be_bytes(artifact[4..8].try_into().unwrap()) as usize;
    let function_count = usize::from(u16::from_be_bytes(artifact[10..12].try_into().unwrap()));
    let capability = 12 + function_count * 4;
    let mut corrupt = artifact;
    corrupt[capability] = 2;
    let digest = Sha256::digest(&corrupt[8..payload_end]);
    corrupt[payload_end..].copy_from_slice(&digest);

    assert_eq!(
        decode_artifact(&corrupt).unwrap_err(),
        "native artifact capability table is not canonical"
    );
}

#[test]
fn five_workload_corpus_has_exact_hbc_and_whole_wasm_results() {
    let corpus: serde_json::Value =
        serde_json::from_str(include_str!("../assets/whole-wasm-workloads.json"))
            .expect("the whole-Wasm workload corpus must be valid JSON");
    assert_eq!(corpus["schema_version"], 1);
    let workloads = corpus["workloads"]
        .as_array()
        .expect("the workload corpus must contain an array");
    assert_eq!(workloads.len(), 5);

    for workload in workloads {
        let id = workload["id"].as_str().expect("workload id");
        let source = workload["hara_source"].as_str().expect("Hara source");
        let expected = workload["expected"]
            .as_str()
            .expect("expected checksum")
            .parse::<i64>()
            .expect("expected checksum must be an i64");
        let program = compile_source(source)
            .unwrap_or_else(|error| panic!("{id} must compile to HBC0: {error}"));
        assert_eq!(
            execute_program(Rc::new(program.clone())).expect("HBC0 workload must execute"),
            Value::Number(expected),
            "HBC0 result mismatch for {id}"
        );
        let artifact = compile_artifact(&program)
            .unwrap_or_else(|error| panic!("{id} must compile to HNW0: {error}"));
        let decoded = decode_artifact(&artifact).expect("HNW0 workload must decode");
        assert!(
            decoded.capabilities[usize::from(decoded.program.entry)],
            "{id} must use the native entry path"
        );
        let mut native = NativeModule::load(&artifact).expect("HNW0 workload must instantiate");
        assert_eq!(
            native.call_entry_i64(),
            Ok(expected),
            "whole-Wasm result mismatch for {id}"
        );
    }
}
