use crate::core::{ResultValue, Value};
use crate::lang::data::MapEntry as PMapEntry;
#[cfg(test)]
use crate::lang::data::{Tuple as PTuple, Vector as PVector};
use crate::lang::protocol::INamespaced;
use num_bigint::BigInt;
use num_traits::ToPrimitive;

const MAGIC: &[u8; 4] = b"HTA0";
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
const DEQUE: u8 = 36;
const PRIORITY_MAP: u8 = 37;
const MAP_ENTRY: u8 = 38;
const RESULT_STRUCT_NAME: &str = "std.native/Result";
const RESULT_STRUCT_FIELDS: [&str; 4] = ["status", "data", "error", "context"];

/// The canonical HTA0 tag inventory. Every host codec must preserve these
/// numeric assignments; omitted values are reserved for future revisions.
pub const HTA0_TAG_INVENTORY: &[(u8, &str)] = &[
    (NIL, "nil"),
    (FALSE, "false"),
    (TRUE, "true"),
    (I64, "i64"),
    (STRING, "string"),
    (BYTES, "bytes"),
    (KEYWORD, "keyword"),
    (SYMBOL, "symbol"),
    (LIST, "list"),
    (VECTOR, "vector"),
    (SET, "set"),
    (MAP, "map"),
    (HANDLE, "handle"),
    (NAMESPACE, "namespace"),
    (VAR, "legacy-var"),
    (F64, "f64"),
    (ATOM, "atom"),
    (ARRAY, "array"),
    (OBJECT, "object"),
    (CHARACTER, "character"),
    (BIG_INTEGER, "big-integer"),
    (REGEX, "regex"),
    (TUPLE, "tuple"),
    (CONS, "cons"),
    (QUEUE, "queue"),
    (ORDERED_MAP, "ordered-map"),
    (SORTED_MAP, "sorted-map"),
    (TRIE, "trie"),
    (ORDERED_SET, "ordered-set"),
    (SORTED_SET, "sorted-set"),
    (TAGGED, "tagged"),
    (EXCEPTION_INFO, "exception-info"),
    (STRUCT, "struct"),
    (POINTER, "pointer"),
    (VAR_REF, "var-ref"),
    (DEQUE, "deque"),
    (PRIORITY_MAP, "priority-map"),
    (MAP_ENTRY, "map-entry"),
];

fn decode_exception_provenance(
    value: Value,
) -> Result<
    (
        Option<crate::core::ExceptionSite>,
        Vec<crate::core::ExceptionSite>,
    ),
    String,
> {
    let entries = crate::core::map_entries(&value)
        .ok_or_else(|| "hta/value-malformed: invalid exception provenance".to_string())?;
    if entries.len() != 2 {
        return Err("hta/value-malformed: invalid exception provenance fields".into());
    }
    let created = entries
        .iter()
        .find_map(|(key, value)| field_name(key, "ex/created-at").then_some(value))
        .ok_or_else(|| "hta/value-malformed: missing exception creation provenance".to_string())?;
    let throws = entries
        .iter()
        .find_map(|(key, value)| field_name(key, "ex/throws").then_some(value))
        .ok_or_else(|| "hta/value-malformed: missing exception throw provenance".to_string())?;
    let created = match created {
        Value::Nil => None,
        value => Some(decode_exception_site(value)?),
    };
    let Value::Vector(throws) = throws else {
        return Err("hta/value-malformed: invalid exception throws provenance".into());
    };
    let throws = throws
        .iter()
        .map(decode_exception_site)
        .collect::<Result<Vec<_>, _>>()?;
    Ok((created, throws))
}

fn decode_exception_site(value: &Value) -> Result<crate::core::ExceptionSite, String> {
    let entries = crate::core::map_entries(value)
        .ok_or_else(|| "hta/value-malformed: invalid exception provenance site".to_string())?;
    if entries.len() != 4 {
        return Err("hta/value-malformed: invalid exception provenance site".into());
    }
    let get = |name: &str| {
        entries
            .iter()
            .find_map(|(key, value)| field_name(key, name).then_some(value))
    };
    let namespace = match get("namespace") {
        Some(Value::Nil) => None,
        Some(Value::String(value)) => Some(value.clone()),
        _ => return Err("hta/value-malformed: invalid exception provenance namespace".into()),
    };
    let resource = match get("resource") {
        Some(Value::Nil) => None,
        Some(Value::String(value)) => Some(value.clone()),
        _ => return Err("hta/value-malformed: invalid exception provenance resource".into()),
    };
    let line = match get("line") {
        Some(Value::Number(value)) if *value >= 0 => *value as usize,
        _ => return Err("hta/value-malformed: invalid exception provenance line".into()),
    };
    let column = match get("column") {
        Some(Value::Number(value)) if *value >= 0 => *value as usize,
        _ => return Err("hta/value-malformed: invalid exception provenance column".into()),
    };
    Ok(crate::core::ExceptionSite {
        namespace,
        resource,
        line,
        column,
    })
}

fn field_name(value: &Value, expected: &str) -> bool {
    match value {
        Value::Keyword(value) => value.as_str() == expected,
        Value::String(value) => value == expected,
        _ => false,
    }
}

pub fn encode(value: &Value) -> Result<Vec<u8>, String> {
    let mut output = MAGIC.to_vec();
    encode_bare(value, &mut output, 0)?;
    if output.len() > MAX_FRAME_BYTES {
        return Err("hta/value-too-large: frame exceeds 64 MiB".into());
    }
    Ok(output)
}

