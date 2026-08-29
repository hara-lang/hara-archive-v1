//! Dependency-free canonical HTA0 codec for portable Hara ABI values.
//!
//! `hara-hta` deliberately operates on [`hara_abi::Value`] rather than the
//! executable runtime value graph. It is suitable for native providers,
//! package tooling, embedding hosts, and durable state boundaries that need
//! canonical Hara bytes without linking the VM, Wasmtime, or host services.

pub mod view;

use hara_abi::{
    ExceptionProvenance, ExceptionSite, ImmutableValue as Value, Value as AbiValue,
};
use std::collections::BTreeMap;

pub const MAGIC: &[u8; 4] = b"HTA0";
pub const MAX_FRAME_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_NESTING_DEPTH: usize = 256;

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
const TUPLE: u8 = 23;
const MAP_ENTRY: u8 = 38;
const CONS: u8 = 24;
const QUEUE: u8 = 25;
const ORDERED_MAP: u8 = 26;
const SORTED_MAP: u8 = 27;
const TRIE: u8 = 28;
const ORDERED_SET: u8 = 29;
const SORTED_SET: u8 = 30;
const TAGGED: u8 = 31;
const EXCEPTION_INFO: u8 = 32;
const STRUCT: u8 = 33;
const POINTER: u8 = 34;
const VAR_REF: u8 = 35;

fn provenance_value(provenance: &ExceptionProvenance) -> Value {
    let mut fields = BTreeMap::new();
    fields.insert(
        "ex/created-at".into(),
        provenance
            .created_at
            .as_ref()
            .map(site_value)
            .unwrap_or(Value::Nil),
    );
    fields.insert(
        "ex/throws".into(),
        Value::Vector(provenance.throws.iter().map(site_value).collect()),
    );
    Value::Record(fields)
}

fn site_value(site: &ExceptionSite) -> Value {
    let mut fields = BTreeMap::new();
    fields.insert(
        "namespace".into(),
        site.namespace.clone().map(Value::String).unwrap_or(Value::Nil),
    );
    fields.insert(
        "resource".into(),
        site.resource.clone().map(Value::String).unwrap_or(Value::Nil),
    );
    fields.insert("line".into(), Value::Integer(site.line as i64));
    fields.insert("column".into(), Value::Integer(site.column as i64));
    Value::Record(fields)
}

fn decode_provenance(value: Value) -> Result<ExceptionProvenance, String> {
    let Value::Record(fields) = value else {
        return Err("hta/value-malformed: invalid exception provenance".into());
    };
    if fields.len() != 2 || !fields.contains_key("ex/created-at") || !fields.contains_key("ex/throws") {
        return Err("hta/value-malformed: invalid exception provenance fields".into());
    }
    let created_at = match fields.get("ex/created-at").expect("checked above") {
        Value::Nil => None,
        value => Some(decode_site(value)?),
    };
    let Value::Vector(throws) = fields.get("ex/throws").expect("checked above") else {
        return Err("hta/value-malformed: invalid exception throws provenance".into());
    };
    Ok(ExceptionProvenance {
        created_at,
        throws: throws.iter().map(decode_site).collect::<Result<_, _>>()?,
    })
}

fn decode_site(value: &Value) -> Result<ExceptionSite, String> {
    let Value::Record(fields) = value else {
        return Err("hta/value-malformed: invalid exception provenance site".into());
    };
    if fields.len() != 4
        || !fields.contains_key("namespace")
        || !fields.contains_key("resource")
        || !fields.contains_key("line")
        || !fields.contains_key("column")
    {
        return Err("hta/value-malformed: invalid exception provenance site".into());
    }
    let optional_string = |name: &str| match fields.get(name).expect("checked above") {
        Value::Nil => Ok(None),
        Value::String(value) => Ok(Some(value.clone())),
        _ => Err(format!("hta/value-malformed: invalid exception provenance {name}")),
    };
    let nonnegative = |name: &str| match fields.get(name).expect("checked above") {
        Value::Integer(value) if *value >= 0 => Ok(*value as u64),
        _ => Err(format!("hta/value-malformed: invalid exception provenance {name}")),
    };
    Ok(ExceptionSite {
        namespace: optional_string("namespace")?,
        resource: optional_string("resource")?,
        line: nonnegative("line")?,
        column: nonnegative("column")?,
    })
}

/// Encode one portable value as an exact canonical HTA0 frame.
pub fn encode_immutable(value: &Value) -> Result<Vec<u8>, String> {
    let mut output = Vec::with_capacity(128);
    write(&mut output, MAGIC)?;
    encode_value(value, 0, &mut output)?;
    Ok(output)
}

