//! Borrowed structural views over exact HTA0 frames.
//!
//! This module validates the complete runtime wire format without constructing
//! executable Hara values. Callers can inspect a closed envelope while retaining
//! exact nested value spans for hashing, storage, or later composition.

use super::{
    encode_value, ARRAY, ATOM, BIG_INTEGER, BYTES, CHARACTER, CONS, EXCEPTION_INFO, F64, FALSE,
    HANDLE, I64, KEYWORD, LIST, MAGIC, MAP, MAP_ENTRY, MAX_FRAME_BYTES, MAX_NESTING_DEPTH,
    NAMESPACE, NIL, OBJECT, ORDERED_MAP, ORDERED_SET, POINTER, QUEUE, REGEX, SET, SORTED_MAP,
    SORTED_SET, STRING, STRUCT, SYMBOL, TAGGED, TRIE, TRUE, TUPLE, VAR, VAR_REF, VECTOR,
};
use hara_abi::ImmutableValue as PortableValue;
use std::str;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Nil,
    Boolean,
    Integer,
    Float,
    String,
    Bytes,
    Keyword,
    Symbol,
    List,
    Vector,
    Set,
    Map,
    Handle,
    Namespace,
    Var,
    Atom,
    Array,
    Object,
    Character,
    BigInteger,
    Regex,
    Tuple,
    Cons,
    Queue,
    OrderedMap,
    SortedMap,
    Trie,
    OrderedSet,
    SortedSet,
    Tagged,
    ExceptionInfo,
    Struct,
    Pointer,
    VarRef,
    MapEntry,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameView<'a> {
    bytes: &'a [u8],
    root: ValueView<'a>,
}

impl<'a> FrameView<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, String> {
        if bytes.len() > MAX_FRAME_BYTES {
            return Err(format!(
                "hta/frame-too-large: {} exceeds {} bytes",
                bytes.len(),
                MAX_FRAME_BYTES
            ));
        }
        if !bytes.starts_with(MAGIC) {
            return Err("hta/frame-invalid: expected HTA0 magic".into());
        }
        let bare = &bytes[MAGIC.len()..];
        let end = scan_value(bare, 0, 0)?;
        if end != bare.len() {
            return Err("hta/frame-invalid: trailing bytes".into());
        }
        Ok(Self {
            bytes,
            root: ValueView { bare },
        })
    }

    pub const fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    pub const fn root(&self) -> ValueView<'a> {
        self.root
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ValueView<'a> {
    bare: &'a [u8],
}