pub fn decode(bytes: &[u8]) -> Result<Value, String> {
    if bytes.len() > MAX_FRAME_BYTES {
        return Err("hta/value-too-large: frame exceeds 64 MiB".into());
    }
    if !bytes.starts_with(MAGIC) {
        return Err("hta/value-malformed: invalid HTA0 header".into());
    }
    let mut reader = Reader {
        bytes,
        cursor: MAGIC.len(),
    };
    let value = reader.value(0)?;
    if reader.cursor != bytes.len() {
        return Err("hta/value-malformed: trailing bytes".into());
    }
    Ok(value)
}

/// Decodes an HTA0 frame and verifies that its bytes are canonical.
///
/// The ordinary decoder is intentionally permissive for trusted legacy state;
/// transport and artifact boundaries should use this entry point instead.
pub fn decode_canonical(bytes: &[u8]) -> Result<Value, String> {
    let value = decode(bytes)?;
    let canonical = encode(&value)?;
    if canonical != bytes {
        return Err("hta/value-noncanonical: frame bytes are not canonical".into());
    }
    Ok(value)
}

fn encode_bare(value: &Value, output: &mut Vec<u8>, depth: usize) -> Result<(), String> {
    if depth > MAX_NESTING_DEPTH {
        return Err("hta/value-too-deep: nesting exceeds 256".into());
    }
    match value {
        Value::Nil => output.push(NIL),
        Value::Bool(false) => output.push(FALSE),
        Value::Bool(true) => output.push(TRUE),
        Value::Number(value) => {
            output.push(I64);
            output.extend_from_slice(&value.to_be_bytes());
        }
        Value::Float(value) => {
            if !value.is_finite() {
                return Err("hta/non-finite number".into());
            }
            output.push(F64);
            output.extend_from_slice(&value.to_bits().to_be_bytes());
        }
        Value::Character(value) => {
            output.push(CHARACTER);
            output.extend_from_slice(&u32::from(*value).to_be_bytes());
        }
        Value::BigInteger(value) => {
            if let Some(value) = value.to_i64() {
                output.push(I64);
                output.extend_from_slice(&value.to_be_bytes());
            } else {
                output.push(BIG_INTEGER);
                encode_bytes(value.to_string().as_bytes(), output)?;
            }
        }
        Value::Regex(value) => {
            output.push(REGEX);
            encode_bytes(value.as_bytes(), output)?;
        }
        Value::String(value) => {
            output.push(STRING);
            encode_bytes(value.as_str().as_bytes(), output)?;
        }
        Value::Bytes(value) => {
            output.push(BYTES);
            encode_bytes(value, output)?;
        }
        Value::ByteBuffer(value) => {
            output.push(BYTES);
            encode_bytes(&value.borrow(), output)?;
        }
        Value::Keyword(value) => {
            output.push(KEYWORD);
            encode_bytes(value.as_str().as_bytes(), output)?;
        }
        Value::Symbol(value) => {
            output.push(SYMBOL);
            encode_bytes(value.as_str().as_bytes(), output)?;
        }
        Value::List(values) => encode_sequence(LIST, values.iter(), output, depth)?,
        Value::Tuple(values) => encode_sequence(VECTOR, values.iter(), output, depth)?,
        Value::MapEntry(entry) => encode_sequence(MAP_ENTRY, entry.iter(), output, depth)?,
        Value::Vector(values) => encode_sequence(VECTOR, values.iter(), output, depth)?,
        Value::Cons(values) => encode_sequence(
            CONS,
            values.iter().collect::<Vec<_>>().iter(),
            output,
            depth,
        )?,
        Value::Queue(values) => encode_sequence(QUEUE, values.iter(), output, depth)?,
        Value::Deque(values) => encode_sequence(DEQUE, values.iter(), output, depth)?,
        Value::Set(values) => {
            let mut encoded = values
                .iter()
                .map(|value| bare(value, depth + 1))
                .collect::<Result<Vec<_>, _>>()?;
            encoded.sort();
            output.push(SET);
            encode_len(encoded.len(), output)?;
            for value in encoded {
                output.extend_from_slice(&value);
            }
        }
        Value::OrderedSet(values) => encode_sequence(ORDERED_SET, values.iter(), output, depth)?,
        Value::SortedSet(values) => {
            let mut encoded = values
                .iter()
                .map(|value| bare(value, depth + 1))
                .collect::<Result<Vec<_>, _>>()?;
            encoded.sort();
            output.push(SORTED_SET);
            encode_len(encoded.len(), output)?;
            for value in encoded {
                output.extend_from_slice(&value);
            }
        }
        Value::OrderedMap(values) => encode_map(
            ORDERED_MAP,
            values.iter().map(|pair| (&pair.0, &pair.1)),
            output,
            depth,
        )?,
        Value::SortedMap(values) => encode_map(SORTED_MAP, values.iter(), output, depth)?,
        Value::PriorityMap(values) => {
            let entries = values.iter().collect::<Vec<_>>();
            encode_map(
                PRIORITY_MAP,
                entries.iter().map(|pair| (&pair.0, &pair.1)),
                output,
                depth,
            )?;
        }
        Value::Trie(values) => {
            let entries = values
                .iter()
                .map(|key| {
                    (
                        Value::String(key.clone()),
                        values.get(&key).unwrap().clone(),
                    )
                })
                .collect::<Vec<_>>();
            encode_map(
                TRIE,
                entries.iter().map(|pair| (&pair.0, &pair.1)),
                output,
                depth,
            )?;
        }
        Value::Map(values) => {
            let mut encoded = values
                .iter()
                .map(|(key, value)| Ok((bare(key, depth + 1)?, bare(value, depth + 1)?)))
                .collect::<Result<Vec<_>, String>>()?;
            encoded.sort_by(|left, right| left.0.cmp(&right.0));
            output.push(MAP);
            encode_len(encoded.len(), output)?;
            for (key, value) in encoded {
                output.extend_from_slice(&key);
                output.extend_from_slice(&value);
            }
        }
        Value::Namespace(value) => {
            output.push(NAMESPACE);
            encode_bytes(value.name().as_str().as_bytes(), output)?;
        }
        Value::Var(value) => {
            output.push(VAR_REF);
            encode_bare(&Value::Symbol(value.symbol().clone()), output, depth + 1)?;
        }
        Value::Atom(value) => {
            output.push(ATOM);
            encode_bare(&value.deref_value(), output, depth + 1)?;
        }
        Value::Array(values) => encode_sequence(ARRAY, values.borrow().iter(), output, depth)?,
        Value::Object(values) => {
            let values = values.borrow();
            output.push(OBJECT);
            encode_len(values.len(), output)?;
            for (key, value) in values.iter() {
                encode_bare(&Value::String(key.clone()), output, depth + 1)?;
                encode_bare(value, output, depth + 1)?;
            }
        }
        Value::Extension(value) => {
            output.push(HANDLE);
            encode_bytes(value.provider.as_bytes(), output)?;
            encode_bytes(value.type_name.as_bytes(), output)?;
            output.extend_from_slice(&value.handle.to_be_bytes());
        }
        Value::Tagged(value) => {
            output.push(TAGGED);
            encode_bare(&Value::Symbol(value.tag().clone()), output, depth + 1)?;
            encode_bare(value.form(), output, depth + 1)?;
        }
        Value::ExceptionInfo(value) => {
            if crate::core::map_entries(&value.data).is_none() {
                return Err("hta/value-invalid: exception data must be a map".into());
            }
            if value
                .cause
                .as_deref()
                .is_some_and(|cause| !matches!(cause, Value::ExceptionInfo(_)))
            {
                return Err("hta/value-invalid: exception cause must be an Exception".into());
            }
            output.push(EXCEPTION_INFO);
            encode_bare(&Value::String(value.message.clone()), output, depth + 1)?;
            encode_bare(&value.data, output, depth + 1)?;
            encode_bare(
                value.cause.as_deref().unwrap_or(&Value::Nil),
                output,
                depth + 1,
            )?;
            encode_bare(
                &crate::core::exception_provenance_value(value),
                output,
                depth + 1,
            )?;
        }
        Value::Result(value) => {
            output.push(STRUCT);
            encode_bare(&Value::String(RESULT_STRUCT_NAME.into()), output, depth + 1)?;
            let fields = RESULT_STRUCT_FIELDS
                .iter()
                .map(|field| Value::String((*field).into()))
                .collect::<Vec<_>>();
            encode_sequence(VECTOR, fields.iter(), output, depth)?;
            let values = [
                value.status_value(),
                value.data.clone(),
                value.error_value(),
                value.transport_context(),
            ];
            encode_sequence(VECTOR, values.iter(), output, depth)?;
        }
        Value::Struct(value) => {
            output.push(STRUCT);
            encode_bare(&Value::String(value.ty.name.clone()), output, depth + 1)?;
            let fields = value
                .ty
                .fields
                .iter()
                .cloned()
                .map(Value::String)
                .collect::<Vec<_>>();
            encode_sequence(VECTOR, fields.iter(), output, depth)?;
            let values = value.ordered_values();
            encode_sequence(VECTOR, values.into_iter(), output, depth)?;
        }
        Value::Pointer(value) => {
            output.push(POINTER);
            encode_bare(&Value::Keyword(value.context().clone()), output, depth + 1)?;
            encode_bare(&Value::Map(value.fields().clone()), output, depth + 1)?;
        }
        Value::Mutable(_) | Value::MutableType(_) => {
            return Err(
                "hta/value-unsupported: mutable values are not serializable; use (into {} value)"
                    .into(),
            )
        }
        _ => return Err(format!("hta/value-unsupported: {}", value.display())),
    }
    Ok(())
}

