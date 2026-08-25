use super::Form;
use num_bigint::BigInt;
use num_traits::ToPrimitive;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};

const MAGIC: &[u8] = b"HALC";
const LEGACY_MAGIC: &[u8] = b"HIR\0";
const FORMAT_VERSION: u16 = 1;
const EXECUTABLE_FOUNDATION_FLAG: u16 = 1;
const HASH_BYTES: usize = 32;
const MAX_PAYLOAD_BYTES: usize = 64 * 1024 * 1024;
const MAX_COLLECTION_ITEMS: i32 = 1_000_000;

const NIL: u8 = 0;
const FALSE: u8 = 1;
const TRUE: u8 = 2;
const LONG: u8 = 3;
const DOUBLE: u8 = 4;
const BIG_INTEGER: u8 = 5;
const STRING: u8 = 6;
const CHARACTER: u8 = 8;
const SYMBOL: u8 = 9;
const KEYWORD: u8 = 10;
const LIST: u8 = 11;
const VECTOR: u8 = 12;
const MAP: u8 = 13;
const SET: u8 = 14;
const ORDERED_MAP: u8 = 15;
const ORDERED_SET: u8 = 16;
const REGEX: u8 = 17;

#[derive(Debug, Clone)]
pub struct HalcModule {
    pub namespace: String,
    pub resource: String,
    pub source_hash: Vec<u8>,
    pub forms: Vec<Form>,
    pub schemas: HalcSchemaIndex,
    pub origin: HalcOrigin,
}

/// The typed declarations recoverable from canonical HALC forms without
/// evaluating the module. Keys are fully qualified Var names.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HalcSchemaIndex {
    pub definitions: HashMap<String, Form>,
    pub functions: HashMap<String, Form>,
    pub definition_types: HashMap<String, super::SchemaType>,
    pub function_types: HashMap<String, super::SchemaType>,
}

impl HalcSchemaIndex {
    /// Resolves a function annotation through named-schema references while
    /// preserving recursive graph edges as `Reference` nodes.
    pub fn resolved_function_type(&self, qualified_var: &str) -> Option<&super::SchemaType> {
        let schema = self.function_types.get(qualified_var)?;
        match schema {
            super::SchemaType::Reference(name) => self.definition_types.get(name).or(Some(schema)),
            _ => Some(schema),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HalcOrigin {
    Halc,
    LegacyHir,
}

pub fn decode_halc(bytes: &[u8]) -> Result<HalcModule, String> {
    let (payload, origin) = decode_envelope(bytes)?;
    let mut reader = ByteReader::new(&payload);
    let namespace = reader.read_string()?;
    let resource = reader.read_string()?;
    let source_hash = reader.read_bytes(HASH_BYTES)?;
    let form_count = reader.read_count()?;
    let mut forms = Vec::with_capacity(form_count as usize);
    for _ in 0..form_count {
        forms.push(reader.read_value()?);
    }
    if !reader.is_empty() {
        return Err("trailing payload bytes".into());
    }
    let forms = canonicalize_schema_references(&namespace, forms)?;
    let schemas = build_schema_index(&namespace, &forms)?;
    Ok(HalcModule {
        namespace,
        resource,
        source_hash,
        forms,
        schemas,
        origin,
    })
}

fn decode_envelope(bytes: &[u8]) -> Result<(Vec<u8>, HalcOrigin), String> {
    let mut reader = ByteReader::new(bytes);
    let magic = reader.read_bytes(MAGIC.len())?;
    let origin = if magic == MAGIC {
        HalcOrigin::Halc
    } else if magic == LEGACY_MAGIC {
        HalcOrigin::LegacyHir
    } else {
        return Err("bad magic".into());
    };
    let version = reader.read_u16()?;
    if version != FORMAT_VERSION {
        return Err(format!("unsupported format version {version}"));
    }
    let flags = reader.read_u16()?;
    if flags != EXECUTABLE_FOUNDATION_FLAG {
        return Err(format!("unsupported flags {flags}"));
    }
    let payload_length = reader.read_u32()? as usize;
    if payload_length > MAX_PAYLOAD_BYTES {
        return Err(format!("invalid payload length {payload_length}"));
    }
    let expected_hash = reader.read_bytes(HASH_BYTES)?;
    let payload = reader.read_bytes(payload_length)?;
    if !reader.is_empty() {
        return Err("trailing bytes".into());
    }
    let actual_hash = Sha256::digest(&payload);
    if actual_hash[..] != expected_hash[..] {
        return Err("payload checksum mismatch".into());
    }
    Ok((payload, origin))
}

struct ByteReader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> ByteReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn is_empty(&self) -> bool {
        self.position >= self.bytes.len()
    }

    fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.position)
    }

