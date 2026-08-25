#![cfg(feature = "whole-wasm")]

use hara_wasm::core::Value;
use hara_wasm::kernel::{Form, FunctionSchema, SchemaField, SchemaType};
use hara_wasm::vm::{compile_source, execute_program};
use hara_wasm::whole_wasm::{compile_artifact, NativeModule};
use std::rc::Rc;

const TEST_NAMESPACE: &str = "hara.whole-wasm.typed-hta-test";

fn typed_module(schema_name: &str, schema: SchemaType) -> NativeModule {
    let mut program = compile_source("(defn echo [value] value)\n0").expect("source compiles");
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
    let artifact = compile_artifact(&program).expect("typed source lowers to HNW0");
    NativeModule::load(&artifact).expect("typed HNW0 loads")
}

fn value_from_source(source: &str) -> Value {
    execute_program(Rc::new(compile_source(source).expect("fixture compiles")))
        .expect("fixture executes")
}

fn function(module: &NativeModule, name: &str) -> u16 {
    module
        .artifact()
        .program
        .functions
        .iter()
        .position(|candidate| candidate.name.as_deref() == Some(name))
        .expect("named function") as u16
}

fn host_schema(name: String) -> SchemaType {
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
                value_type: SchemaType::Vector(Box::new(SchemaType::Primitive("keyword".into()))),
            },
        ],
    }
}

#[test]
fn typed_defstruct_values_are_checked_and_round_tripped_by_hta() {
    let input = value_from_source(
        "(defstruct Host [[environment :any] [name :str] [tags [:vector :keyword]]])
         (Host {:region :au} \"worker\" [:service])",
    );
    let Value::Struct(structure) = &input else {
        panic!("typed defstruct fixture")
    };
    let schema_name = structure.ty.name.clone();
    let mut native = typed_module(&schema_name, host_schema(schema_name.clone()));
    let request = hara_wasm::hta::encode(&Value::Vector(vec![input].into())).unwrap();
    let response = native
        .call_hta(function(&native, "echo"), &request)
        .expect("typed HTA call");
    let Value::Struct(output) = hara_wasm::hta::decode(&response).unwrap() else {
        panic!("typed struct response")
    };

    assert_eq!(output.ty.name, schema_name);
    assert_eq!(output.ty.fields, vec!["environment", "name", "tags"]);
}

#[test]
fn legacy_dynamic_schemas_remain_permissive_at_the_hta_boundary() {
    let schema_name = format!("{TEST_NAMESPACE}/Legacy");
    let mut native = typed_module(&schema_name, SchemaType::Primitive("any".into()));
    let input = value_from_source("{:nested [1 2 3] :label \"legacy\"}");
    let request = hara_wasm::hta::encode(&Value::Vector(vec![input.clone()].into())).unwrap();

    let response = native
        .call_hta(function(&native, "echo"), &request)
        .expect("legacy dynamic HTA call");

    assert_eq!(hara_wasm::hta::decode(&response).unwrap(), input);
}

#[test]
fn typed_struct_validation_reports_nested_shape_errors_before_calling_wasm() {
    let input = value_from_source(
        "(defstruct Host [[environment :any] [name :str] [tags [:vector :keyword]]])
         (Host nil \"worker\" '(:service))",
    );
    let Value::Struct(structure) = &input else {
        panic!("typed defstruct fixture")
    };
    let schema_name = structure.ty.name.clone();
    let mut native = typed_module(&schema_name, host_schema(schema_name.clone()));
    let request = hara_wasm::hta::encode(&Value::Vector(vec![input].into())).unwrap();

    assert_eq!(
        native.call_hta(function(&native, "echo"), &request),
        Err("hta/invocation-schema: argument 0.tags expected a vector, got list".into())
    );
}

#[test]
fn mutable_schema_declarations_are_rejected_until_hta_has_mutable_transport() {
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
    let input = value_from_source(
        "(defstruct Cursor [[position :int] [limit {:optional true} :int]])
         (Cursor 2 nil)",
    );
    let mut native = typed_module(&schema_name, schema);
    let request = hara_wasm::hta::encode(&Value::Vector(vec![input].into())).unwrap();

    assert_eq!(
        native.call_hta(function(&native, "echo"), &request),
        Err(format!(
            "hta/invocation-schema: argument 0 mutable struct {schema_name} is not transportable over HTA0"
        ))
    );
}