fn encode_map<'a>(
    tag: u8,
    values: impl Iterator<Item = (&'a Value, &'a Value)>,
    output: &mut Vec<u8>,
    depth: usize,
) -> Result<(), String> {
    let values = values.collect::<Vec<_>>();
    output.push(tag);
    encode_len(values.len(), output)?;
    for (key, value) in values {
        encode_bare(key, output, depth + 1)?;
        encode_bare(value, output, depth + 1)?;
    }
    Ok(())
}

fn bare(value: &Value, depth: usize) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    encode_bare(value, &mut output, depth)?;
    Ok(output)
}

fn encode_sequence<'a>(
    tag: u8,
    values: impl Iterator<Item = &'a Value>,
    output: &mut Vec<u8>,
    depth: usize,
) -> Result<(), String> {
    let values = values.collect::<Vec<_>>();
    output.push(tag);
    encode_len(values.len(), output)?;
    for value in values {
        encode_bare(value, output, depth + 1)?;
    }
    Ok(())
}

fn encode_bytes(value: &[u8], output: &mut Vec<u8>) -> Result<(), String> {
    encode_len(value.len(), output)?;
    output.extend_from_slice(value);
    Ok(())
}
fn encode_len(value: usize, output: &mut Vec<u8>) -> Result<(), String> {
    let value = u32::try_from(value).map_err(|_| "hta/value-too-large")?;
    output.extend_from_slice(&value.to_be_bytes());
    Ok(())
}

