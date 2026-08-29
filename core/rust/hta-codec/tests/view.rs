use hara_abi::ImmutableValue as PortableValue;
use hara_hta::view::{compose_record, compose_vector, Fragment, FrameView, Kind};
use hara_hta::{encode_immutable, MAGIC};
use std::collections::BTreeMap;

const NIL: u8 = 0;
const FALSE: u8 = 1;
const TRUE: u8 = 2;
const I64: u8 = 3;
const STRING: u8 = 4;
const BYTES: u8 = 5;
const KEYWORD: u8 = 6;
const SYMBOL: u8 = 7;
const LIST: u8 = 8;
const VECTOR: u8 = 9;
const SET: u8 = 10;
const MAP: u8 = 11;
const HANDLE: u8 = 12;
const NAMESPACE: u8 = 13;
const VAR: u8 = 14;
const F64: u8 = 15;
const ATOM: u8 = 16;
const ARRAY: u8 = 17;
const OBJECT: u8 = 18;
const CHARACTER: u8 = 19;
const BIG_INTEGER: u8 = 20;
const REGEX: u8 = 22;
const MAP_ENTRY: u8 = 38;

fn sized(tag: u8, value: &[u8]) -> Vec<u8> {
    let mut output = vec![tag];
    output.extend_from_slice(&(value.len() as u32).to_be_bytes());
    output.extend_from_slice(value);
    output
}

fn integer(value: i64) -> Vec<u8> {
    [vec![I64], value.to_be_bytes().to_vec()].concat()
}

fn vector(values: &[Vec<u8>]) -> Vec<u8> {
    let mut output = vec![VECTOR];
    output.extend_from_slice(&(values.len() as u32).to_be_bytes());
    for value in values {
        output.extend_from_slice(value);
    }
    output
}

fn map_entry(key: Vec<u8>, value: Vec<u8>) -> Vec<u8> {
    [
        vec![MAP_ENTRY],
        2_u32.to_be_bytes().to_vec(),
        key,
        value,
    ]
    .concat()
}

fn map(mut entries: Vec<(Vec<u8>, Vec<u8>)>) -> Vec<u8> {
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    let mut output = vec![MAP];
    output.extend_from_slice(&(entries.len() as u32).to_be_bytes());
    for (key, value) in entries {
        output.extend_from_slice(&key);
        output.extend_from_slice(&value);
    }
    output
}

fn frame(bare: Vec<u8>) -> Vec<u8> {
    [MAGIC.as_slice(), bare.as_slice()].concat()
}

