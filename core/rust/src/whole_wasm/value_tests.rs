use super::{compile_artifact, NativeModule};
use crate::core::Value;
use crate::kernel::{FunctionSchema, SchemaType};
use crate::vm::{compile_source, eval_source, FunctionId, Program};

const TEST_NAMESPACE: &str = "hara.whole-wasm.value-test";

fn dynamic_schema(arity: usize) -> SchemaType {
    let any = SchemaType::Primitive("any".into());
    SchemaType::Function(vec![FunctionSchema {
        fixed: vec![any.clone(); arity],
        rest: None,
        output: Box::new(any),
    }])
}

fn declare_dynamic_abi(program: &mut Program) {
    program.namespace = Some(TEST_NAMESPACE.into());
    for function in &program.functions {
        if let Some(name) = function.name.as_deref() {
            let local = name.rsplit('/').next().unwrap_or(name);
            program.function_types.insert(
                format!("{TEST_NAMESPACE}/{local}"),
                dynamic_schema(usize::from(function.arity)),
            );
        }
    }
}

fn function(module: &NativeModule, name: &str) -> FunctionId {
    module
        .artifact()
        .program
        .functions
        .iter()
        .position(|function| {
            function.name.as_deref().is_some_and(|candidate| {
                candidate == name || candidate.rsplit('/').next() == Some(name)
            })
        })
        .expect("named function") as FunctionId
}

fn module(source: &str) -> NativeModule {
    let mut program = compile_source(source).expect("source must compile");
    declare_dynamic_abi(&mut program);
    let artifact = compile_artifact(&program).expect("program must lower to whole-Wasm");
    NativeModule::load(&artifact).expect("whole-Wasm module must load")
}

fn scalar_module(source: &str, name: &str) -> NativeModule {
    let mut program = compile_source(source).expect("source must compile");
    program.namespace = Some(TEST_NAMESPACE.into());
    let int = SchemaType::Primitive("int".into());
    program.function_types.insert(
        format!("{TEST_NAMESPACE}/{name}"),
        SchemaType::Function(vec![FunctionSchema {
            fixed: vec![int.clone()],
            rest: None,
            output: Box::new(int),
        }]),
    );
    let artifact = compile_artifact(&program).expect("program must lower to whole-Wasm");
    NativeModule::load(&artifact).expect("whole-Wasm module must load")
}

#[test]
fn dynamic_values_round_trip_through_a_compiled_hara_function() {
    let mut native = module("(defn echo [value] value)\n0");
    let input = eval_source("{:nested [1 2 3] :label \"historia\"}").unwrap();
    let function = function(&native, "echo");
    let output = native
        .call_value(function, &[input.clone()])
        .expect("dynamic Hara value call");
    assert_eq!(output, input);
}

#[test]
fn arbitrary_integers_round_trip_through_dynamic_whole_wasm_calls() {
    let mut native = module("(defn echo [value] value)\n0");
    let input = eval_source("9223372036854775808").unwrap();
    let function = function(&native, "echo");
    let output = native
        .call_value(function, &[input.clone()])
        .expect("dynamic Hara integer call");
    assert_eq!(output, input);
}

#[test]
fn dynamic_values_are_transformed_inside_whole_wasm() {
    let mut native = module("(defn annotate [value] (assoc value :answer 42))\n0");
    let input = eval_source("{:nested [1 2 3]}").unwrap();
    let expected = eval_source("{:nested [1 2 3] :answer 42}").unwrap();
    let function = function(&native, "annotate");
    let output = native
        .call_value(function, &[input])
        .expect("dynamic Hara collection call");
    assert_eq!(output, expected);
}

#[test]
fn dynamic_values_cross_static_hara_calls_without_reboxing() {
    let mut native = module(
        "(defn annotate [value] (assoc value :answer 42))\n\
         (defn pipeline [value] (annotate value))\n0",
    );
    let input = eval_source("{:nested [1 2 3]}").unwrap();
    let expected = eval_source("{:nested [1 2 3] :answer 42}").unwrap();
    let function = function(&native, "pipeline");
    let output = native
        .call_value(function, &[input])
        .expect("dynamic static-call pipeline");
    assert_eq!(output, expected);
}

#[test]
fn hta_is_the_portable_boundary_for_dynamic_whole_wasm_calls() {
    let mut native = module(
        "(defn annotate [value] (assoc value :answer 42))\n\
         (defn pipeline [value] (annotate value))\n0",
    );
    let arguments =
        eval_source("[{:nested [1 2 3] :label \"historia\" :symbol 'analyzer}]").unwrap();
    let expected =
        eval_source("{:nested [1 2 3] :label \"historia\" :symbol 'analyzer :answer 42}").unwrap();
    let request = crate::hta::encode(&arguments).expect("HTA request");
    let function = function(&native, "pipeline");

    let response = native
        .call_hta(function, &request)
        .expect("portable HTA whole-Wasm call");
    let output = crate::hta::decode(&response).expect("HTA response");

    assert_eq!(output, expected);
}

#[test]
fn hta_boundary_rejects_non_sequential_argument_frames() {
    let mut native = module("(defn echo [value] value)\n0");
    let request = crate::hta::encode(&Value::Number(42)).unwrap();
    let function = function(&native, "echo");

    assert_eq!(
        native.call_hta(function, &request),
        Err("hta/invocation-malformed: expected an HTA sequence of arguments".into())
    );
}

#[test]
fn hta_boundary_does_not_replace_the_scalar_abi() {
    let mut native = scalar_module("(defn increment [value] (+ value 1))\n0", "increment");
    let request = crate::hta::encode(&eval_source("[41]").unwrap()).unwrap();
    let function = function(&native, "increment");

    assert_eq!(
        native.call_hta(function, &request),
        Err(format!(
            "hta/invocation-abi: whole-Wasm function must declare handle-backed arguments and result: {function}"
        ))
    );
    assert_eq!(native.call_i64(function, &[41]), Ok(42));
}

#[test]
fn scalar_entry_calls_keep_the_existing_abi() {
    let mut native = module("(+ 19 23)");
    assert_eq!(native.call_entry_i64(), Ok(42));
}