impl<'a> ValueView<'a> {
    pub const fn bare_bytes(&self) -> &'a [u8] {
        self.bare
    }

    pub fn to_frame(&self) -> Vec<u8> {
        let mut output = Vec::with_capacity(MAGIC.len() + self.bare.len());
        output.extend_from_slice(MAGIC);
        output.extend_from_slice(self.bare);
        output
    }

    pub fn kind(&self) -> Kind {
        match self.bare[0] {
            NIL => Kind::Nil,
            FALSE | TRUE => Kind::Boolean,
            I64 => Kind::Integer,
            F64 => Kind::Float,
            STRING => Kind::String,
            BYTES => Kind::Bytes,
            KEYWORD => Kind::Keyword,
            SYMBOL => Kind::Symbol,
            LIST => Kind::List,
            VECTOR => Kind::Vector,
            SET => Kind::Set,
            MAP => Kind::Map,
            HANDLE => Kind::Handle,
            NAMESPACE => Kind::Namespace,
            VAR => Kind::Var,
            ATOM => Kind::Atom,
            ARRAY => Kind::Array,
            OBJECT => Kind::Object,
            CHARACTER => Kind::Character,
            BIG_INTEGER => Kind::BigInteger,
            REGEX => Kind::Regex,
            TUPLE => Kind::Tuple,
            MAP_ENTRY => Kind::MapEntry,
            CONS => Kind::Cons,
            QUEUE => Kind::Queue,
            ORDERED_MAP => Kind::OrderedMap,
            SORTED_MAP => Kind::SortedMap,
            TRIE => Kind::Trie,
            ORDERED_SET => Kind::OrderedSet,
            SORTED_SET => Kind::SortedSet,
            TAGGED => Kind::Tagged,
            EXCEPTION_INFO => Kind::ExceptionInfo,
            STRUCT => Kind::Struct,
            POINTER => Kind::Pointer,
            VAR_REF => Kind::VarRef,
            _ => unreachable!("validated ValueView always has a known tag"),
        }
    }

    pub fn boolean(&self) -> Result<bool, String> {
        match self.bare[0] {
            FALSE => Ok(false),
            TRUE => Ok(true),
            _ => Err(kind_error("boolean", self.kind())),
        }
    }

    pub fn integer(&self) -> Result<i64, String> {
        if self.bare[0] != I64 {
            return Err(kind_error("integer", self.kind()));
        }
        Ok(i64::from_be_bytes(
            self.bare[1..9].try_into().expect("validated i64 span"),
        ))
    }

    pub fn float_bits(&self) -> Result<u64, String> {
        if self.bare[0] != F64 {
            return Err(kind_error("float", self.kind()));
        }
        Ok(u64::from_be_bytes(
            self.bare[1..9].try_into().expect("validated f64 span"),
        ))
    }

    pub fn character(&self) -> Result<char, String> {
        if self.bare[0] != CHARACTER {
            return Err(kind_error("character", self.kind()));
        }
        let scalar = u32::from_be_bytes(
            self.bare[1..5]
                .try_into()
                .expect("validated character span"),
        );
        char::from_u32(scalar).ok_or_else(|| "hta/value-malformed: invalid character scalar".into())
    }

    pub fn text(&self) -> Result<&'a str, String> {
        match self.bare[0] {
            STRING | KEYWORD | SYMBOL | NAMESPACE | BIG_INTEGER | REGEX => {
                str::from_utf8(sized_payload(self.bare)?)
                    .map_err(|_| "hta/value-malformed: text payload is not valid UTF-8".into())
            }
            _ => Err(kind_error("text scalar", self.kind())),
        }
    }

    pub fn string(&self) -> Result<&'a str, String> {
        self.exact_text(STRING, "string")
    }

    pub fn keyword(&self) -> Result<&'a str, String> {
        self.exact_text(KEYWORD, "keyword")
    }

    pub fn symbol(&self) -> Result<&'a str, String> {
        self.exact_text(SYMBOL, "symbol")
    }

    pub fn bytes(&self) -> Result<&'a [u8], String> {
        if self.bare[0] != BYTES {
            return Err(kind_error("bytes", self.kind()));
        }
        sized_payload(self.bare)
    }

    pub fn items(&self) -> Result<Vec<ValueView<'a>>, String> {
        match self.bare[0] {
            LIST | VECTOR | SET | ARRAY | TUPLE | MAP_ENTRY | CONS | QUEUE | ORDERED_SET
            | SORTED_SET => sequence_items(self.bare),
            _ => Err(kind_error("sequence", self.kind())),
        }
    }

    pub fn vector_items(&self) -> Result<Vec<ValueView<'a>>, String> {
        if self.bare[0] != VECTOR {
            return Err(kind_error("vector", self.kind()));
        }
        sequence_items(self.bare)
    }

    pub fn map_entries(&self) -> Result<Vec<(ValueView<'a>, ValueView<'a>)>, String> {
        if !matches!(self.bare[0], MAP | ORDERED_MAP | SORTED_MAP | TRIE) {
            return Err(kind_error("map", self.kind()));
        }
        map_entries(self.bare)
    }

    pub fn record_entries(&self) -> Result<Vec<(&'a str, ValueView<'a>)>, String> {
        self.map_entries()?
            .into_iter()
            .map(|(key, value)| Ok((key.keyword()?, value)))
            .collect()
    }

    pub fn field(&self, keyword: &str) -> Result<Option<ValueView<'a>>, String> {
        for (key, value) in self.record_entries()? {
            if key == keyword {
                return Ok(Some(value));
            }
        }
        Ok(None)
    }

    pub fn require_field(&self, keyword: &str) -> Result<ValueView<'a>, String> {
        self.field(keyword)?
            .ok_or_else(|| format!("hta/record-field-missing: expected :{keyword}"))
    }

    fn exact_text(&self, tag: u8, label: &'static str) -> Result<&'a str, String> {
        if self.bare[0] != tag {
            return Err(kind_error(label, self.kind()));
        }
        str::from_utf8(sized_payload(self.bare)?)
            .map_err(|_| format!("hta/value-malformed: {label} is not valid UTF-8"))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Fragment<'a> {
    Portable(PortableValue),
    Borrowed(ValueView<'a>),
}

impl<'a> Fragment<'a> {
    fn bare_bytes(self, depth: usize) -> Result<Vec<u8>, String> {
        match self {
            Self::Portable(value) => {
                let mut output = Vec::new();
                encode_value(&value, depth, &mut output)?;
                Ok(output)
            }
            Self::Borrowed(value) => {
                let end = scan_value(value.bare_bytes(), 0, depth)?;
                if end != value.bare_bytes().len() {
                    return Err("hta/value-malformed: trailing borrowed bytes".into());
                }
                Ok(value.bare_bytes().to_vec())
            }
        }
    }
}

pub fn compose_vector<'a>(
    values: impl IntoIterator<Item = Fragment<'a>>,
) -> Result<Vec<u8>, String> {
    let values = values.into_iter().collect::<Vec<_>>();
    let mut output = MAGIC.to_vec();
    output.push(VECTOR);
    write_len(&mut output, values.len())?;
    for value in values {
        output.extend_from_slice(&value.bare_bytes(1)?);
        ensure_frame_limit(&output)?;
    }
    Ok(output)
}

pub fn compose_record<'a>(
    entries: impl IntoIterator<Item = (String, Fragment<'a>)>,
) -> Result<Vec<u8>, String> {
    let entries = entries.into_iter().collect::<Vec<_>>();
    let mut encoded = Vec::with_capacity(entries.len());
    for (key, value) in entries {
        let mut key_bytes = Vec::new();
        encode_value(&PortableValue::Keyword(key), 1, &mut key_bytes)?;
        encoded.push((key_bytes, value.bare_bytes(1)?));
    }
    encoded.sort_by(|left, right| left.0.cmp(&right.0));
    for pair in encoded.windows(2) {
        if pair[0].0 == pair[1].0 {
            return Err("hta/record-duplicate-key".into());
        }
    }

    let mut output = MAGIC.to_vec();
    output.push(MAP);
    write_len(&mut output, encoded.len())?;
    for (key, value) in encoded {
        output.extend_from_slice(&key);
        output.extend_from_slice(&value);
        ensure_frame_limit(&output)?;
    }
    Ok(output)
}

fn kind_error(expected: &'static str, actual: Kind) -> String {
    format!("hta/value-kind: expected {expected}, found {actual:?}")
}

fn scan_value(bytes: &[u8], start: usize, depth: usize) -> Result<usize, String> {
    if depth > MAX_NESTING_DEPTH {
        return Err("hta/value-too-deep".into());
    }
    let tag = *bytes
        .get(start)
        .ok_or_else(|| "hta/value-malformed: truncated value".to_string())?;
    let cursor = start + 1;
    match tag {
        NIL | FALSE | TRUE => Ok(cursor),
        I64 | F64 => take_end(bytes, cursor, 8),
        CHARACTER => {
            let end = take_end(bytes, cursor, 4)?;
            let scalar =
                u32::from_be_bytes(bytes[cursor..end].try_into().expect("four character bytes"));
            if char::from_u32(scalar).is_none() {
                return Err("hta/value-malformed: invalid character scalar".into());
            }
            Ok(end)
        }
        STRING | KEYWORD | SYMBOL | NAMESPACE | BIG_INTEGER | REGEX => {
            let (data_start, end) = sized_range(bytes, cursor)?;
            let text = str::from_utf8(&bytes[data_start..end])
                .map_err(|_| "hta/value-malformed: invalid UTF-8".to_string())?;
            if tag == BIG_INTEGER {
                super::validate_canonical_big_integer(text)?;
            }
            Ok(end)
        }
        BYTES => sized_range(bytes, cursor).map(|(_, end)| end),
        LIST | VECTOR | ARRAY | TUPLE | CONS | QUEUE | ORDERED_SET => {
            scan_sequence(bytes, cursor, depth, false)
        }
        MAP_ENTRY => scan_map_entry(bytes, cursor, depth),
        SET | SORTED_SET => scan_sequence(bytes, cursor, depth, true),
        MAP => scan_map(bytes, cursor, depth, true),
        ORDERED_MAP | SORTED_MAP | TRIE => scan_map(bytes, cursor, depth, false),
        OBJECT => scan_object(bytes, cursor, depth),
        VAR => {
            let symbol_end = scan_value(bytes, cursor, depth + 1)?;
            if bytes[cursor] != SYMBOL {
                return Err("hta/value-malformed: invalid var symbol".into());
            }
            scan_value(bytes, symbol_end, depth + 1)
        }
        ATOM => scan_value(bytes, cursor, depth + 1),
        HANDLE => {
            let (provider_start, provider_end) = sized_range(bytes, cursor)?;
            str::from_utf8(&bytes[provider_start..provider_end])
                .map_err(|_| "hta/value-malformed: invalid handle owner".to_string())?;
            let (type_start, type_end) = sized_range(bytes, provider_end)?;
            str::from_utf8(&bytes[type_start..type_end])
                .map_err(|_| "hta/value-malformed: invalid handle type".to_string())?;
            take_end(bytes, type_end, 8)
        }
        TAGGED => {
            let tag_end = scan_value(bytes, cursor, depth + 1)?;
            if bytes[cursor] != SYMBOL {
                return Err("hta/value-malformed: invalid tagged literal tag".into());
            }
            scan_value(bytes, tag_end, depth + 1)
        }
        EXCEPTION_INFO => {
            let message_end = scan_value(bytes, cursor, depth + 1)?;
            if bytes[cursor] != STRING {
                return Err("hta/value-malformed: invalid exception message".into());
            }
            let data_end = scan_value(bytes, message_end, depth + 1)?;
            let cause_end = scan_value(bytes, data_end, depth + 1)?;
            scan_value(bytes, cause_end, depth + 1)
        }
        STRUCT => {
            let name_end = scan_value(bytes, cursor, depth + 1)?;
            if bytes[cursor] != STRING {
                return Err("hta/value-malformed: invalid struct name".into());
            }
            let fields_end = scan_value(bytes, name_end, depth + 1)?;
            if bytes[name_end] != VECTOR {
                return Err("hta/value-malformed: invalid struct fields".into());
            }
            let values_end = scan_value(bytes, fields_end, depth + 1)?;
            if bytes[fields_end] != VECTOR {
                return Err("hta/value-malformed: invalid struct values".into());
            }
            Ok(values_end)
        }
        POINTER => {
            let context_end = scan_value(bytes, cursor, depth + 1)?;
            if bytes[cursor] != KEYWORD {
                return Err("hta/value-malformed: invalid pointer context".into());
            }
            let fields_end = scan_value(bytes, context_end, depth + 1)?;
            if bytes[context_end] != MAP {
                return Err("hta/value-malformed: invalid pointer fields".into());
            }
            Ok(fields_end)
        }
        VAR_REF => {
            let end = scan_value(bytes, cursor, depth + 1)?;
            if bytes[cursor] != SYMBOL {
                return Err("hta/value-malformed: invalid Var reference".into());
            }
            Ok(end)
        }
        _ => Err(format!("hta/value-malformed: unknown tag {tag}")),
    }
}

fn scan_sequence(
    bytes: &[u8],
    cursor: usize,
    depth: usize,
    canonical_order: bool,
) -> Result<usize, String> {
    let (count, mut cursor) = read_len(bytes, cursor)?;
    if count > bytes.len().saturating_sub(cursor) {
        return Err("hta/value-malformed: impossible sequence length".into());
    }
    let mut previous: Option<&[u8]> = None;
    for _ in 0..count {
        let start = cursor;
        cursor = scan_value(bytes, cursor, depth + 1)?;
        let current = &bytes[start..cursor];
        if canonical_order {
            if previous.is_some_and(|value| value >= current) {
                return Err("hta/value-noncanonical: set values must be strictly ordered".into());
            }
            previous = Some(current);
        }
    }
    Ok(cursor)
}

fn scan_map_entry(bytes: &[u8], cursor: usize, depth: usize) -> Result<usize, String> {
    let (count, cursor) = read_len(bytes, cursor)?;
    if count != 2 {
        return Err("hta/value-malformed: map entry must contain two values".into());
    }
    let cursor = scan_value(bytes, cursor, depth + 1)?;
    scan_value(bytes, cursor, depth + 1)
}

fn scan_map(
    bytes: &[u8],
    cursor: usize,
    depth: usize,
    canonical_order: bool,
) -> Result<usize, String> {
    let (count, mut cursor) = read_len(bytes, cursor)?;
    if count > bytes.len().saturating_sub(cursor) / 2 {
        return Err("hta/value-malformed: impossible map length".into());
    }
    let mut previous: Option<&[u8]> = None;
    for _ in 0..count {
        let key_start = cursor;
        cursor = scan_value(bytes, cursor, depth + 1)?;
        let key = &bytes[key_start..cursor];
        if canonical_order && previous.is_some_and(|value| value >= key) {
            return Err("hta/value-noncanonical: map keys must be strictly ordered".into());
        }
        previous = Some(key);
        cursor = scan_value(bytes, cursor, depth + 1)?;
    }
    Ok(cursor)
}

fn scan_object(bytes: &[u8], cursor: usize, depth: usize) -> Result<usize, String> {
    let (count, mut cursor) = read_len(bytes, cursor)?;
    if count > bytes.len().saturating_sub(cursor) / 2 {
        return Err("hta/value-malformed: impossible object length".into());
    }
    for _ in 0..count {
        let key_start = cursor;
        cursor = scan_value(bytes, cursor, depth + 1)?;
        if bytes[key_start] != STRING {
            return Err("hta/value-malformed: invalid object key".into());
        }
        cursor = scan_value(bytes, cursor, depth + 1)?;
    }
    Ok(cursor)
}

fn sequence_items(bytes: &[u8]) -> Result<Vec<ValueView<'_>>, String> {
    let (count, mut cursor) = read_len(bytes, 1)?;
    let mut output = Vec::with_capacity(count);
    for _ in 0..count {
        let start = cursor;
        cursor = scan_value(bytes, cursor, 1)?;
        output.push(ValueView {
            bare: &bytes[start..cursor],
        });
    }
    Ok(output)
}

