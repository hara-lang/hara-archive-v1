use hara_abi::Value as PortableValue;
use hara_wasm::core::Value;
use hara_wasm::hta;
use hara_wasm::kernel::{parse_forms, Form};
use hara_wasm::spec_registry;
use num_bigint::BigInt;
use std::collections::BTreeMap;

fn runtime_record(entries: impl IntoIterator<Item = (&'static str, Value)>) -> Value {
    Value::Map(
        entries
            .into_iter()
            .map(|(key, value)| (Value::Keyword(key.into()), value))
            .collect(),
    )
}

fn portable_record(
    entries: impl IntoIterator<Item = (&'static str, PortableValue)>,
) -> PortableValue {
    PortableValue::Record(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect::<BTreeMap<_, _>>(),
    )
}

#[test]
fn portable_codec_matches_the_runtime_codec_byte_for_byte() {
    let runtime = runtime_record([
        (
            "a",
            Value::Vector(vec![Value::Bool(true), Value::Nil].into()),
        ),
        ("b", Value::Number(2)),
        (
            "big",
            Value::BigInteger(BigInt::parse_bytes(b"9223372036854775808", 10).unwrap()),
        ),
        ("bytes", Value::Bytes(vec![0, 1, 255].into())),
        ("float", Value::Float(0.28)),
        ("keyword", Value::Keyword("profile.primary".into())),
        ("string", Value::String("portable".into())),
    ]);
    let portable = portable_record([
        (
            "a",
            PortableValue::Vector(vec![PortableValue::Boolean(true), PortableValue::Nil]),
        ),
        ("b", PortableValue::Integer(2)),
        (
            "big",
            PortableValue::BigInteger("9223372036854775808".into()),
        ),
        ("bytes", PortableValue::Bytes(vec![0, 1, 255])),
        ("float", PortableValue::Float(0.28)),
        ("keyword", PortableValue::Keyword("profile.primary".into())),
        ("string", PortableValue::String("portable".into())),
    ]);

    let runtime_bytes = hta::encode(&runtime).unwrap();
    let portable_bytes = hara_hta::encode(&portable).unwrap();
    assert_eq!(portable_bytes, runtime_bytes);
    assert_eq!(hara_hta::decode(&runtime_bytes).unwrap(), portable);
    assert_eq!(hta::decode(&portable_bytes).unwrap(), runtime);
}

#[test]
fn portable_record_order_uses_the_runtime_canonical_key_sort() {
    let runtime = runtime_record([
        ("z", Value::Number(1)),
        ("aa", Value::Number(2)),
        ("a", Value::Number(3)),
    ]);
    let portable = portable_record([
        ("a", PortableValue::Integer(3)),
        ("z", PortableValue::Integer(1)),
        ("aa", PortableValue::Integer(2)),
    ]);
    assert_eq!(
        hta::encode(&runtime).unwrap(),
        hara_hta::encode(&portable).unwrap()
    );
}

#[test]
fn registry_golden_vector_matches_rust_encoding() {
    let source = std::fs::read_to_string(spec_registry::require(
        "02-platform/000050-transport-hta/draft/conformance/transport-hta.edn",
    ))
    .expect("HTA conformance suite is readable");
    let root = parse_forms(&source)
        .expect("HTA conformance suite parses")
        .into_iter()
        .next()
        .expect("HTA conformance suite has a root");
    let Form::Map(root) = root else {
        panic!("HTA conformance suite root must be a map");
    };
    let Form::Vector(cases) = lookup(&root, "suite/cases") else {
        panic!("HTA conformance suite cases must be a vector");
    };
    let case = cases
        .iter()
        .find(|case| matches!(lookup_map(case, "case/id"), Form::Keyword(id) if id == "hta.case/golden-vector"))
        .expect("registry HTA golden-vector case");
    let Form::Vector(input) = lookup_map(case, "case/input") else {
        panic!("HTA golden-vector input must be a vector");
    };
    let Form::Vector(expected) = lookup_map(case, "case/expect") else {
        panic!("HTA golden-vector expectation must be a vector");
    };
    let values = input.iter().map(runtime_value).collect::<Vec<_>>();
    let expected = expected
        .iter()
        .map(|value| match value {
            Form::Number(value) => u8::try_from(*value).expect("golden byte fits in u8"),
            other => panic!("HTA golden byte must be a number: {other:?}"),
        })
        .collect::<Vec<_>>();
    assert_eq!(
        expected,
        hta::encode(&Value::Vector(values.into())).unwrap()
    );
}

fn runtime_value(form: &Form) -> Value {
    match form {
        Form::Nil => Value::Nil,
        Form::Bool(value) => Value::Bool(*value),
        Form::Number(value) => Value::Number(*value),
        Form::String(value) => Value::String(value.clone()),
        other => panic!("unsupported HTA golden input: {other:?}"),
    }
}

fn lookup<'a>(map: &'a [(Form, Form)], key: &str) -> &'a Form {
    map.iter()
        .find_map(|(candidate, value)| (candidate == &Form::Keyword(key.into())).then_some(value))
        .unwrap_or_else(|| panic!("missing registry key :{key}"))
}

fn lookup_map<'a>(form: &'a Form, key: &str) -> &'a Form {
    let Form::Map(map) = form else {
        panic!("registry case must be a map");
    };
    lookup(map, key)
}