    fn read_byte(&mut self) -> Result<u8, String> {
        if self.position >= self.bytes.len() {
            return Err("truncated artifact".into());
        }
        let byte = self.bytes[self.position];
        self.position += 1;
        Ok(byte)
    }

    fn read_bytes(&mut self, count: usize) -> Result<Vec<u8>, String> {
        if self.remaining() < count {
            return Err("truncated artifact".into());
        }
        let bytes = self.bytes[self.position..self.position + count].to_vec();
        self.position += count;
        Ok(bytes)
    }

    fn read_u16(&mut self) -> Result<u16, String> {
        let bytes = self.read_bytes(2)?;
        Ok(u16::from_be_bytes([bytes[0], bytes[1]]))
    }

    fn read_u32(&mut self) -> Result<u32, String> {
        let bytes = self.read_bytes(4)?;
        Ok(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn read_i64(&mut self) -> Result<i64, String> {
        let bytes = self.read_bytes(8)?;
        Ok(i64::from_be_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]))
    }

    fn read_f64(&mut self) -> Result<f64, String> {
        let bytes = self.read_bytes(8)?;
        let value = f64::from_be_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]);
        if !value.is_finite() {
            return Err("non-finite number".into());
        }
        Ok(value)
    }

    fn read_string(&mut self) -> Result<String, String> {
        let length = self.read_u32()? as usize;
        if length > MAX_PAYLOAD_BYTES {
            return Err(format!("invalid string length {length}"));
        }
        let bytes = self.read_bytes(length)?;
        String::from_utf8(bytes).map_err(|_| "invalid UTF-8 in string".to_string())
    }

    fn read_nullable_string(&mut self) -> Result<Option<String>, String> {
        let present = self.read_byte()? != 0;
        if present {
            Ok(Some(self.read_string()?))
        } else {
            Ok(None)
        }
    }

    fn read_count(&mut self) -> Result<i32, String> {
        let count = self.read_u32()? as i32;
        if count < 0 || count > MAX_COLLECTION_ITEMS {
            return Err(format!("invalid collection count {count}"));
        }
        Ok(count)
    }

    fn read_metadata(&mut self) -> Result<Option<Form>, String> {
        let present = self.read_byte()? != 0;
        if present {
            Ok(Some(self.read_value()?))
        } else {
            Ok(None)
        }
    }

    fn read_value(&mut self) -> Result<Form, String> {
        let opcode = self.read_byte()?;
        match opcode {
            NIL => Ok(Form::Nil),
            FALSE => Ok(Form::Bool(false)),
            TRUE => Ok(Form::Bool(true)),
            LONG => Ok(Form::Number(self.read_i64()?)),
            DOUBLE => Ok(Form::Float(self.read_f64()?)),
            BIG_INTEGER => {
                let text = self.read_string()?;
                let value = BigInt::parse_bytes(text.as_bytes(), 10)
                    .ok_or_else(|| "invalid big integer".to_string())?;
                Ok(match value.to_i64() {
                    Some(value) => Form::Number(value),
                    None => Form::BigInteger(value),
                })
            }
            STRING => Ok(Form::String(self.read_string()?)),
            CHARACTER => Ok(Form::Character(
                char::from_u32(self.read_u32()?).ok_or("invalid character code point")?,
            )),
            SYMBOL => {
                let namespace = self.read_nullable_string()?;
                let name = self.read_string()?;
                Ok(with_metadata(
                    Form::Symbol(namespaced(namespace, name)),
                    self.read_metadata()?,
                ))
            }
            KEYWORD => {
                let namespace = self.read_nullable_string()?;
                let name = self.read_string()?;
                Ok(with_metadata(
                    Form::Keyword(namespaced(namespace, name)),
                    self.read_metadata()?,
                ))
            }
            LIST => {
                let count = self.read_count()?;
                let items = self.read_values(count)?;
                Ok(with_metadata(Form::List(items), self.read_metadata()?))
            }
            VECTOR => {
                let count = self.read_count()?;
                let items = self.read_values(count)?;
                Ok(with_metadata(Form::Vector(items), self.read_metadata()?))
            }
            MAP | ORDERED_MAP => {
                let count = self.read_count()?;
                let mut entries = Vec::with_capacity(count as usize);
                for _ in 0..count {
                    let key = self.read_value()?;
                    let value = self.read_value()?;
                    entries.push((key, value));
                }
                Ok(with_metadata(Form::Map(entries), self.read_metadata()?))
            }
            SET | ORDERED_SET => {
                let count = self.read_count()?;
                let items = self.read_values(count)?;
                Ok(with_metadata(Form::Set(items), self.read_metadata()?))
            }
            REGEX => Ok(Form::Regex(self.read_string()?)),
            _ => Err(format!("unknown value opcode {opcode}")),
        }
    }

    fn read_values(&mut self, count: i32) -> Result<Vec<Form>, String> {
        let mut values = Vec::with_capacity(count as usize);
        for _ in 0..count {
            values.push(self.read_value()?);
        }
        Ok(values)
    }
}