/// Decode one exact canonical HTA0 frame into a portable value.
///
/// Runtime-only wire tags such as symbols, lists, sets, handles, namespaces,
/// vars, atoms, arrays, objects, characters, big integers, and regex values
/// fail closed. HTA maps decode only when every key is a unique keyword, which
/// maps directly to [`Value::Record`].
pub fn decode_immutable(bytes: &[u8]) -> Result<Value, String> {
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
    let mut reader = Reader {
        bytes: &bytes[MAGIC.len()..],
        cursor: 0,
    };
    let value = reader.value(0)?;
    if reader.cursor != reader.bytes.len() {
        return Err("hta/frame-invalid: trailing bytes".into());
    }
    Ok(value)
}

/// Decode one portable HTA0 value only when the supplied bytes are bounded and
/// already use the exact canonical encoding produced by [`encode`].
///
/// This is the generic provider boundary for small values. It deliberately does
/// not read an object, compute a digest, select a provider, or interpret an
/// application schema. Callers must verify immutable-object identity separately.
pub fn decode_immutable_canonical(bytes: &[u8], max_bytes: usize) -> Result<Value, String> {
    if max_bytes == 0 || max_bytes > MAX_FRAME_BYTES {
        return Err(format!(
            "hta/maximum-invalid: requested maximum must be between 1 and {MAX_FRAME_BYTES} bytes"
        ));
    }
    if bytes.len() > max_bytes {
        return Err(format!(
            "hta/frame-too-large: {} exceeds requested maximum {} bytes",
            bytes.len(),
            max_bytes
        ));
    }

    let value = decode_immutable(bytes)?;
    let canonical = encode_immutable(&value)?;
    if canonical != bytes {
        return Err("hta/frame-noncanonical: decoded value has different canonical bytes".into());
    }
    Ok(value)
}

/// Encode the stable provider ABI subset using the full immutable codec.
pub fn encode(value: &AbiValue) -> Result<Vec<u8>, String> {
    encode_immutable(&abi_to_immutable(value)?)
}

/// Decode only values representable by the stable provider ABI subset.
pub fn decode(bytes: &[u8]) -> Result<AbiValue, String> {
    immutable_to_abi(decode_immutable(bytes)?)
}

pub fn decode_canonical(bytes: &[u8], max_bytes: usize) -> Result<AbiValue, String> {
    immutable_to_abi(decode_immutable_canonical(bytes, max_bytes)?)
}

pub(crate) fn canonical_big_integer(value: &str) -> Result<Value, String> {
    let (negative, digits) = match value.strip_prefix('-') {
        Some(digits) => (true, digits),
        None => (false, value),
    };
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("hta/value-malformed: invalid big integer".into());
    }
    let digits = digits.trim_start_matches('0');
    let digits = if digits.is_empty() { "0" } else { digits };
    let canonical = if negative && digits != "0" {
        format!("-{digits}")
    } else {
        digits.to_owned()
    };
    match canonical.parse::<i64>() {
        Ok(value) => Ok(Value::Integer(value)),
        Err(_) => Ok(Value::BigInteger(canonical)),
    }
}

pub(crate) fn validate_canonical_big_integer(value: &str) -> Result<(), String> {
    match canonical_big_integer(value)? {
        Value::BigInteger(canonical) if canonical == value => Ok(()),
        Value::BigInteger(_) => {
            Err("hta/value-noncanonical: big integer text is not canonical".into())
        }
        Value::Integer(_) => {
            Err("hta/value-noncanonical: signed 64-bit integers use the i64 tag".into())
        }
        _ => unreachable!("canonical big integer returned a non-integer value"),
    }
}

fn abi_to_immutable(value: &AbiValue) -> Result<Value, String> {
    Ok(match value {
        AbiValue::Nil => Value::Nil,
        AbiValue::Boolean(value) => Value::Boolean(*value),
        AbiValue::String(value) => Value::String(value.clone()),
        AbiValue::Integer(value) => Value::Integer(*value),
        AbiValue::BigInteger(value) => canonical_big_integer(value)?,
        AbiValue::Float(value) => {
            if !value.is_finite() {
                return Err("hta/non-finite number".into());
            }
            Value::Float(*value)
        }
        AbiValue::Bytes(value) => Value::Bytes(value.clone()),
        AbiValue::Keyword(value) => Value::Keyword(value.clone()),
        AbiValue::Vector(values) => Value::Vector(
            values
                .iter()
                .map(abi_to_immutable)
                .collect::<Result<Vec<_>, _>>()?,
        ),
        AbiValue::Record(values) => Value::Record(
            values
                .iter()
                .map(|(key, value)| Ok((key.clone(), abi_to_immutable(value)?)))
                .collect::<Result<_, String>>()?,
        ),
    })
}