fn decode_result_struct(
    name: &str,
    fields: &[String],
    values: &[Value],
) -> Result<Option<Value>, String> {
    let exact_fields = fields.len() == RESULT_STRUCT_FIELDS.len()
        && fields
            .iter()
            .zip(RESULT_STRUCT_FIELDS.iter())
            .all(|(field, expected)| field == expected);
    if name != RESULT_STRUCT_NAME || !exact_fields {
        return Ok(None);
    }
    let [status, data, error, context] = values else {
        return Err("hta/value-malformed: Result arity mismatch".into());
    };
    let result = match status {
        Value::Keyword(status) if status.as_str() == "success" => {
            if !matches!(error, Value::Nil) {
                return Err("hta/value-malformed: success Result contains an error".into());
            }
            ResultValue::success(data.clone(), context.clone())
        }
        Value::Keyword(status) if status.as_str() == "error" => {
            if !matches!(data, Value::Nil) {
                return Err("hta/value-malformed: error Result contains success data".into());
            }
            if !matches!(error, Value::ExceptionInfo(_)) {
                return Err("hta/value-malformed: error Result lacks a native Error".into());
            }
            ResultValue::error(error.clone(), context.clone())
        }
        _ => return Err("hta/value-malformed: invalid Result status".into()),
    }
    .map_err(|error| format!("hta/value-malformed: invalid Result: {error}"))?;
    Ok(Some(Value::Result(std::rc::Rc::new(result))))
}