fn with_metadata(value: Form, metadata: Option<Form>) -> Form {
    match metadata {
        Some(metadata) => Form::Metadata(Box::new(metadata), Box::new(value)),
        None => value,
    }
}

fn namespaced(namespace: Option<String>, name: String) -> String {
    match namespace {
        Some(ns) => format!("{ns}/{name}"),
        None => name,
    }
}

#[cfg(any(test, feature = "halc-encoder"))]
fn write_string(output: &mut Vec<u8>, value: &str) {
    output.extend_from_slice(&(value.len() as u32).to_be_bytes());
    output.extend_from_slice(value.as_bytes());
}

#[cfg(any(test, feature = "halc-encoder"))]
fn write_count(output: &mut Vec<u8>, count: i32) {
    output.extend_from_slice(&count.to_be_bytes());
}

#[cfg(any(test, feature = "halc-encoder"))]
fn write_namespaced(output: &mut Vec<u8>, symbol: &str) {
    if let Some((ns, name)) = symbol.rsplit_once('/') {
        output.push(1);
        write_string(output, ns);
        write_string(output, name);
    } else {
        output.push(0);
        write_string(output, symbol);
    }
}

#[cfg(any(test, feature = "halc-encoder"))]
fn write_values(output: &mut Vec<u8>, values: &[Form]) {
    write_count(output, values.len() as i32);
    for value in values {
        write_value(output, value);
    }
}

#[cfg(any(test, feature = "halc-encoder"))]
fn write_value(output: &mut Vec<u8>, form: &Form) {
    match form {
        Form::Metadata(metadata, value) => write_value_with_metadata(output, value, Some(metadata)),
        _ => write_value_with_metadata(output, form, None),
    }
}

#[cfg(any(test, feature = "halc-encoder"))]
fn write_metadata(output: &mut Vec<u8>, metadata: Option<&Form>) {
    match metadata {
        Some(metadata) => {
            output.push(1);
            write_value(output, metadata);
        }
        None => output.push(0),
    }
}

#[cfg(any(test, feature = "halc-encoder"))]
fn write_value_with_metadata(output: &mut Vec<u8>, form: &Form, metadata: Option<&Form>) {
    match form {
        Form::Nil => output.push(NIL),
        Form::Bool(false) => output.push(FALSE),
        Form::Bool(true) => output.push(TRUE),
        Form::Number(n) => {
            output.push(LONG);
            output.extend_from_slice(&n.to_be_bytes());
        }
        Form::Float(f) => {
            assert!(f.is_finite(), "non-finite number");
            output.push(DOUBLE);
            output.extend_from_slice(&f.to_be_bytes());
        }
        Form::BigInteger(s) => {
            output.push(BIG_INTEGER);
            write_string(output, &s.to_string());
        }
        Form::String(s) => {
            output.push(STRING);
            write_string(output, s);
        }
        Form::Character(c) => {
            output.push(CHARACTER);
            output.extend_from_slice(&(*c as u32).to_be_bytes());
        }
        Form::Symbol(s) => {
            output.push(SYMBOL);
            write_namespaced(output, s);
            write_metadata(output, metadata);
        }
        Form::Keyword(s) => {
            output.push(KEYWORD);
            write_namespaced(output, s);
            write_metadata(output, metadata);
        }
        Form::List(items) => {
            output.push(LIST);
            write_values(output, items);
            write_metadata(output, metadata);
        }
        Form::Vector(items) => {
            output.push(VECTOR);
            write_values(output, items);
            write_metadata(output, metadata);
        }
        Form::Map(entries) => {
            output.push(ORDERED_MAP);
            write_count(output, entries.len() as i32);
            for (key, value) in entries {
                write_value(output, key);
                write_value(output, value);
            }
            write_metadata(output, metadata);
        }
        Form::Set(items) => {
            output.push(ORDERED_SET);
            write_values(output, items);
            write_metadata(output, metadata);
        }
        Form::Regex(s) => {
            output.push(REGEX);
            write_string(output, s);
        }
        Form::Tagged(_, _) | Form::Metadata(_, _) => {
            panic!("test encoder does not support tagged/metadata forms")
        }
    }
}