fn immutable_to_abi(value: Value) -> Result<AbiValue, String> {
    Ok(match value {
        Value::Nil => AbiValue::Nil,
        Value::Boolean(value) => AbiValue::Boolean(value),
        Value::String(value) => AbiValue::String(value),
        Value::Integer(value) => AbiValue::Integer(value),
        Value::BigInteger(value) => match canonical_big_integer(&value)? {
            Value::Integer(value) => AbiValue::Integer(value),
            Value::BigInteger(value) => AbiValue::BigInteger(value),
            _ => unreachable!("canonical big integer returned a non-integer value"),
        },
        Value::Float(value) => {
            if !value.is_finite() {
                return Err("hta/non-finite number".into());
            }
            AbiValue::Float(value)
        }
        Value::Bytes(value) => AbiValue::Bytes(value),
        Value::Keyword(value) => AbiValue::Keyword(value),
        Value::Vector(values) => AbiValue::Vector(
            values
                .into_iter()
                .map(immutable_to_abi)
                .collect::<Result<Vec<_>, _>>()?,
        ),
        Value::Record(values) => AbiValue::Record(
            values
                .into_iter()
                .map(|(key, value)| Ok((key, immutable_to_abi(value)?)))
                .collect::<Result<_, String>>()?,
        ),
        value => {
            return Err(format!(
                "hta/value-unsupported: immutable value is outside the provider ABI: {value:?}"
            ))
        }
    })
}

fn encode_value(value: &Value, depth: usize, output: &mut Vec<u8>) -> Result<(), String> {
    if depth > MAX_NESTING_DEPTH {
        return Err("hta/value-too-deep".into());
    }
    match value {
        Value::Nil => push(output, NIL),
        Value::Boolean(false) => push(output, FALSE),
        Value::Boolean(true) => push(output, TRUE),
        Value::Integer(value) => {
            push(output, I64)?;
            write(output, &value.to_be_bytes())
        }
        Value::Float(value) => {
            if !value.is_finite() {
                return Err("hta/non-finite number".into());
            }
            push(output, F64)?;
            write(output, &value.to_bits().to_be_bytes())
        }
        Value::Character(value) => {
            push(output, CHARACTER)?;
            write(output, &u32::from(*value).to_be_bytes())
        }
        Value::BigInteger(value) => match canonical_big_integer(value)? {
            Value::Integer(value) => {
                push(output, I64)?;
                write(output, &value.to_be_bytes())
            }
            Value::BigInteger(value) => write_sized(output, BIG_INTEGER, value.as_bytes()),
            _ => unreachable!("canonical big integer returned a non-integer value"),
        },
        Value::String(value) => write_sized(output, STRING, value.as_bytes()),
        Value::Bytes(value) => write_sized(output, BYTES, value),
        Value::Keyword(value) => write_sized(output, KEYWORD, value.as_bytes()),
        Value::Regex(value) => write_sized(output, REGEX, value.as_bytes()),
        Value::Symbol(value) => write_sized(output, SYMBOL, value.as_bytes()),
        Value::List(values) => encode_sequence(LIST, values, depth, output),
        Value::Vector(values) => encode_sequence(VECTOR, values, depth, output),
        Value::MapEntry(values) => encode_map_entry(values, depth, output),
        Value::Tuple(values) => encode_sequence(VECTOR, values, depth, output),
        Value::Cons(values) => encode_sequence(CONS, values, depth, output),
        Value::Queue(values) => encode_sequence(QUEUE, values, depth, output),
        Value::Set(values) => encode_unordered_sequence(SET, values, depth, output),
        Value::OrderedSet(values) => encode_sequence(ORDERED_SET, values, depth, output),
        Value::SortedSet(values) => encode_unordered_sequence(SORTED_SET, values, depth, output),
        Value::Map(values) => encode_map(MAP, values, depth, output, true),
        Value::OrderedMap(values) => encode_map(ORDERED_MAP, values, depth, output, false),
        Value::SortedMap(values) => encode_map(SORTED_MAP, values, depth, output, false),
        Value::Trie(values) => {
            let entries = values
                .iter()
                .map(|(key, value)| (Value::String(key.clone()), value.clone()))
                .collect::<Vec<_>>();
            encode_map(TRIE, &entries, depth, output, false)
        }
        Value::Record(values) => encode_record(values, depth, output),
        Value::Tagged { tag, form } => {
            push(output, TAGGED)?;
            encode_value(&Value::Symbol(tag.clone()), depth + 1, output)?;
            encode_value(form, depth + 1, output)
        }
        Value::ExceptionInfo {
            message,
            data,
            cause,
            provenance,
        } => {
            if !matches!(data.as_ref(), Value::Map(_) | Value::Record(_)) {
                return Err("hta/value-invalid: exception data must be a map".into());
            }
            if cause
                .as_ref()
                .is_some_and(|cause| !matches!(cause.as_ref(), Value::ExceptionInfo { .. }))
            {
                return Err("hta/value-invalid: exception cause must be an Exception".into());
            }
            push(output, EXCEPTION_INFO)?;
            encode_value(&Value::String(message.clone()), depth + 1, output)?;
            encode_value(data, depth + 1, output)?;
            encode_value(cause.as_deref().unwrap_or(&Value::Nil), depth + 1, output)?;
            encode_value(&provenance_value(provenance), depth + 1, output)
        }
        Value::Struct {
            name,
            fields,
            values,
        } => {
            if fields.len() != values.len() {
                return Err("hta/value-invalid: struct arity mismatch".into());
            }
            push(output, STRUCT)?;
            encode_value(&Value::String(name.clone()), depth + 1, output)?;
            encode_value(
                &Value::Vector(fields.iter().cloned().map(Value::String).collect()),
                depth + 1,
                output,
            )?;
            encode_value(&Value::Vector(values.clone()), depth + 1, output)
        }
        Value::Pointer { context, fields } => {
            push(output, POINTER)?;
            encode_value(&Value::Keyword(context.clone()), depth + 1, output)?;
            encode_record(fields, depth + 1, output)
        }
        Value::VarRef(symbol) => {
            push(output, VAR_REF)?;
            encode_value(&Value::Symbol(symbol.clone()), depth + 1, output)
        }
    }
}

