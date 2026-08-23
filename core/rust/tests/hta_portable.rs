use hara_abi::Value as PortableValue;
use hara_wasm::core::Value;
use hara_wasm::hta;
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