struct Reader<'a> {
    bytes: &'a [u8],
    cursor: usize,
}
impl Reader<'_> {
    fn value(&mut self, depth: usize) -> Result<Value, String> {
        if depth > MAX_NESTING_DEPTH {
            return Err("hta/value-too-deep: nesting exceeds 256".into());
        }
        let tag = self.byte()?;
        match tag {
            NIL => Ok(Value::Nil),
            FALSE => Ok(Value::Bool(false)),
            TRUE => Ok(Value::Bool(true)),
            I64 => {
                let bytes = self.take(8)?;
                Ok(Value::Number(i64::from_be_bytes(bytes.try_into().unwrap())))
            }
            F64 => {
                let bytes = self.take(8)?;
                let value = f64::from_bits(u64::from_be_bytes(bytes.try_into().unwrap()));
                if !value.is_finite() {
                    return Err("hta/non-finite number".into());
                }
                Ok(Value::Float(value))
            }
            CHARACTER => {
                let codepoint = u32::from_be_bytes(self.take(4)?.try_into().unwrap());
                char::from_u32(codepoint)
                    .map(Value::Character)
                    .ok_or_else(|| "hta/value-malformed: invalid character scalar".into())
            }
            BIG_INTEGER => {
                let text = String::from_utf8(self.data()?.to_vec())
                    .map_err(|_| "hta/value-malformed: invalid big integer")?;
                let value = BigInt::parse_bytes(text.as_bytes(), 10)
                    .ok_or_else(|| "hta/value-malformed: invalid big integer".to_string())?;
                Ok(crate::numeric::compact_integer(value))
            }
            REGEX => Ok(Value::Regex(
                String::from_utf8(self.data()?.to_vec())
                    .map_err(|_| "hta/value-malformed: invalid regex")?,
            )),
            STRING => Ok(Value::String(
                String::from_utf8(self.data()?.to_vec())
                    .map_err(|_| "hta/value-malformed: invalid UTF-8")?,
            )),
            BYTES => Ok(Value::Bytes(self.data()?.to_vec())),
            KEYWORD => Ok(Value::Keyword(
                String::from_utf8(self.data()?.to_vec())
                    .map_err(|_| "hta/value-malformed: invalid UTF-8")?
                    .into(),
            )),
            SYMBOL => Ok(Value::Symbol(
                String::from_utf8(self.data()?.to_vec())
                    .map_err(|_| "hta/value-malformed: invalid UTF-8")?
                    .into(),
            )),
            LIST => Ok(Value::List(self.sequence(depth)?.into())),
            TUPLE => Ok(Value::Tuple(Box::new(
                crate::lang::data::Tuple::from_values(self.sequence(depth)?)?,
            ))),
            MAP_ENTRY => {
                let values = self.sequence(depth)?;
                let [key, value] = values.as_slice() else {
                    return Err("hta/value-malformed: map entry must contain two values".into());
                };
                Ok(Value::MapEntry(Box::new(PMapEntry::new(
                    key.clone(),
                    value.clone(),
                ))))
            }
            VECTOR => Ok(Value::Vector(self.sequence(depth)?.into())),
            CONS => {
                let mut values = self.sequence(depth)?;
                if values.is_empty() {
                    return Err("hta/value-malformed: empty cons".into());
                }
                let first = values.remove(0);
                Ok(Value::Cons(Box::new(crate::lang::data::Cons::new(
                    first,
                    values.into_iter().collect(),
                ))))
            }
            QUEUE => Ok(Value::Queue(Box::new(
                self.sequence(depth)?.into_iter().collect(),
            ))),
            DEQUE => Ok(Value::Deque(Box::new(
                self.sequence(depth)?.into_iter().collect(),
            ))),
            SET => Ok(Value::Set(self.sequence(depth)?.into())),
            ORDERED_SET => Ok(Value::OrderedSet(Box::new(
                self.sequence(depth)?.into_iter().collect(),
            ))),
            SORTED_SET => Ok(Value::SortedSet(Box::new(
                self.sequence(depth)?.into_iter().collect(),
            ))),
            MAP => {
                let size = self.len()?;
                if size > self.bytes.len().saturating_sub(self.cursor) / 2 {
                    return Err("hta/value-malformed: impossible map length".into());
                }
                let mut values = Vec::with_capacity(size);
                for _ in 0..size {
                    values.push((self.value(depth + 1)?, self.value(depth + 1)?));
                }
                Ok(Value::Map(values.into_iter().collect()))
            }
            ORDERED_MAP => Ok(Value::OrderedMap(Box::new(
                self.entries(depth)?.into_iter().collect(),
            ))),
            SORTED_MAP => Ok(Value::SortedMap(Box::new(
                self.entries(depth)?.into_iter().collect(),
            ))),
            PRIORITY_MAP => Ok(Value::PriorityMap(Box::new(
                self.entries(depth)?.into_iter().collect(),
            ))),
            TRIE => {
                let mut trie = crate::lang::data::Trie::new();
                for (key, value) in self.entries(depth)? {
                    let Value::String(key) = key else {
                        return Err("hta/value-malformed: invalid trie key".into());
                    };
                    trie = trie.assoc_value(key, value);
                }
                Ok(Value::Trie(Box::new(trie)))
            }
            NAMESPACE => {
                let name = String::from_utf8(self.data()?.to_vec())
                    .map_err(|_| "hta/value-malformed: invalid namespace name")?;
                Ok(Value::Namespace(std::rc::Rc::new(
                    crate::kernel::Namespace::new(name),
                )))
            }
            VAR => Err("hta/value-malformed: legacy var tag is not supported; use var-ref".into()),
            VAR_REF => {
                let symbol = match self.value(depth + 1)? {
                    Value::Symbol(symbol) if symbol.get_namespace().is_some() => symbol,
                    _ => return Err("hta/value-malformed: invalid Var reference".into()),
                };
                Ok(Value::Var(crate::kernel::Var::new(
                    symbol.as_str(),
                    Value::Nil,
                )))
            }
            ATOM => Ok(Value::Atom(Box::new(crate::core::RuntimeAtom::new(
                self.value(depth + 1)?,
                true,
            )))),
            ARRAY => Ok(Value::Array(std::rc::Rc::new(std::cell::RefCell::new(
                self.sequence(depth)?,
            )))),
            OBJECT => {
                let size = self.len()?;
                if size > self.bytes.len().saturating_sub(self.cursor) / 2 {
                    return Err("hta/value-malformed: impossible object length".into());
                }
                let mut values = Vec::with_capacity(size);
                for _ in 0..size {
                    let Value::String(key) = self.value(depth + 1)? else {
                        return Err("hta/value-malformed: invalid object key".into());
                    };
                    values.push((key, self.value(depth + 1)?));
                }
                Ok(Value::Object(std::rc::Rc::new(std::cell::RefCell::new(
                    values,
                ))))
            }
            HANDLE => {
                let provider = String::from_utf8(self.data()?.to_vec())
                    .map_err(|_| "hta/value-malformed: invalid handle owner")?;
                let type_name = String::from_utf8(self.data()?.to_vec())
                    .map_err(|_| "hta/value-malformed: invalid handle type")?;
                let bytes = self.take(8)?;
                Ok(Value::Extension(crate::core::ExtensionValue {
                    provider,
                    type_name,
                    handle: u64::from_be_bytes(bytes.try_into().unwrap()),
                }))
            }
            TAGGED => {
                let Value::Symbol(tag) = self.value(depth + 1)? else {
                    return Err("hta/value-malformed: invalid tagged literal tag".into());
                };
                Ok(Value::Tagged(Box::new(
                    crate::lang::data::TaggedLiteral::new(tag, self.value(depth + 1)?),
                )))
            }
            EXCEPTION_INFO => {
                let Value::String(message) = self.value(depth + 1)? else {
                    return Err("hta/value-malformed: invalid exception message".into());
                };
                let data = self.value(depth + 1)?;
                let cause = match self.value(depth + 1)? {
                    Value::Nil => None,
                    value @ Value::ExceptionInfo(_) => Some(Box::new(value)),
                    _ => {
                        return Err("hta/value-malformed: invalid exception cause".into());
                    }
                };
                if crate::core::map_entries(&data).is_none() {
                    return Err("hta/value-malformed: invalid exception data".into());
                }
                let provenance = self.value(depth + 1)?;
                let (created_at, throws) = decode_exception_provenance(provenance)?;
                Ok(Value::ExceptionInfo(std::rc::Rc::new(
                    crate::core::ExceptionInfo {
                        message,
                        data: Box::new(data),
                        cause,
                        provenance: std::rc::Rc::new(std::cell::RefCell::new(
                            crate::core::ExceptionProvenance { created_at, throws },
                        )),
                    },
                )))
            }
            STRUCT => {
                let Value::String(name) = self.value(depth + 1)? else {
                    return Err("hta/value-malformed: invalid struct name".into());
                };
                let fields = match self.value(depth + 1)? {
                    Value::Vector(values) => values
                        .iter()
                        .map(|value| match value {
                            Value::String(field) => Ok(field.clone()),
                            _ => Err("hta/value-malformed: invalid struct field".into()),
                        })
                        .collect::<Result<Vec<_>, String>>()?,
                    _ => return Err("hta/value-malformed: invalid struct fields".into()),
                };
                let values: Vec<Value> = match self.value(depth + 1)? {
                    Value::Vector(values) => values.iter().cloned().collect(),
                    _ => return Err("hta/value-malformed: invalid struct values".into()),
                };
                if fields.len() != values.len() {
                    return Err("hta/value-malformed: struct arity mismatch".into());
                }
                if let Some(result) = decode_result_struct(&name, &fields, &values)? {
                    return Ok(result);
                }
                Ok(Value::Struct(std::rc::Rc::new(
                    crate::core::StructValue::from_values(
                        std::rc::Rc::new(crate::core::StructType::detached(name, fields)),
                        values,
                        None,
                    )?,
                )))
            }
            POINTER => {
                let Value::Keyword(context) = self.value(depth + 1)? else {
                    return Err("hta/value-malformed: invalid pointer context".into());
                };
                let Value::Map(fields) = self.value(depth + 1)? else {
                    return Err("hta/value-malformed: invalid pointer fields".into());
                };
                Ok(Value::Pointer(crate::lang::data::Pointer::new(
                    context, fields,
                )))
            }
            _ => Err(format!("hta/value-malformed: unknown value tag {tag}")),
        }
    }
    fn sequence(&mut self, depth: usize) -> Result<Vec<Value>, String> {
        let size = self.len()?;
        if size > self.bytes.len().saturating_sub(self.cursor) {
            return Err("hta/value-malformed: impossible sequence length".into());
        }
        (0..size).map(|_| self.value(depth + 1)).collect()
    }
    fn entries(&mut self, depth: usize) -> Result<Vec<(Value, Value)>, String> {
        let size = self.len()?;
        if size > self.bytes.len().saturating_sub(self.cursor) / 2 {
            return Err("hta/value-malformed: impossible map length".into());
        }
        (0..size)
            .map(|_| Ok((self.value(depth + 1)?, self.value(depth + 1)?)))
            .collect()
    }
    fn data(&mut self) -> Result<&[u8], String> {
        let size = self.len()?;
        self.take(size)
    }
    fn len(&mut self) -> Result<usize, String> {
        let bytes = self.take(4)?;
        Ok(u32::from_be_bytes(bytes.try_into().unwrap()) as usize)
    }
    fn byte(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn take(&mut self, size: usize) -> Result<&[u8], String> {
        let end = self
            .cursor
            .checked_add(size)
            .ok_or("hta/value-malformed: length overflow")?;
        if end > self.bytes.len() {
            return Err("hta/value-malformed: truncated value".into());
        }
        let output = &self.bytes[self.cursor..end];
        self.cursor = end;
        Ok(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn canonical_round_trip() {
        let value = Value::Map(
            vec![
                (Value::Keyword("b".into()), Value::Number(2)),
                (
                    Value::Keyword("a".into()),
                    Value::Vector(PVector::from(vec![Value::Bool(true), Value::Nil])),
                ),
            ]
            .into_iter()
            .collect(),
        );
        let encoded = encode(&value).unwrap();
        assert_eq!(encode(&decode(&encoded).unwrap()).unwrap(), encoded);
    }
    #[test]
    fn compact_vectors_and_legacy_tuples_have_distinct_wire_boundaries() {
        let tuple = Value::Tuple(Box::new(
            PTuple::from_values(vec![Value::Number(1), Value::Number(2)]).unwrap(),
        ));
        let encoded = encode(&tuple).unwrap();
        assert_eq!(encoded[4], VECTOR);
        assert!(matches!(decode(&encoded).unwrap(), Value::Vector(_)));

        let mut legacy = encoded;
        legacy[4] = TUPLE;
        assert!(matches!(decode(&legacy).unwrap(), Value::Tuple(_)));

        let entry = Value::MapEntry(Box::new(PMapEntry::new(
            Value::Keyword("key".into()),
            Value::Number(42),
        )));
        let encoded_entry = encode(&entry).unwrap();
        assert_eq!(encoded_entry[4], MAP_ENTRY);
        assert_eq!(decode(&encoded_entry).unwrap(), entry);
    }

    #[test]
    fn pointers_round_trip_as_descriptors() {
        let fields = vec![(Value::Keyword("id".into()), Value::String("ROOT".into()))]
            .into_iter()
            .collect();
        let pointer = Value::Pointer(crate::lang::data::Pointer::new(
            crate::lang::data::Keyword::from("kernel"),
            fields,
        ));
        assert_eq!(decode(&encode(&pointer).unwrap()).unwrap(), pointer);
    }

    #[test]
    fn immutable_v3_values_round_trip_without_collection_normalization() {
        let queue = Value::Queue(Box::new(
            vec![Value::Number(1), Value::Number(2)]
                .into_iter()
                .collect(),
        ));
        assert!(matches!(
            decode(&encode(&queue).unwrap()).unwrap(),
            Value::Queue(_)
        ));
        let deque = Value::Deque(Box::new(
            vec![Value::Number(1), Value::Number(2)]
                .into_iter()
                .collect(),
        ));
        assert!(matches!(
            decode(&encode(&deque).unwrap()).unwrap(),
            Value::Deque(_)
        ));
        let priority_map = Value::PriorityMap(Box::new(
            vec![
                (Value::Keyword("a".into()), Value::Number(2)),
                (Value::Keyword("b".into()), Value::Number(1)),
            ]
            .into_iter()
            .collect(),
        ));
        let decoded = decode(&encode(&priority_map).unwrap()).unwrap();
        assert!(matches!(decoded, Value::PriorityMap(_)));
        assert_eq!(
            crate::core::map_entries(&decoded).unwrap()[0].0,
            Value::Keyword("b".into())
        );
        let tagged = Value::Tagged(Box::new(crate::lang::data::TaggedLiteral::new(
            crate::lang::data::Symbol::parse("demo/tag"),
            Value::Number(42),
        )));
        assert!(matches!(
            decode(&encode(&tagged).unwrap()).unwrap(),
            Value::Tagged(_)
        ));
    }
    #[test]
    fn floats_round_trip_with_ieee_754_bits() {
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
    fn big_integer_wire_widths_are_canonicalized() {
        for (value, tag) in [
            (BigInt::from(i64::MIN), I64),
            (BigInt::from(42_i64), I64),
            (BigInt::from(i64::MAX), I64),
            (BigInt::from(i64::MAX) + 1, BIG_INTEGER),
        ] {
            let encoded = encode(&Value::BigInteger(value)).unwrap();
            assert_eq!(encoded[4], tag);
            assert_eq!(
                decode_canonical(&encoded).unwrap(),
                decode(&encoded).unwrap()
            );
        }
    }

    #[test]
    fn canonical_decoder_rejects_noncanonical_big_integer_and_map_frames() {
        let noncanonical_integer = b"HTA0\x14\0\0\0\x02\x34\x32";
        assert!(decode_canonical(noncanonical_integer)
            .unwrap_err()
            .starts_with("hta/value-noncanonical:"));

        let mut noncanonical_map = b"HTA0\x0b\0\0\0\x02".to_vec();
        noncanonical_map.extend(bare(&Value::String("b".into()), 0).unwrap());
        noncanonical_map.extend(bare(&Value::Number(2), 0).unwrap());
        noncanonical_map.extend(bare(&Value::String("a".into()), 0).unwrap());
        noncanonical_map.extend(bare(&Value::Number(1), 0).unwrap());
        assert!(decode_canonical(&noncanonical_map)
            .unwrap_err()
            .starts_with("hta/value-noncanonical:"));
    }

    #[test]
    fn portable_language_scalars_round_trip() {
        for value in [
            Value::Character('雪'),
            Value::BigInteger(BigInt::parse_bytes(b"123456789012345678901234567890", 10).unwrap()),
            Value::Float(1.25),
            Value::Regex("^[a-z]+$".into()),
        ] {
            assert_eq!(decode(&encode(&value).unwrap()).unwrap(), value);
        }
    }

    #[test]
    fn scalar_regex_and_pointer_tags_match_the_portable_golden_vectors() {
        assert_eq!(
            encode(&Value::Character('λ')).unwrap(),
            b"HTA0\x13\0\0\x03\xbb"
        );
        assert_eq!(
            encode(&Value::Regex("a+".into())).unwrap(),
            b"HTA0\x16\0\0\0\x02a+"
        );
        let fields = vec![(Value::Keyword("id".into()), Value::String("ROOT".into()))]
            .into_iter()
            .collect();
        let pointer = Value::Pointer(crate::lang::data::Pointer::new(
            crate::lang::data::Keyword::from("kernel"),
            fields,
        ));
        assert_eq!(
            encode(&pointer).unwrap(),
            b"HTA0\x22\x06\0\0\0\x06kernel\x0b\0\0\0\x01\x06\0\0\0\x02id\x04\0\0\0\x04ROOT"
        );
    }

    #[test]
    fn tag_inventory_keeps_legacy_var_distinct_from_var_reference() {
        assert_eq!(
            HTA0_TAG_INVENTORY
                .iter()
                .find(|(_, name)| *name == "character"),
            Some(&(19, "character"))
        );
        assert_eq!(
            HTA0_TAG_INVENTORY
                .iter()
                .find(|(_, name)| *name == "pointer"),
            Some(&(34, "pointer"))
        );
        assert_eq!(
            HTA0_TAG_INVENTORY
                .iter()
                .find(|(_, name)| *name == "var-ref"),
            Some(&(35, "var-ref"))
        );
        assert!(decode(b"HTA0\x0e\x07\0\0\0\x04rank\0").is_err());
    }

    #[test]
    fn native_result_round_trips_through_the_canonical_struct_shape() {
        let context = Value::Map(
            vec![(Value::Keyword("source".into()), Value::String("hta".into()))]
                .into_iter()
                .collect(),
        );
        let value = Value::Result(std::rc::Rc::new(
            ResultValue::success(Value::Number(42), context).unwrap(),
        ));

        let encoded = encode(&value).unwrap();
        let decoded = decode(&encoded).unwrap();

        assert_eq!(decoded, value);
        assert!(matches!(decoded, Value::Result(_)));
        assert!(encoded
            .windows(17)
            .any(|bytes| bytes == b"std.native/Result"));
    }

    #[test]
    fn canonical_maps_ignore_insertion_order() {
        let a = Value::Map(
            vec![
                (Value::String("b".into()), Value::Number(2)),
                (Value::String("a".into()), Value::Number(1)),
            ]
            .into_iter()
            .collect(),
        );
        let b = Value::Map(
            vec![
                (Value::String("a".into()), Value::Number(1)),
                (Value::String("b".into()), Value::Number(2)),
            ]
            .into_iter()
            .collect(),
        );
        assert_eq!(encode(&a).unwrap(), encode(&b).unwrap());
    }
    #[test]
    fn namespaces_and_vars_use_snapshot_and_reference_contracts() {
        let namespace = crate::kernel::Namespace::new("example.lib");
        let var = namespace.intern("answer", Value::Number(42));
        let value = Value::Map(
            vec![
                (
                    Value::Keyword("namespace".into()),
                    Value::Namespace(std::rc::Rc::new(namespace)),
                ),
                (Value::Keyword("var".into()), Value::Var(var)),
            ]
            .into_iter()
            .collect(),
        );
        let decoded = decode(&encode(&value).unwrap()).unwrap();
        let Value::Map(decoded) = decoded else {
            panic!("map snapshot")
        };
        let Value::Namespace(namespace) = decoded.get(&Value::Keyword("namespace".into())).unwrap()
        else {
            panic!("namespace snapshot")
        };
        assert_eq!(namespace.name().as_str(), "example.lib");
        let Value::Var(var) = decoded.get(&Value::Keyword("var".into())).unwrap() else {
            panic!("var snapshot")
        };
        assert_eq!(var.symbol().as_str(), "example.lib/answer");
        assert_eq!(var.deref_value(), Value::Nil);
        let encoded = encode(&Value::Var(var.clone())).unwrap();
        assert_eq!(encoded[4], VAR_REF);
        assert_eq!(encoded, b"HTA0\x23\x07\x00\x00\x00\x12example.lib/answer");
    }

    #[test]
    fn opaque_handles_round_trip() {
        let value = Value::Extension(crate::core::ExtensionValue {
            provider: "runtime".into(),
            type_name: "cursor".into(),
            handle: 42,
        });
        assert_eq!(decode(&encode(&value).unwrap()).unwrap(), value);
    }

    #[test]
    fn structs_preserve_wire_shape_and_mutables_are_rejected() {
        let ty = std::rc::Rc::new(crate::core::StructType::detached(
            "demo/Point".into(),
            vec!["x".into(), "y".into()],
        ));
        let value = Value::Struct(std::rc::Rc::new(
            crate::core::StructValue::from_values(
                ty,
                vec![Value::Number(1), Value::Number(2)],
                None,
            )
            .unwrap(),
        ));
        let decoded = decode(&encode(&value).unwrap()).unwrap();
        assert_eq!(
            crate::core::call_value(Value::Keyword("x".into()), vec![decoded.clone()]).unwrap(),
            Value::Number(1)
        );
        assert_eq!(
            crate::core::call_value(
                Value::Keyword("missing".into()),
                vec![decoded.clone(), Value::Number(7)],
            )
            .unwrap(),
            Value::Number(7)
        );
        let Value::Struct(decoded) = decoded else {
            panic!("struct value")
        };
        assert_eq!(decoded.ty.name, "demo/Point");
        assert_eq!(decoded.ty.fields, vec!["x", "y"]);
        assert_eq!(
            decoded
                .ordered_values()
                .into_iter()
                .cloned()
                .collect::<Vec<_>>(),
            vec![Value::Number(1), Value::Number(2)]
        );

        let mutable = Value::Mutable(std::rc::Rc::new(
            crate::core::MutableValue::from_values(
                std::rc::Rc::new(crate::core::MutableType::detached(
                    "demo/Cursor".into(),
                    vec!["x".into()],
                )),
                vec![Value::Number(1)],
                None,
            )
            .unwrap(),
        ));
        assert_eq!(
            encode(&mutable).unwrap_err(),
            "hta/value-unsupported: mutable values are not serializable; use (into {} value)"
        );
    }

    #[test]
    fn nesting_depth_is_bounded_on_encode_and_decode() {
        let mut value = Value::Nil;
        for _ in 0..=MAX_NESTING_DEPTH {
            value = Value::Vector(PVector::from(vec![value]));
        }
        assert!(encode(&value).unwrap_err().contains("value-too-deep"));

        let mut bytes = MAGIC.to_vec();
        for _ in 0..=MAX_NESTING_DEPTH {
            bytes.extend_from_slice(&[VECTOR, 0, 0, 0, 1]);
        }
        bytes.push(NIL);
        assert!(decode(&bytes).unwrap_err().contains("value-too-deep"));
    }

    #[test]
    fn impossible_container_lengths_fail_before_allocating() {
        let mut bytes = MAGIC.to_vec();
        bytes.extend_from_slice(&[VECTOR, 0xff, 0xff, 0xff, 0xff]);
        assert!(decode(&bytes)
            .unwrap_err()
            .contains("impossible sequence length"));
    }
}