/// Encodes a canonical HALC artifact from parsed forms.
///
/// This mirrors the v1 format used by `decode_halc` so that integration tests
/// can construct artifacts without depending on an external encoder.
#[cfg(any(test, feature = "halc-encoder"))]
pub fn encode_halc_module(
    namespace: &str,
    resource: &str,
    source: &str,
    forms: Vec<Form>,
) -> Result<Vec<u8>, String> {
    let forms = canonicalize_schema_references(namespace, forms)?;
    for form in &forms {
        validate_finite_form(form)?;
    }
    build_schema_index(namespace, &forms)?;
    let mut payload = Vec::new();
    write_string(&mut payload, namespace);
    write_string(&mut payload, resource);
    payload.extend_from_slice(&Sha256::digest(source.as_bytes()));
    write_count(&mut payload, forms.len() as i32);
    for form in forms {
        write_value(&mut payload, &form);
    }
    let mut artifact = Vec::new();
    artifact.extend_from_slice(MAGIC);
    artifact.extend_from_slice(&FORMAT_VERSION.to_be_bytes());
    artifact.extend_from_slice(&EXECUTABLE_FOUNDATION_FLAG.to_be_bytes());
    artifact.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    artifact.extend_from_slice(&Sha256::digest(&payload));
    artifact.extend_from_slice(&payload);
    Ok(artifact)
}