#[test]
fn views_closed_argument_envelopes_without_decoding_nested_maps() {
    let nested = map(vec![
        (
            sized(STRING, b"digest-key"),
            vector(&[integer(1), integer(2)]),
        ),
        (sized(STRING, b"other-key"), sized(BYTES, &[0, 1, 255])),
    ]);
    let request = map(vec![
        (sized(KEYWORD, b"operation"), sized(STRING, b"initialize")),
        (
            sized(KEYWORD, b"protocol"),
            sized(STRING, b"example.store-request/1"),
        ),
        (sized(KEYWORD, b"revision"), integer(7)),
        (sized(KEYWORD, b"value"), nested.clone()),
        (
            sized(KEYWORD, b"value-digest"),
            sized(
                STRING,
                b"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            ),
        ),
    ]);
    let arguments = frame(vector(&[request]));

    let parsed = FrameView::parse(&arguments).unwrap();
    let items = parsed.root().vector_items().unwrap();
    assert_eq!(items.len(), 1);
    let request = items[0];
    assert_eq!(
        request
            .require_field("revision")
            .unwrap()
            .integer()
            .unwrap(),
        7
    );
    assert_eq!(
        request
            .require_field("operation")
            .unwrap()
            .string()
            .unwrap(),
        "initialize"
    );
    let value = request.require_field("value").unwrap();
    assert_eq!(value.bare_bytes(), nested.as_slice());
    assert_eq!(value.to_frame(), frame(nested));
}

#[test]
fn scans_every_runtime_wire_shape() {
    let handle = [
        vec![HANDLE],
        (3_u32).to_be_bytes().to_vec(),
        b"own".to_vec(),
        (4_u32).to_be_bytes().to_vec(),
        b"type".to_vec(),
        9_u64.to_be_bytes().to_vec(),
    ]
    .concat();
    let var = [vec![VAR], sized(SYMBOL, b"demo/value"), integer(3)].concat();
    let atom = [vec![ATOM], integer(4)].concat();
    let object = {
        let mut output = vec![OBJECT];
        output.extend_from_slice(&1_u32.to_be_bytes());
        output.extend_from_slice(&sized(STRING, b"key"));
        output.extend_from_slice(&integer(5));
        output
    };
    let character = [vec![CHARACTER], u32::from('λ').to_be_bytes().to_vec()].concat();
    let values = vec![
        vec![NIL],
        vec![FALSE],
        vec![TRUE],
        integer(1),
        [vec![F64], 1.5_f64.to_bits().to_be_bytes().to_vec()].concat(),
        sized(STRING, b"text"),
        sized(BYTES, &[1, 2]),
        sized(KEYWORD, b"key"),
        sized(SYMBOL, b"symbol"),
        [vec![LIST], 0_u32.to_be_bytes().to_vec()].concat(),
        vector(&[]),
        [vec![SET], 0_u32.to_be_bytes().to_vec()].concat(),
        map(vec![]),
        handle,
        sized(NAMESPACE, b"demo"),
        var,
        atom,
        [vec![ARRAY], 0_u32.to_be_bytes().to_vec()].concat(),
        object,
        character,
        sized(BIG_INTEGER, b"12345678901234567890"),
        sized(REGEX, b"a.*b"),
        map_entry(integer(1), integer(2)),
    ];
    for value in values {
        FrameView::parse(&frame(value)).unwrap();
    }
}

#[test]
fn validates_map_entries_as_exact_two_value_sequences() {
    let entry = frame(map_entry(integer(1), integer(2)));
    let parsed = FrameView::parse(&entry).unwrap();
    assert_eq!(parsed.root().kind(), Kind::MapEntry);
    assert_eq!(parsed.root().items().unwrap().len(), 2);

    for count in [0_u32, 1, 3] {
        let mut bare = vec![MAP_ENTRY];
        bare.extend_from_slice(&count.to_be_bytes());
        for _ in 0..count {
            bare.extend_from_slice(&integer(1));
        }
        assert!(FrameView::parse(&frame(bare)).is_err());
    }
}

#[test]
fn rejects_noncanonical_maps_and_sets() {
    let key_a = sized(STRING, b"a");
    let key_b = sized(STRING, b"b");
    let mut unsorted_map = vec![MAP];
    unsorted_map.extend_from_slice(&2_u32.to_be_bytes());
    unsorted_map.extend_from_slice(&key_b);
    unsorted_map.push(NIL);
    unsorted_map.extend_from_slice(&key_a);
    unsorted_map.push(TRUE);
    assert!(FrameView::parse(&frame(unsorted_map))
        .unwrap_err()
        .contains("map keys must be strictly ordered"));

    let mut duplicate_set = vec![SET];
    duplicate_set.extend_from_slice(&2_u32.to_be_bytes());
    duplicate_set.extend_from_slice(&integer(1));
    duplicate_set.extend_from_slice(&integer(1));
    assert!(FrameView::parse(&frame(duplicate_set))
        .unwrap_err()
        .contains("set values must be strictly ordered"));
}

#[test]
fn rejects_noncanonical_big_integer_text() {
    let valid = frame(sized(BIG_INTEGER, b"9223372036854775808"));
    FrameView::parse(&valid).unwrap();

    for text in [b"9223372036854775807".as_slice(), b"009223372036854775808", b"+9223372036854775808", b"-0", b"not-an-integer"] {
        assert!(FrameView::parse(&frame(sized(BIG_INTEGER, text))).is_err());
    }
}

#[test]
fn compose_record_splices_borrowed_values_without_reencoding() {
    let raw = map(vec![(
        sized(STRING, b"string-key"),
        sized(BYTES, &[0, 255]),
    )]);
    let raw_frame = frame(raw.clone());
    let parsed_raw = FrameView::parse(&raw_frame).unwrap();
    let composed = compose_record([
        (
            "operation".to_string(),
            Fragment::Portable(PortableValue::String("load".into())),
        ),
        (
            "protocol".to_string(),
            Fragment::Portable(PortableValue::String("example.store-result/1".into())),
        ),
        ("value".to_string(), Fragment::Borrowed(parsed_raw.root())),
    ])
    .unwrap();

    let result = FrameView::parse(&composed).unwrap();
    let value = result.root().require_field("value").unwrap();
    assert_eq!(value.bare_bytes(), raw.as_slice());
    assert_eq!(value.to_frame(), raw_frame);
}

#[test]
fn compose_vector_and_record_are_canonical_and_reject_duplicate_keys() {
    let vector = compose_vector([
        Fragment::Portable(PortableValue::Integer(1)),
        Fragment::Portable(PortableValue::Boolean(true)),
    ])
    .unwrap();
    let parsed = FrameView::parse(&vector).unwrap();
    assert_eq!(parsed.root().vector_items().unwrap().len(), 2);

    let duplicate = compose_record([
        ("a".to_string(), Fragment::Portable(PortableValue::Nil)),
        (
            "a".to_string(),
            Fragment::Portable(PortableValue::Boolean(true)),
        ),
    ]);
    assert_eq!(duplicate.unwrap_err(), "hta/record-duplicate-key");
}

#[test]
fn scalar_accessors_fail_closed_on_wrong_kinds() {
    let encoded = frame(sized(STRING, b"text"));
    let parsed = FrameView::parse(&encoded).unwrap();
    assert_eq!(parsed.root().string().unwrap(), "text");
    assert!(parsed
        .root()
        .integer()
        .unwrap_err()
        .contains("expected integer"));
    assert!(parsed
        .root()
        .bytes()
        .unwrap_err()
        .contains("expected bytes"));
}

#[test]
fn malformed_runtime_shapes_fail_before_views_escape() {
    let invalid_var = frame([vec![VAR], sized(STRING, b"not-symbol"), vec![NIL]].concat());
    assert!(FrameView::parse(&invalid_var)
        .unwrap_err()
        .contains("invalid var symbol"));

    let invalid_object = {
        let mut bare = vec![OBJECT];
        bare.extend_from_slice(&1_u32.to_be_bytes());
        bare.extend_from_slice(&sized(KEYWORD, b"not-string"));
        bare.push(NIL);
        frame(bare)
    };
    assert!(FrameView::parse(&invalid_object)
        .unwrap_err()
        .contains("invalid object key"));

    let invalid_character = frame([vec![CHARACTER], 0x11_0000_u32.to_be_bytes().to_vec()].concat());
    assert!(FrameView::parse(&invalid_character)
        .unwrap_err()
        .contains("invalid character scalar"));

    let mut trailing = frame(vec![NIL]);
    trailing.push(TRUE);
    assert!(FrameView::parse(&trailing)
        .unwrap_err()
        .contains("trailing bytes"));
}

#[test]
fn portable_fragment_composition_matches_existing_record_encoding() {
    let mut record = BTreeMap::new();
    record.insert("a".to_string(), PortableValue::Integer(1));
    record.insert("z".to_string(), PortableValue::String("last".into()));
    let existing = encode_immutable(&PortableValue::Record(record)).unwrap();
    let composed = compose_record([
        (
            "z".to_string(),
            Fragment::Portable(PortableValue::String("last".into())),
        ),
        (
            "a".to_string(),
            Fragment::Portable(PortableValue::Integer(1)),
        ),
    ])
    .unwrap();
    assert_eq!(composed, existing);
}