fn map_entries(bytes: &[u8]) -> Result<Vec<(ValueView<'_>, ValueView<'_>)>, String> {
    let (count, mut cursor) = read_len(bytes, 1)?;
    let mut output = Vec::with_capacity(count);
    for _ in 0..count {
        let key_start = cursor;
        cursor = scan_value(bytes, cursor, 1)?;
        let key = ValueView {
            bare: &bytes[key_start..cursor],
        };
        let value_start = cursor;
        cursor = scan_value(bytes, cursor, 1)?;
        let value = ValueView {
            bare: &bytes[value_start..cursor],
        };
        output.push((key, value));
    }
    Ok(output)
}

fn sized_payload(bytes: &[u8]) -> Result<&[u8], String> {
    let (start, end) = sized_range(bytes, 1)?;
    Ok(&bytes[start..end])
}

fn sized_range(bytes: &[u8], cursor: usize) -> Result<(usize, usize), String> {
    let (len, start) = read_len(bytes, cursor)?;
    let end = take_end(bytes, start, len)?;
    Ok((start, end))
}

fn read_len(bytes: &[u8], cursor: usize) -> Result<(usize, usize), String> {
    let end = take_end(bytes, cursor, 4)?;
    let len = u32::from_be_bytes(bytes[cursor..end].try_into().expect("four length bytes"));
    Ok((len as usize, end))
}

fn take_end(bytes: &[u8], cursor: usize, size: usize) -> Result<usize, String> {
    let end = cursor
        .checked_add(size)
        .ok_or_else(|| "hta/value-malformed: length overflow".to_string())?;
    if end > bytes.len() {
        return Err("hta/value-malformed: truncated value".into());
    }
    Ok(end)
}

fn write_len(output: &mut Vec<u8>, len: usize) -> Result<(), String> {
    let len = u32::try_from(len)
        .map_err(|_| "hta/value-too-large: container length exceeds u32".to_string())?;
    output.extend_from_slice(&len.to_be_bytes());
    ensure_frame_limit(output)
}

fn ensure_frame_limit(output: &[u8]) -> Result<(), String> {
    if output.len() > MAX_FRAME_BYTES {
        Err(format!(
            "hta/frame-too-large: encoded frame exceeds {} bytes",
            MAX_FRAME_BYTES
        ))
    } else {
        Ok(())
    }
}