fn encode_sequence(
    tag: u8,
    values: &[Value],
    depth: usize,
    output: &mut Vec<u8>,
) -> Result<(), String> {
    push(output, tag)?;
    write_len(output, values.len())?;
    for value in values {
        encode_value(value, depth + 1, output)?;
    }
    Ok(())
}

fn encode_map_entry(values: &[Value], depth: usize, output: &mut Vec<u8>) -> Result<(), String> {
    if values.len() != 2 {
        return Err("hta/value-invalid: map entry must contain two values".into());
    }
    encode_sequence(MAP_ENTRY, values, depth, output)
}

fn encode_unordered_sequence(
    tag: u8,
    values: &[Value],
    depth: usize,
    output: &mut Vec<u8>,
) -> Result<(), String> {
    let mut encoded = Vec::with_capacity(values.len());
    for value in values {
        let mut bytes = Vec::new();
        encode_value(value, depth + 1, &mut bytes)?;
        encoded.push(bytes);
    }
    encoded.sort();
    encoded.dedup();
    push(output, tag)?;
    write_len(output, encoded.len())?;
    for value in encoded {
        write(output, &value)?;
    }
    Ok(())
}

fn encode_map(
    tag: u8,
    values: &[(Value, Value)],
    depth: usize,
    output: &mut Vec<u8>,
    canonical_order: bool,
) -> Result<(), String> {
    let mut entries = Vec::with_capacity(values.len());
    for (key, value) in values {
        let mut key_bytes = Vec::new();
        encode_value(key, depth + 1, &mut key_bytes)?;
        let mut value_bytes = Vec::new();
        encode_value(value, depth + 1, &mut value_bytes)?;
        entries.push((key_bytes, value_bytes));
    }
    if canonical_order {
        entries.sort_by(|left, right| left.0.cmp(&right.0));
    }
    for pair in entries.windows(2) {
        if pair[0].0 == pair[1].0 {
            return Err("hta/value-invalid: duplicate map key".into());
        }
    }
    push(output, tag)?;
    write_len(output, entries.len())?;
    for (key, value) in entries {
        write(output, &key)?;
        write(output, &value)?;
    }
    Ok(())
}

fn encode_record(
    values: &BTreeMap<String, Value>,
    depth: usize,
    output: &mut Vec<u8>,
) -> Result<(), String> {
    push(output, MAP)?;
    write_len(output, values.len())?;

    let mut entries = Vec::with_capacity(values.len());
    for (key, value) in values {
        let mut key_bytes = Vec::with_capacity(key.len() + 5);
        encode_value(&Value::Keyword(key.clone()), depth + 1, &mut key_bytes)?;
        let mut value_bytes = Vec::new();
        encode_value(value, depth + 1, &mut value_bytes)?;
        entries.push((key_bytes, value_bytes));
    }
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    for (key, value) in entries {
        write(output, &key)?;
        write(output, &value)?;
    }
    Ok(())
}

