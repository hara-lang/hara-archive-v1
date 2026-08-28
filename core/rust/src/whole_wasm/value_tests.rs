use super::{compile_artifact, NativeModule};
use crate::core::{StructType, StructValue, Value};
use crate::kernel::{Form, FunctionSchema, SchemaField, SchemaType};
use crate::vm::{compile_source, eval_source, FunctionId, Program};
use std::rc::Rc;

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

fn typed_module(schema_name: &str, schema: SchemaType) -> NativeModule {
    let mut program = compile_source("(defn echo [value] value)\n0").expect("source must compile");
    program.namespace = Some(TEST_NAMESPACE.into());
    program.schema_types.insert(schema_name.into(), schema);
    program.function_types.insert(
        format!("{TEST_NAMESPACE}/echo"),
        SchemaType::Function(vec![FunctionSchema {
            fixed: vec![SchemaType::Reference(schema_name.into())],
            rest: None,
            output: Box::new(SchemaType::Reference(schema_name.into())),
        }]),
    );
    let artifact = compile_artifact(&program).expect("typed program must lower to whole-Wasm");
    NativeModule::load(&artifact).expect("typed whole-Wasm module must load")
}

fn struct_value(name: &str, fields: &[(&str, Value)]) -> Value {
    let ty = Rc::new(StructType::detached(
        name.into(),
        fields.iter().map(|(field, _)| (*field).into()).collect(),
    ));
    Value::Struct(Rc::new(
        StructValue::from_values(
            ty,
            fields.iter().map(|(_, value)| value.clone()).collect(),
            None,
        )
        .expect("struct fields must match the type"),
    ))
}

fn host_schema() -> (String, SchemaType) {
    let name = format!("{TEST_NAMESPACE}/Host");
    (
        name.clone(),
        SchemaType::Struct {
            name,
            mutable: false,
            fields: vec![
                SchemaField {
                    name: Form::Keyword("environment".into()),
                    properties: None,
                    value_type: SchemaType::Primitive("any".into()),
                },
                SchemaField {
                    name: Form::Keyword("name".into()),
                    properties: None,
                    value_type: SchemaType::Primitive("str".into()),
                },
                SchemaField {
                    name: Form::Keyword("tags".into()),
                    properties: None,
                    value_type: SchemaType::Vector(Box::new(SchemaType::Primitive(
                        "keyword".into(),
                    ))),
                },
            ],
        },
    )
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
    let output = crate::hta::decode_canonical(&response).expect("HTA response");

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
fn typed_struct_schema_round_trips_through_the_hta_boundary() {
    let (schema_name, schema) = host_schema();
    let mut native = typed_module(&schema_name, schema);
    let input = struct_value(
        &schema_name,
        &[
            (
                "environment",
                eval_source("{:region :au}").expect("environment map"),
            ),
            ("name", Value::String("worker".into())),
            (
                "tags",
                Value::Vector(vec![Value::Keyword("service".into())].into()),
            ),
        ],
    );
    let request = crate::hta::encode(&Value::Vector(vec![input].into())).expect("HTA request");
    let function = function(&native, "echo");

    let response = native
        .call_hta(function, &request)
        .expect("typed struct HTA call");
    let Value::Struct(output) = crate::hta::decode_canonical(&response).expect("HTA response")
    else {
        panic!("typed struct response")
    };

    assert_eq!(output.ty.name, schema_name);
    assert_eq!(output.ty.fields, vec!["environment", "name", "tags"]);
    assert_eq!(
        output
            .ordered_values()
            .into_iter()
            .cloned()
            .collect::<Vec<_>>(),
        vec![
            eval_source("{:region :au}").expect("environment map"),
            Value::String("worker".into()),
            Value::Vector(vec![Value::Keyword("service".into())].into()),
        ]
    );
}

#[test]
fn typed_struct_schema_rejects_wrong_nested_hta_values() {
    let (schema_name, schema) = host_schema();
    let mut native = typed_module(&schema_name, schema);
    let input = struct_value(
        &schema_name,
        &[
            ("environment", Value::Nil),
            ("name", Value::String("worker".into())),
            (
                "tags",
                Value::List(vec![Value::Keyword("service".into())].into()),
            ),
        ],
    );
    let request = crate::hta::encode(&Value::Vector(vec![input].into())).expect("HTA request");
    let function = function(&native, "echo");

    assert_eq!(
        native.call_hta(function, &request),
        Err("hta/invocation-schema: argument 0.tags expected a vector, got list".into())
    );
}

#[test]
fn mutable_struct_schema_is_explicitly_rejected_by_hta0() {
    let schema_name = format!("{TEST_NAMESPACE}/Cursor");
    let schema = SchemaType::Struct {
        name: schema_name.clone(),
        mutable: true,
        fields: vec![
            SchemaField {
                name: Form::Keyword("position".into()),
                properties: None,
                value_type: SchemaType::Primitive("int".into()),
            },
            SchemaField {
                name: Form::Keyword("limit".into()),
                properties: Some(Form::Map(vec![(
                    Form::Keyword("optional".into()),
                    Form::Bool(true),
                )])),
                value_type: SchemaType::Primitive("int".into()),
            },
        ],
    };
    let mut native = typed_module(&schema_name, schema);
    let input = struct_value(
        &schema_name,
        &[("position", Value::Number(2)), ("limit", Value::Nil)],
    );
    let request = crate::hta::encode(&Value::Vector(vec![input].into())).expect("HTA request");
    let function = function(&native, "echo");

    assert_eq!(
        native.call_hta(function, &request),
        Err(format!(
            "hta/invocation-schema: argument 0 mutable struct {schema_name} is not transportable over HTA0"
        ))
    );
}

#[test]
fn scalar_entry_calls_keep_the_existing_abi() {
    let mut native = module("(+ 19 23)");
    assert_eq!(native.call_entry_i64(), Ok(42));
}

#[test]
fn scalar_entry_modulo_uses_canonical_remainder_semantics() {
    let mut native = module("(mod -7 3)");
    assert_eq!(native.call_entry_i64(), Ok(-1));

    let mut native = module("(mod 7 -3)");
    assert_eq!(native.call_entry_i64(), Ok(1));
}