#[cfg(any(test, feature = "halc-encoder"))]
fn validate_finite_form(form: &Form) -> Result<(), String> {
    match form {
        Form::Float(value) if !value.is_finite() => Err("non-finite number".into()),
        Form::Tagged(_, value) => validate_finite_form(value),
        Form::Metadata(metadata, value) => {
            validate_finite_form(metadata)?;
            validate_finite_form(value)
        }
        Form::Map(entries) => {
            for (key, value) in entries {
                validate_finite_form(key)?;
                validate_finite_form(value)?;
            }
            Ok(())
        }
        Form::Set(values) | Form::Vector(values) | Form::List(values) => {
            for value in values {
                validate_finite_form(value)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn canonicalize_schema_references(
    namespace: &str,
    mut forms: Vec<Form>,
) -> Result<Vec<Form>, String> {
    let definitions: HashSet<String> = forms
        .iter()
        .filter_map(|form| {
            let Form::List(items) = form else { return None };
            let Form::Symbol(operator) = items.first()? else {
                return None;
            };
            if !matches!(
                operator.as_str(),
                "def" | "defn" | "defn-" | "defmacro" | "defstruct" | "declare"
            ) {
                return None;
            }
            binding_name(items.get(1)?).map(str::to_owned)
        })
        .collect();
    let schema_values: HashMap<String, usize> = forms
        .iter()
        .enumerate()
        .filter_map(|(index, form)| {
            let Form::List(items) = form else { return None };
            if !matches!(items.first(), Some(Form::Symbol(operator)) if operator == "def") {
                return None;
            }
            binding_name(items.get(1)?).map(|name| (name.to_owned(), index))
        })
        .collect();
    let aliases = module_aliases(&forms);
    let mut schema_roots = Vec::new();

    for form in &mut forms {
        let Form::List(items) = form else { continue };
        let Some(Form::Symbol(operator)) = items.first() else {
            continue;
        };
        if operator != "defn" && operator != "defn-" {
            continue;
        }
        let Some(Form::Metadata(metadata, _)) = items.get_mut(1) else {
            continue;
        };
        let Form::Map(entries) = metadata.as_mut() else {
            continue;
        };
        let Some((_, schema)) = entries
            .iter_mut()
            .find(|(key, _)| matches!(key, Form::Keyword(name) if name == "schema"))
        else {
            continue;
        };
        let Form::List(reference) = schema else {
            continue;
        };
        if reference.len() != 2
            || !matches!(&reference[0], Form::Symbol(operator) if operator == "var")
        {
            continue;
        }
        let Form::Symbol(target) = &reference[1] else {
            continue;
        };
        let (qualifier, local) = target
            .rsplit_once('/')
            .map_or((None, target.as_str()), |(qualifier, local)| {
                (Some(qualifier), local)
            });
        let target_namespace = match qualifier {
            None | Some("-") => namespace,
            Some(qualifier) => aliases.get(qualifier).map_or(qualifier, String::as_str),
        };
        if target_namespace == namespace && !definitions.contains(local) {
            return Err(format!("schema Var does not exist: {target}"));
        }
        if target_namespace == namespace {
            schema_roots.push(local.to_owned());
        }
        reference[1] = Form::Symbol(format!("{target_namespace}/{local}"));
    }

    let mut visited = HashSet::new();
    while let Some(schema_name) = schema_roots.pop() {
        if !visited.insert(schema_name.clone()) {
            continue;
        }
        let Some(index) = schema_values.get(&schema_name).copied() else {
            continue;
        };
        let Form::List(definition) = &mut forms[index] else {
            continue;
        };
        let Some(schema_value) = definition.get_mut(2) else {
            continue;
        };
        canonicalize_nested_schema_references(
            schema_value,
            namespace,
            &aliases,
            &definitions,
            &mut schema_roots,
        )?;
    }
    Ok(forms)
}

fn canonicalize_nested_schema_references(
    form: &mut Form,
    namespace: &str,
    aliases: &HashMap<String, String>,
    definitions: &HashSet<String>,
    local_references: &mut Vec<String>,
) -> Result<(), String> {
    if let Form::List(reference) = form {
        if reference.len() == 2
            && matches!(&reference[0], Form::Symbol(operator) if operator == "var")
        {
            let Form::Symbol(target) = &reference[1] else {
                return Ok(());
            };
            let original = target.clone();
            let (qualifier, local) = original
                .rsplit_once('/')
                .map_or((None, original.as_str()), |(qualifier, local)| {
                    (Some(qualifier), local)
                });
            let target_namespace = match qualifier {
                None | Some("-") => namespace,
                Some(qualifier) => aliases.get(qualifier).map_or(qualifier, String::as_str),
            };
            if target_namespace == namespace {
                if !definitions.contains(local) {
                    return Err(format!("schema Var does not exist: {original}"));
                }
                local_references.push(local.to_owned());
            }
            reference[1] = Form::Symbol(format!("{target_namespace}/{local}"));
            return Ok(());
        }
    }

    match form {
        Form::Tagged(_, value) => canonicalize_nested_schema_references(
            value,
            namespace,
            aliases,
            definitions,
            local_references,
        ),
        Form::Metadata(metadata, value) => {
            canonicalize_nested_schema_references(
                metadata,
                namespace,
                aliases,
                definitions,
                local_references,
            )?;
            canonicalize_nested_schema_references(
                value,
                namespace,
                aliases,
                definitions,
                local_references,
            )
        }
        Form::Map(entries) => {
            for (key, value) in entries {
                canonicalize_nested_schema_references(
                    key,
                    namespace,
                    aliases,
                    definitions,
                    local_references,
                )?;
                canonicalize_nested_schema_references(
                    value,
                    namespace,
                    aliases,
                    definitions,
                    local_references,
                )?;
            }
            Ok(())
        }
        Form::Set(values) | Form::Vector(values) | Form::List(values) => {
            for value in values {
                canonicalize_nested_schema_references(
                    value,
                    namespace,
                    aliases,
                    definitions,
                    local_references,
                )?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn binding_name(form: &Form) -> Option<&str> {
    match form {
        Form::Symbol(name) => Some(name),
        Form::Metadata(_, value) => binding_name(value),
        _ => None,
    }
}

fn module_aliases(forms: &[Form]) -> HashMap<String, String> {
    let mut aliases = HashMap::new();
    for form in forms {
        let Form::List(declaration) = form else {
            continue;
        };
        if !matches!(declaration.first(), Some(Form::Symbol(operator)) if operator == "ns") {
            continue;
        }
        for clause in declaration.iter().skip(2) {
            let Form::List(clause) = clause else { continue };
            if !matches!(clause.first(), Some(Form::Keyword(keyword)) if keyword == "require") {
                continue;
            }
            for spec in clause.iter().skip(1) {
                let Form::Vector(spec) = spec else { continue };
                let Some(Form::Symbol(target)) = spec.first() else {
                    continue;
                };
                for option in spec[1..].chunks(2) {
                    if let [Form::Keyword(key), Form::Symbol(alias)] = option {
                        if key == "as" {
                            aliases.insert(alias.clone(), target.clone());
                        }
                    }
                }
            }
        }
    }
    aliases
}

fn build_schema_index(namespace: &str, forms: &[Form]) -> Result<HalcSchemaIndex, String> {
    let mut index = HalcSchemaIndex::default();
    let mut values = HashMap::new();
    let mut roots = Vec::new();

    for form in forms {
        let Form::List(items) = form else { continue };
        let Some(Form::Symbol(operator)) = items.first() else {
            continue;
        };
        let Some(name) = items.get(1).and_then(binding_name) else {
            continue;
        };
        let qualified_name = format!("{namespace}/{name}");
        if operator == "def" {
            if let Some(value) = items.get(2) {
                values.insert(name.to_owned(), value.clone());
            }
            continue;
        }
        if operator != "defn" && operator != "defn-" {
            continue;
        }
        let Some(Form::Metadata(metadata, _)) = items.get(1) else {
            continue;
        };
        let Form::Map(entries) = metadata.as_ref() else {
            continue;
        };
        let Some(schema) = entries.iter().find_map(|(key, value)| {
            matches!(key, Form::Keyword(name) if name == "schema").then_some(value)
        }) else {
            continue;
        };
        index.functions.insert(qualified_name, schema.clone());
        collect_local_schema_references(schema, namespace, &mut roots);
    }

    let mut visited = HashSet::new();
    while let Some(name) = roots.pop() {
        if !visited.insert(name.clone()) {
            continue;
        }
        let Some(value) = values.get(&name) else {
            continue;
        };
        index
            .definitions
            .insert(format!("{namespace}/{name}"), value.clone());
        collect_local_schema_references(value, namespace, &mut roots);
    }
    for (name, schema) in &index.definitions {
        index.definition_types.insert(
            name.clone(),
            super::normalize_schema(schema)
                .map_err(|error| format!("invalid schema {name}: {error}"))?,
        );
    }
    for (name, schema) in &index.functions {
        index.function_types.insert(
            name.clone(),
            super::normalize_schema(schema)
                .map_err(|error| format!("invalid function schema {name}: {error}"))?,
        );
    }
    Ok(index)
}

fn collect_local_schema_references(form: &Form, namespace: &str, output: &mut Vec<String>) {
    if let Form::List(reference) = form {
        if reference.len() == 2
            && matches!(&reference[0], Form::Symbol(operator) if operator == "var")
        {
            if let Form::Symbol(target) = &reference[1] {
                if let Some((qualifier, local)) = target.rsplit_once('/') {
                    if qualifier == namespace {
                        output.push(local.to_owned());
                    }
                }
            }
            return;
        }
    }
    match form {
        Form::Tagged(_, value) => collect_local_schema_references(value, namespace, output),
        Form::Metadata(metadata, value) => {
            collect_local_schema_references(metadata, namespace, output);
            collect_local_schema_references(value, namespace, output);
        }
        Form::Map(entries) => {
            for (key, value) in entries {
                collect_local_schema_references(key, namespace, output);
                collect_local_schema_references(value, namespace, output);
            }
        }
        Form::Set(values) | Form::Vector(values) | Form::List(values) => {
            for value in values {
                collect_local_schema_references(value, namespace, output);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::parse;

    fn artifact_payload(forms: Vec<Form>) -> Vec<u8> {
        encode_halc_module("demo.ns", "demo.hal", "", forms).unwrap()
    }

    #[test]
    fn round_trips_primitive_values() {
        let cases = [
            "nil",
            "true",
            "false",
            "42",
            "-7",
            "3.14",
            "\"hello\"",
            "\\x",
            ":key",
            ":ns/key",
            "symbol",
            "ns/symbol",
        ];
        for source in cases {
            let original = parse(source).unwrap();
            let bytes = artifact_payload(vec![original.clone()]);
            let decoded = decode_halc(&bytes).unwrap();
            assert_eq!(decoded.forms.len(), 1);
            assert_eq!(decoded.forms[0], original, "{source}");
        }
    }

    #[test]
    fn round_trips_collections() {
        let original = parse("(do {:a [1 2] :b #{3 4}})").unwrap();
        let bytes = artifact_payload(vec![original.clone()]);
        let decoded = decode_halc(&bytes).unwrap();
        assert_eq!(decoded.forms.len(), 1);
        assert_eq!(decoded.forms[0], original);
    }

    #[test]
    fn schema_var_references_are_checked_and_namespace_canonicalized() {
        let source = "(ns demo.schema) \
                      (def Customer [:map [:id :int]]) \
                      (defn ^{:schema #'-/Customer} customer-id [customer] customer)";
        let bytes = encode_halc_module(
            "demo.schema",
            "demo/schema.hal",
            source,
            crate::kernel::parse_forms(source).unwrap(),
        )
        .unwrap();
        let module = decode_halc(&bytes).unwrap();
        let Form::List(definition) = &module.forms[2] else {
            panic!("expected defn");
        };
        let Form::Metadata(metadata, _) = &definition[1] else {
            panic!("expected definition metadata");
        };
        let Form::Map(metadata) = metadata.as_ref() else {
            panic!("expected metadata map");
        };
        let schema = metadata
            .iter()
            .find_map(|(key, value)| {
                matches!(key, Form::Keyword(name) if name == "schema").then_some(value)
            })
            .unwrap();
        assert_eq!(
            schema,
            &Form::List(vec![
                Form::Symbol("var".into()),
                Form::Symbol("demo.schema/Customer".into()),
            ])
        );

        let missing = "(ns demo.schema) \
                       (defn ^{:schema #'MissingSchema} invalid [value] value)";
        assert_eq!(
            encode_halc_module(
                "demo.schema",
                "demo/schema.hal",
                missing,
                crate::kernel::parse_forms(missing).unwrap(),
            )
            .unwrap_err(),
            "schema Var does not exist: MissingSchema"
        );
    }

    #[test]
    fn nested_schema_var_references_are_canonicalized_and_checked() {
        let source = "(ns demo.schema) \
                      (def Address [:map [:street :str]]) \
                      (def Customer [:map [:address #'-/Address]]) \
                      (defn ^{:schema #'Customer} save [customer] customer)";
        let bytes = encode_halc_module(
            "demo.schema",
            "demo/schema.hal",
            source,
            crate::kernel::parse_forms(source).unwrap(),
        )
        .unwrap();
        let module = decode_halc(&bytes).unwrap();
        assert!(module.forms[2]
            .to_string()
            .contains("(var demo.schema/Address)"));
        assert_eq!(module.schemas.functions.len(), 1);
        assert!(module.schemas.functions.contains_key("demo.schema/save"));
        assert_eq!(module.schemas.definitions.len(), 2);
        assert!(module
            .schemas
            .definitions
            .contains_key("demo.schema/Address"));
        assert!(module
            .schemas
            .definitions
            .contains_key("demo.schema/Customer"));
        assert!(matches!(
            module.schemas.resolved_function_type("demo.schema/save"),
            Some(super::super::SchemaType::Map(fields)) if fields.len() == 1
        ));

        let missing = "(ns demo.schema) \
                       (def Customer [:map [:address #'MissingAddress]]) \
                       (defn ^{:schema #'Customer} save [customer] customer)";
        assert_eq!(
            encode_halc_module(
                "demo.schema",
                "demo/schema.hal",
                missing,
                crate::kernel::parse_forms(missing).unwrap(),
            )
            .unwrap_err(),
            "schema Var does not exist: MissingAddress"
        );

        let recursive = "(ns demo.schema) \
                         (def Node [:map [:children [:vector #'Node]]]) \
                         (defn ^{:schema #'Node} walk [node] node)";
        assert!(encode_halc_module(
            "demo.schema",
            "demo/schema.hal",
            recursive,
            crate::kernel::parse_forms(recursive).unwrap(),
        )
        .is_ok());

        let malformed = "(ns demo.schema) \
                         (def Customer [:map [:name]]) \
                         (defn ^{:schema #'Customer} save [customer] customer)";
        assert_eq!(
            encode_halc_module(
                "demo.schema",
                "demo/schema.hal",
                malformed,
                crate::kernel::parse_forms(malformed).unwrap(),
            )
            .unwrap_err(),
            "invalid schema demo.schema/Customer: :map schema fields must be [name type] or [name properties type]"
        );
    }

    #[test]
    fn round_trips_metadata() {
        let original = parse("^:dynamic *value*").unwrap();
        let bytes = artifact_payload(vec![original.clone()]);
        let decoded = decode_halc(&bytes).unwrap();
        assert_eq!(decoded.forms, vec![original]);
    }

    #[test]
    fn rejects_bad_magic() {
        let mut bytes = artifact_payload(vec![Form::Nil]);
        bytes[0] = 0;
        assert!(decode_halc(&bytes).unwrap_err().contains("bad magic"));
    }

    #[test]
    fn rejects_checksum_mismatch() {
        let mut bytes = artifact_payload(vec![Form::Nil]);
        let last = bytes.len() - 1;
        bytes[last] = bytes[last].wrapping_add(1);
        assert!(decode_halc(&bytes).unwrap_err().contains("checksum"));
    }

    #[test]
    fn decodes_the_truffle_portable_format_golden_artifact() {
        // This is the canonical v1 artifact emitted by Truffle's
        // HalcArtifactTest.goldenBytesLockThePortableFormat. Keep this test
        // independent of Rust's test-only encoder: it is the cross-runtime
        // compatibility boundary, rather than a Rust encoder/decoder
        // round-trip.
        let bytes = hex_bytes(concat!(
            "48414c43000100010000013f57211e103028689092d59627fbba64015c289acd1bc5b2e7be27ec53d8bf4c35",
            "00000001740000000174e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            "0000001100010203000000000000002a044004000000000000050000001e313233343536373839303132",
            "333435363738393031323334353637383930060000000668c3a172c3a008000000780901000000056d792e6e73",
            "000000066d792d73796d000a00000000026b77000b00000002030000000000000001060000000161000c00000002",
            "030000000000000001060000000161000d00000002030000000000000001060000000161030000000000000002",
            "060000000162000e00000002030000000000000001030000000000000002000f00000002030000000000000002",
            "060000000162030000000000000001060000000161001000000002030000000000000002030000000000000001",
            "001100000003612b62",
        ));

        let module = decode_halc(&bytes).unwrap();
        assert_eq!(module.origin, HalcOrigin::Halc);
        assert_eq!(module.namespace, "t");
        assert_eq!(module.resource, "t");
        assert_eq!(module.forms.len(), 17);
        assert_eq!(module.forms[0], Form::Nil);
        assert_eq!(module.forms[3], Form::Number(42));
        assert_eq!(module.forms[6], Form::String("hárà".into()));
        assert_eq!(module.forms[7], Form::Character('x'));
        assert_eq!(module.forms[8], Form::Symbol("my.ns/my-sym".into()));
        assert_eq!(module.forms[9], Form::Keyword("kw".into()));
        assert_eq!(module.forms[16], Form::Regex("a+b".into()));
    }

    #[test]
    fn legacy_hir_magic_decodes_but_encoding_always_uses_halc_magic() {
        let halc = artifact_payload(vec![Form::Number(42)]);
        let mut legacy = halc.clone();
        legacy[..4].copy_from_slice(LEGACY_MAGIC);

        assert_eq!(decode_halc(&legacy).unwrap().origin, HalcOrigin::LegacyHir);
        assert_eq!(&halc[..4], MAGIC);
    }

    #[test]
    fn shared_cross_runtime_goldens_decode() {
        let complete = std::fs::read(crate::spec_registry::require(
            "01-lang/009-halc/draft/conformance/golden/complete.halc",
        ))
        .expect("complete HALC golden is readable");
        let legacy = std::fs::read(crate::spec_registry::require(
            "01-lang/009-halc/draft/conformance/golden/legacy-v1.hir",
        ))
        .expect("legacy HIR golden is readable");
        let current = decode_halc(&complete).unwrap();
        assert_eq!(current.origin, HalcOrigin::Halc);
        assert_eq!(current.namespace, "halc.conformance.complete");
        assert_eq!(current.resource, "conformance/complete.hal");
        assert_eq!(decode_halc(&legacy).unwrap().origin, HalcOrigin::LegacyHir);
    }

    #[test]
    fn registry_golden_matches_rust_encoding() {
        let source_path = crate::spec_registry::require(
            "01-lang/009-halc/draft/conformance/complete.hal",
        );
        let source = std::fs::read_to_string(source_path).expect("HALC source is readable");
        let forms = crate::kernel::parse_forms(&source).expect("HALC source parses");
        let encoded = encode_halc_module(
            "halc.conformance.complete",
            "conformance/complete.hal",
            &source,
            forms,
        )
        .expect("HALC source encodes");
        let expected = std::fs::read(crate::spec_registry::require(
            "01-lang/009-halc/draft/conformance/golden/complete.halc",
        ))
        .expect("HALC golden is readable");
        assert_eq!(expected, encoded);
    }

    fn hex_bytes(hex: &str) -> Vec<u8> {
        (0..hex.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).unwrap())
            .collect()
    }
}