fn write_sized(output: &mut Vec<u8>, tag: u8, bytes: &[u8]) -> Result<(), String> {
    push(output, tag)?;
    write_len(output, bytes.len())?;
    write(output, bytes)
}

fn write_len(output: &mut Vec<u8>, len: usize) -> Result<(), String> {
    let len = u32::try_from(len)
        .map_err(|_| "hta/value-too-large: container or scalar length exceeds u32".to_string())?;
    write(output, &len.to_be_bytes())
}

fn push(output: &mut Vec<u8>, byte: u8) -> Result<(), String> {
    write(output, &[byte])
}

fn write(output: &mut Vec<u8>, bytes: &[u8]) -> Result<(), String> {
    let next = output
        .len()
        .checked_add(bytes.len())
        .ok_or_else(|| "hta/frame-too-large: length overflow".to_string())?;
    if next > MAX_FRAME_BYTES {
        return Err(format!(
            "hta/frame-too-large: encoded frame exceeds {} bytes",
            MAX_FRAME_BYTES
        ));
    }
    output.extend_from_slice(bytes);
    Ok(())
}

struct Reader<'a> {
    bytes: &'a [u8],
    cursor: usize,
}

impl Reader<'_> {
    fn value(&mut self, depth: usize) -> Result<Value, String> {
        if depth > MAX_NESTING_DEPTH {
            return Err("hta/value-too-deep".into());
        }
        match self.byte()? {
            NIL => Ok(Value::Nil),
            FALSE => Ok(Value::Boolean(false)),
            TRUE => Ok(Value::Boolean(true)),
            I64 => Ok(Value::Integer(i64::from_be_bytes(
                self.take(8)?.try_into().expect("eight bytes"),
            ))),
            F64 => {
                let value = f64::from_bits(u64::from_be_bytes(
                    self.take(8)?.try_into().expect("eight bytes"),
                ));
                if !value.is_finite() {
                    return Err("hta/non-finite number".into());
                }
                Ok(Value::Float(value))
            }
            CHARACTER => {
                let scalar = u32::from_be_bytes(self.take(4)?.try_into().expect("four bytes"));
                char::from_u32(scalar)
                    .map(Value::Character)
                    .ok_or_else(|| "hta/value-malformed: invalid character scalar".into())
            }
            STRING => Ok(Value::String(self.text()?)),
            BYTES => Ok(Value::Bytes(self.sized()?.to_vec())),
            KEYWORD => Ok(Value::Keyword(self.text()?)),
            SYMBOL => Ok(Value::Symbol(self.text()?)),
            BIG_INTEGER => canonical_big_integer(&self.text()?),
            REGEX => Ok(Value::Regex(self.text()?)),
            LIST => self.sequence(depth).map(Value::List),
            VECTOR => self.vector(depth),
            TUPLE => self.sequence(depth).map(Value::Tuple),
            MAP_ENTRY => {
                let values = self.sequence(depth)?;
                if values.len() != 2 {
                    return Err("hta/value-malformed: map entry must contain two values".into());
                }
                Ok(Value::MapEntry(values))
            }
            CONS => {
                let values = self.sequence(depth)?;
                if values.is_empty() {
                    Err("hta/value-malformed: empty cons".into())
                } else {
                    Ok(Value::Cons(values))
                }
            }
            QUEUE => self.sequence(depth).map(Value::Queue),
            SET => self.sequence(depth).map(Value::Set),
            ORDERED_SET => self.sequence(depth).map(Value::OrderedSet),
            SORTED_SET => self.sequence(depth).map(Value::SortedSet),
            MAP => self.map(depth, MAP),
            ORDERED_MAP => self.map(depth, ORDERED_MAP),
            SORTED_MAP => self.map(depth, SORTED_MAP),
            TRIE => self.trie(depth),
            TAGGED => {
                let Value::Symbol(tag) = self.value(depth + 1)? else {
                    return Err("hta/value-malformed: invalid tagged literal tag".into());
                };
                Ok(Value::Tagged {
                    tag,
                    form: Box::new(self.value(depth + 1)?),
                })
            }
            EXCEPTION_INFO => {
                let Value::String(message) = self.value(depth + 1)? else {
                    return Err("hta/value-malformed: invalid exception message".into());
                };
                let data = Box::new(self.value(depth + 1)?);
                let cause = match self.value(depth + 1)? {
                    Value::Nil => None,
                    value @ Value::ExceptionInfo { .. } => Some(Box::new(value)),
                    _ => return Err("hta/value-malformed: invalid exception cause".into()),
                };
                if !matches!(data.as_ref(), Value::Map(_) | Value::Record(_)) {
                    return Err("hta/value-malformed: invalid exception data".into());
                }
                let provenance = decode_provenance(self.value(depth + 1)?)?;
                Ok(Value::ExceptionInfo {
                    message,
                    data,
                    cause,
                    provenance,
                })
            }
            STRUCT => self.structure(depth),
            POINTER => {
                let Value::Keyword(context) = self.value(depth + 1)? else {
                    return Err("hta/value-malformed: invalid pointer context".into());
                };
                let Value::Record(fields) = self.value(depth + 1)? else {
                    return Err("hta/value-malformed: invalid pointer fields".into());
                };
                Ok(Value::Pointer { context, fields })
            }
            VAR_REF => {
                let Value::Symbol(symbol) = self.value(depth + 1)? else {
                    return Err("hta/value-malformed: invalid Var reference".into());
                };
                Ok(Value::VarRef(symbol))
            }
            tag @ (HANDLE | NAMESPACE | VAR | ATOM | ARRAY | OBJECT) => Err(format!(
                "hta/value-unsupported: runtime wire tag {tag} is not portable"
            )),
            tag => Err(format!("hta/value-malformed: unknown tag {tag}")),
        }
    }

    fn vector(&mut self, depth: usize) -> Result<Value, String> {
        self.sequence(depth).map(Value::Vector)
    }

    fn sequence(&mut self, depth: usize) -> Result<Vec<Value>, String> {
        let len = self.len()?;
        if len > self.remaining() {
            return Err("hta/value-malformed: impossible sequence length".into());
        }
        let mut values = Vec::with_capacity(len);
        for _ in 0..len {
            values.push(self.value(depth + 1)?);
        }
        Ok(values)
    }

    fn entries(&mut self, depth: usize) -> Result<Vec<(Value, Value)>, String> {
        let len = self.len()?;
        if len > self.remaining() / 2 {
            return Err("hta/value-malformed: impossible map length".into());
        }
        let mut values = Vec::with_capacity(len);
        for _ in 0..len {
            values.push((self.value(depth + 1)?, self.value(depth + 1)?));
        }
        Ok(values)
    }

    fn map(&mut self, depth: usize, tag: u8) -> Result<Value, String> {
        let entries = self.entries(depth)?;
        if tag == MAP
            && entries
                .iter()
                .all(|(key, _)| matches!(key, Value::Keyword(_)))
        {
            let mut record = BTreeMap::new();
            for (key, value) in entries {
                let Value::Keyword(key) = key else {
                    unreachable!()
                };
                if record.insert(key.clone(), value).is_some() {
                    return Err(format!(
                        "hta/value-malformed: duplicate portable record key :{key}"
                    ));
                }
            }
            return Ok(Value::Record(record));
        }
        match tag {
            MAP => Ok(Value::Map(entries)),
            ORDERED_MAP => Ok(Value::OrderedMap(entries)),
            SORTED_MAP => Ok(Value::SortedMap(entries)),
            _ => unreachable!(),
        }
    }

    fn trie(&mut self, depth: usize) -> Result<Value, String> {
        self.entries(depth)?
            .into_iter()
            .map(|(key, value)| match key {
                Value::String(key) => Ok((key, value)),
                _ => Err("hta/value-malformed: invalid trie key".into()),
            })
            .collect::<Result<Vec<_>, String>>()
            .map(Value::Trie)
    }

    fn structure(&mut self, depth: usize) -> Result<Value, String> {
        let Value::String(name) = self.value(depth + 1)? else {
            return Err("hta/value-malformed: invalid struct name".into());
        };
        let Value::Vector(field_values) = self.value(depth + 1)? else {
            return Err("hta/value-malformed: invalid struct fields".into());
        };
        let fields = field_values
            .into_iter()
            .map(|value| match value {
                Value::String(value) => Ok(value),
                _ => Err("hta/value-malformed: invalid struct field".into()),
            })
            .collect::<Result<Vec<_>, String>>()?;
        let Value::Vector(values) = self.value(depth + 1)? else {
            return Err("hta/value-malformed: invalid struct values".into());
        };
        if fields.len() != values.len() {
            return Err("hta/value-malformed: struct arity mismatch".into());
        }
        Ok(Value::Struct {
            name,
            fields,
            values,
        })
    }

    fn text(&mut self) -> Result<String, String> {
        String::from_utf8(self.sized()?.to_vec())
            .map_err(|_| "hta/value-malformed: invalid UTF-8".into())
    }

    fn sized(&mut self) -> Result<&[u8], String> {
        let len = self.len()?;
        self.take(len)
    }

    fn len(&mut self) -> Result<usize, String> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().expect("four bytes")) as usize)
    }

    fn byte(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }

    fn take(&mut self, size: usize) -> Result<&[u8], String> {
        let end = self
            .cursor
            .checked_add(size)
            .ok_or_else(|| "hta/value-malformed: length overflow".to_string())?;
        if end > self.bytes.len() {
            return Err("hta/value-malformed: truncated value".into());
        }
        let output = &self.bytes[self.cursor..end];
        self.cursor = end;
        Ok(output)
    }

    fn remaining(&self) -> usize {
        self.bytes.len() - self.cursor
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode(value: &Value) -> Result<Vec<u8>, String> {
        encode_immutable(value)
    }

    fn decode(bytes: &[u8]) -> Result<Value, String> {
        decode_immutable(bytes)
    }

    fn decode_canonical(bytes: &[u8], max_bytes: usize) -> Result<Value, String> {
        decode_immutable_canonical(bytes, max_bytes)
    }

    fn record(entries: impl IntoIterator<Item = (&'static str, Value)>) -> Value {
        Value::Record(
            entries
                .into_iter()
                .map(|(key, value)| (key.to_owned(), value))
                .collect(),
        )
    }

    #[test]
    fn canonical_portable_round_trip() {
        let value = record([
            ("a", Value::Vector(vec![Value::Boolean(true), Value::Nil])),
            ("b", Value::Integer(2)),
            ("big", Value::BigInteger("9223372036854775808".into())),
            ("bytes", Value::Bytes(vec![0, 1, 255])),
            ("float", Value::Float(0.28)),
            ("keyword", Value::Keyword("profile.primary".into())),
        ]);
        let encoded = encode(&value).unwrap();
        assert_eq!(decode(&encoded).unwrap(), value);
        assert_eq!(encode(&decode(&encoded).unwrap()).unwrap(), encoded);
        assert_eq!(decode_canonical(&encoded, encoded.len()).unwrap(), value);
    }

    #[test]
    fn immutable_hara_values_and_references_round_trip() {
        let values = vec![
            Value::Symbol("tool.lint/lint-source".into()),
            Value::Character('雪'),
            Value::BigInteger("123456789012345678901234567890".into()),
            Value::Regex("^[a-z]+$".into()),
            Value::List(vec![Value::Integer(1), Value::Integer(2)]),
            Value::MapEntry(vec![Value::Integer(1), Value::Integer(2)]),
            Value::Queue(vec![Value::Integer(1), Value::Integer(2)]),
            Value::Set(vec![Value::Keyword("a".into()), Value::Keyword("b".into())]),
            Value::Map(vec![(Value::String("answer".into()), Value::Integer(42))]),
            Value::Tagged {
                tag: "demo/value".into(),
                form: Box::new(Value::Integer(42)),
            },
            Value::Pointer {
                context: "kernel".into(),
                fields: [("id".into(), Value::String("ROOT".into()))]
                    .into_iter()
                    .collect(),
            },
            Value::VarRef("user/answer".into()),
        ];
        for value in values {
            assert_eq!(decode(&encode(&value).unwrap()).unwrap(), value);
        }
    }

    #[test]
    fn compact_tuples_use_the_vector_wire_identity_and_map_entries_are_distinct() {
        let tuple = Value::Tuple(vec![Value::Integer(1), Value::Integer(2)]);
        let encoded = encode(&tuple).unwrap();
        assert_eq!(encoded[4], VECTOR);
        assert_eq!(
            decode(&encoded).unwrap(),
            Value::Vector(vec![Value::Integer(1), Value::Integer(2)])
        );

        let mut legacy = encoded;
        legacy[4] = TUPLE;
        assert_eq!(decode(&legacy).unwrap(), tuple);

        let entry = Value::MapEntry(vec![Value::Keyword("key".into()), Value::Integer(42)]);
        let encoded_entry = encode(&entry).unwrap();
        assert_eq!(encoded_entry[4], MAP_ENTRY);
        assert_eq!(decode(&encoded_entry).unwrap(), entry);
    }

    #[test]
    fn records_are_canonical_independent_of_construction_order() {
        let first = record([("z", Value::Integer(1)), ("a", Value::Integer(2))]);
        let second = record([("a", Value::Integer(2)), ("z", Value::Integer(1))]);
        assert_eq!(encode(&first).unwrap(), encode(&second).unwrap());
    }

    #[test]
    fn canonical_decode_rejects_noncanonical_map_order() {
        let mut bytes = MAGIC.to_vec();
        bytes.extend_from_slice(&[MAP, 0, 0, 0, 2]);
        bytes.extend_from_slice(&[KEYWORD, 0, 0, 0, 2, b'a', b'a', NIL]);
        bytes.extend_from_slice(&[KEYWORD, 0, 0, 0, 1, b'z', TRUE]);

        assert!(decode(&bytes).is_ok());
        assert!(decode_canonical(&bytes, bytes.len())
            .unwrap_err()
            .contains("frame-noncanonical"));
    }

    #[test]
    fn canonical_decode_enforces_the_requested_maximum() {
        let bytes = encode(&Value::Nil).unwrap();
        assert_eq!(decode_canonical(&bytes, bytes.len()).unwrap(), Value::Nil);
        assert!(decode_canonical(&bytes, bytes.len() - 1)
            .unwrap_err()
            .contains("requested maximum"));
        assert!(decode_canonical(&bytes, 0)
            .unwrap_err()
            .contains("maximum-invalid"));
        assert!(decode_canonical(&bytes, MAX_FRAME_BYTES + 1)
            .unwrap_err()
            .contains("maximum-invalid"));
    }

    #[test]
    fn floats_preserve_ieee_754_bits() {
        for value in [0.28, -0.0] {
            let decoded = decode(&encode(&Value::Float(value)).unwrap()).unwrap();
            let Value::Float(decoded) = decoded else {
                panic!("float value")
            };
            assert_eq!(decoded.to_bits(), value.to_bits());
        }
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(encode(&Value::Float(value)).is_err());
        }
    }

    #[test]
    fn general_maps_accept_non_keyword_keys_and_records_reject_duplicates() {
        let mut non_keyword = MAGIC.to_vec();
        non_keyword.extend_from_slice(&[MAP, 0, 0, 0, 1]);
        non_keyword.extend_from_slice(&[STRING, 0, 0, 0, 1, b'a', NIL]);
        assert_eq!(
            decode(&non_keyword).unwrap(),
            Value::Map(vec![(Value::String("a".into()), Value::Nil)])
        );

        let mut duplicate = MAGIC.to_vec();
        duplicate.extend_from_slice(&[MAP, 0, 0, 0, 2]);
        for value in [NIL, TRUE] {
            duplicate.extend_from_slice(&[KEYWORD, 0, 0, 0, 1, b'a', value]);
        }
        assert!(decode(&duplicate)
            .unwrap_err()
            .contains("duplicate portable record key :a"));
    }

    #[test]
    fn runtime_only_tags_fail_closed() {
        for tag in [HANDLE, NAMESPACE, VAR, ATOM, ARRAY, OBJECT] {
            let bytes = [MAGIC.as_slice(), &[tag]].concat();
            assert!(decode(&bytes).unwrap_err().contains("runtime wire tag"));
            assert!(decode_canonical(&bytes, bytes.len())
                .unwrap_err()
                .contains("runtime wire tag"));
        }
    }

    #[test]
    fn retired_decimal_tag_fails_closed() {
        let bytes = [MAGIC.as_slice(), &[21]].concat();
        assert!(decode(&bytes).unwrap_err().contains("unknown tag 21"));
        assert!(decode_canonical(&bytes, bytes.len())
            .unwrap_err()
            .contains("unknown tag 21"));
    }

    #[test]
    fn frame_shape_and_lengths_are_bounded() {
        assert!(decode(b"not-hta")
            .unwrap_err()
            .contains("expected HTA0 magic"));

        let trailing = [MAGIC.as_slice(), &[NIL, NIL]].concat();
        assert!(decode(&trailing).unwrap_err().contains("trailing bytes"));

        let mut impossible = MAGIC.to_vec();
        impossible.extend_from_slice(&[VECTOR, 0xff, 0xff, 0xff, 0xff]);
        assert!(decode(&impossible)
            .unwrap_err()
            .contains("impossible sequence length"));
    }

    #[test]
    fn nesting_depth_is_bounded_on_encode_and_decode() {
        let mut value = Value::Nil;
        for _ in 0..=MAX_NESTING_DEPTH {
            value = Value::Vector(vec![value]);
        }
        assert!(encode(&value).unwrap_err().contains("value-too-deep"));

        let mut bytes = MAGIC.to_vec();
        for _ in 0..=MAX_NESTING_DEPTH {
            bytes.extend_from_slice(&[VECTOR, 0, 0, 0, 1]);
        }
        bytes.push(NIL);
        assert!(decode(&bytes).unwrap_err().contains("value-too-deep"));
    }
}
