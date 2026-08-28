//! Alpha persistent encoding for validated VM programs.

use std::collections::HashMap;
use std::rc::Rc;

use num_bigint::BigInt;
use num_traits::ToPrimitive;
use sha2::{Digest, Sha256};

use super::opcode::Instruction;
use super::program::{CatchEntry, FunctionPrototype, Program, TryEntry};
use super::source_map::SourceMap;
#[cfg(test)]
use crate::core::Value;
use crate::kernel::{FunctionSchema, Position, SchemaField, SchemaType};
use crate::lang::data::{Keyword, Metadata, MetadataValue, Symbol};

const MAGIC: &[u8; 4] = b"HBC0";

/// Encodes a program after validating it. Constants use the portable HTA
/// value codec; unsupported runtime-only values are rejected explicitly.
pub fn encode_program(program: &Program) -> Result<Vec<u8>, String> {
    super::validate::validate(program).map_err(|error| error.to_string())?;
    let mut payload = Writer::default();
    payload.u16(program.entry);
    payload.option_string(program.namespace.as_deref())?;
    payload.len(program.constants.len())?;
    for value in &program.constants {
        payload.bytes(&crate::hta::encode(value)?)?;
    }
    payload.len(program.var_metadata.len())?;
    for metadata in &program.var_metadata {
        write_metadata(&mut payload, metadata)?;
    }
    write_schema_map(&mut payload, &program.schema_types)?;
    write_schema_map(&mut payload, &program.function_types)?;
    write_schema_map(&mut payload, &program.inferred_function_types)?;
    payload.len(program.functions.len())?;
    for function in &program.functions {
        write_function(&mut payload, function)?;
    }
    let digest = Sha256::digest(&payload.bytes);
    let mut output = MAGIC.to_vec();
    output.extend_from_slice(
        &u32::try_from(payload.bytes.len())
            .map_err(|_| "bytecode artifact is too large")?
            .to_be_bytes(),
    );
    output.extend_from_slice(&payload.bytes);
    output.extend_from_slice(&digest);
    Ok(output)
}

/// Decodes, authenticates, and validates a persistent VM program.
pub fn decode_program(bytes: &[u8]) -> Result<Program, String> {
    if !bytes.starts_with(MAGIC) {
        return Err("bytecode artifact has invalid magic".into());
    }
    if bytes.len() < 8 + 32 {
        return Err("bytecode artifact is truncated".into());
    }
    let payload_len = u32::from_be_bytes(bytes[4..8].try_into().unwrap()) as usize;
    let payload_end = 8usize
        .checked_add(payload_len)
        .ok_or("bytecode artifact length overflow")?;
    if payload_end.checked_add(32) != Some(bytes.len()) {
        return Err("bytecode artifact length mismatch".into());
    }
    let payload = &bytes[8..payload_end];
    if Sha256::digest(payload)[..] != bytes[payload_end..] {
        return Err("bytecode artifact checksum mismatch".into());
    }
    let mut reader = Reader::new(payload);
    let entry = reader.u16()?;
    let namespace = reader.option_string()?;
    let constants = reader.many(|reader| crate::hta::decode_canonical(reader.bytes()?))?;
    let var_metadata = reader.many(|reader| read_metadata(reader))?;
    let schema_types = read_schema_map(&mut reader)?;
    let function_types = read_schema_map(&mut reader)?;
    let inferred_function_types = read_schema_map(&mut reader)?;
    let functions = reader.many(|reader| read_function(reader, 5))?;
    reader.finish()?;
    let program = Program {
        namespace,
        constants,
        var_metadata,
        schema_types,
        function_types,
        inferred_function_types,
        functions,
        entry,
    };
    super::validate::validate(&program).map_err(|error| error.to_string())?;
    Ok(program)
}

fn write_function(out: &mut Writer, function: &FunctionPrototype) -> Result<(), String> {
    out.option_string(function.name.as_deref())?;
    out.byte(u8::from(function.async_function));
    out.u16(function.arity);
    out.byte(u8::from(function.variadic));
    out.u16(function.capture_count);
    out.u16(function.local_count);
    out.u16(function.max_stack);
    out.len(function.code.len())?;
    for instruction in &function.code {
        write_instruction(out, instruction);
    }
    out.len(function.source_map.len())?;
    for index in 0..function.source_map.len() {
        match function.source_map.position(index) {
            Some(position) => {
                out.byte(1);
                out.usize32(position.offset)?;
                out.usize32(position.line)?;
                out.usize32(position.column)?;
            }
            None => out.byte(0),
        }
    }
    out.len(function.handlers.len())?;
    for handler in &function.handlers {
        out.u32(handler.start);
        out.u32(handler.end);
        out.u16(handler.depth);
        out.len(handler.catches.len())?;
        for catch in &handler.catches {
            out.string(&catch.class)?;
            out.u16(catch.binding);
            out.u32(catch.target);
        }
        out.option_u32(handler.finally);
        out.option_u16(handler.pending_value);
        out.option_u16(handler.pending_error);
    }
    Ok(())
}

fn read_function(reader: &mut Reader<'_>, version: u8) -> Result<FunctionPrototype, String> {
    let name = reader.option_string()?;
    let async_function = version >= 2 && reader.boolean()?;
    let arity = reader.u16()?;
    let variadic = reader.boolean()?;
    let capture_count = reader.u16()?;
    let local_count = reader.u16()?;
    let max_stack = reader.u16()?;
    let code = reader.many(read_instruction)?;
    let positions = reader.many(|reader| {
        if reader.boolean()? {
            Ok(Some(Position {
                offset: reader.u32()? as usize,
                line: reader.u32()? as usize,
                column: reader.u32()? as usize,
            }))
        } else {
            Ok(None)
        }
    })?;
    let mut source_map = SourceMap::default();
    for position in positions {
        source_map.record(position);
    }
    let handlers = reader.many(|reader| {
        let start = reader.u32()?;
        let end = reader.u32()?;
        let depth = reader.u16()?;
        let catches = reader.many(|reader| {
            Ok(CatchEntry {
                class: reader.string()?,
                binding: reader.u16()?,
                target: reader.u32()?,
            })
        })?;
        Ok(TryEntry {
            start,
            end,
            depth,
            catches,
            finally: reader.option_u32()?,
            pending_value: reader.option_u16()?,
            pending_error: reader.option_u16()?,
        })
    })?;
    Ok(FunctionPrototype {
        name,
        async_function,
        arity,
        variadic,
        capture_count,
        local_count,
        max_stack,
        code,
        source_map,
        handlers,
    })
}

fn write_instruction(out: &mut Writer, instruction: &Instruction) {
    use Instruction::*;
    match instruction {
        Constant(value) => {
            out.byte(0);
            out.u32(*value);
        }
        Nil => out.byte(1),
        True => out.byte(2),
        False => out.byte(3),
        LoadLocal(value) => {
            out.byte(4);
            out.u16(*value);
        }
        StoreLocal(value) => {
            out.byte(5);
            out.u16(*value);
        }
        Pop => out.byte(6),
        Dup => out.byte(28),
        IntrinsicCall { target, argc } => {
            out.byte(50);
            out.u32(*target);
            out.byte(*argc);
        }
        Jump(value) => {
            out.byte(8);
            out.u32(*value);
        }
        JumpIfFalse(value) => {
            out.byte(9);
            out.u32(*value);
        }
        Closure {
            prototype,
            captures,
        } => {
            out.byte(10);
            out.u16(*prototype);
            out.byte(*captures);
        }
        Call { argc } => {
            out.byte(11);
            out.byte(*argc);
        }
        CallStatic { prototype, argc } => {
            out.byte(12);
            out.u16(*prototype);
            out.byte(*argc);
        }
        Throw => out.byte(13),
        Rethrow => out.byte(14),
        GetGlobal(value) => {
            out.byte(15);
            out.u32(*value);
        }
        DefGlobal { name, metadata } => {
            out.byte(16);
            out.u32(*name);
            out.option_u16(*metadata);
        }
        SetGlobal(value) => {
            out.byte(17);
            out.u32(*value);
        }
        VarGlobal(value) => {
            out.byte(18);
            out.u32(*value);
        }
        DeclareGlobal(value) => {
            out.byte(19);
            out.u32(*value);
        }
        DefStruct { name, fields } => {
            out.byte(20);
            out.u32(*name);
            out.u32(*fields);
        }
        InstanceOf => out.byte(22),
        MakeMultiArity { name, count } => {
            out.byte(23);
            out.u32(*name);
            out.byte(*count);
        }
        Await => out.byte(26),
        HostCall => out.byte(27),
        DotCall { method, argc } => {
            out.byte(45);
            out.u32(*method);
            out.byte(*argc);
        }
        BuildVector(count) => {
            out.byte(29);
            out.u16(*count);
        }
        BuildMap(count) => {
            out.byte(30);
            out.u16(*count);
        }
        BuildSet(count) => {
            out.byte(31);
            out.u16(*count);
        }
        DefMacro { name, metadata } => {
            out.byte(32);
            out.u32(*name);
            out.option_u16(*metadata);
        }
        BuildList(count) => {
            out.byte(33);
            out.u16(*count);
        }
        ConcatList(count) => {
            out.byte(34);
            out.u16(*count);
        }
        ToVector => out.byte(35),
        DefProtocol(index) => {
            out.byte(41);
            out.u32(*index);
        }
        ExtendType(index) => {
            out.byte(42);
            out.u32(*index);
        }
        DefMulti(index) => {
            out.byte(43);
            out.u32(*index);
        }
        DefMethod(index) => {
            out.byte(44);
            out.u32(*index);
        }
        IntrinsicValue(target) => {
            out.byte(51);
            out.u32(*target);
        }
        ProtocolCall { target, argc } => {
            out.byte(52);
            out.u32(*target);
            out.byte(*argc);
        }
        BuiltinValue(index) => {
            out.byte(38);
            out.u32(*index);
        }
        NamespaceValue(index) => {
            out.byte(53);
            out.u32(*index);
        }
        NamespaceOperation(index) => {
            out.byte(54);
            out.u32(*index);
        }
        DynamicBind(index) => {
            out.byte(39);
            out.u32(*index);
        }
        DynamicUnbind(index) => {
            out.byte(40);
            out.u32(*index);
        }
        Yield => out.byte(46),
        DefMutable { name, fields } => {
            out.byte(47);
            out.u32(*name);
            out.u32(*fields);
        }
        MutableFieldGet(value) => {
            out.byte(48);
            out.u32(*value);
        }
        MutableFieldSet(value) => {
            out.byte(49);
            out.u32(*value);
        }
        Return => out.byte(24),
    }
}

fn read_instruction(reader: &mut Reader<'_>) -> Result<Instruction, String> {
    Ok(match reader.byte()? {
        0 => Instruction::Constant(reader.u32()?),
        1 => Instruction::Nil,
        2 => Instruction::True,
        3 => Instruction::False,
        4 => Instruction::LoadLocal(reader.u16()?),
        5 => Instruction::StoreLocal(reader.u16()?),
        6 => Instruction::Pop,
        7 => {
            return Err(
                "bytecode artifact uses retired Primitive opcode 7; rebuild required".into(),
            )
        }
        8 => Instruction::Jump(reader.u32()?),
        9 => Instruction::JumpIfFalse(reader.u32()?),
        10 => Instruction::Closure {
            prototype: reader.u16()?,
            captures: reader.byte()?,
        },
        11 => Instruction::Call {
            argc: reader.byte()?,
        },
        12 => Instruction::CallStatic {
            prototype: reader.u16()?,
            argc: reader.byte()?,
        },
        13 => Instruction::Throw,
        14 => Instruction::Rethrow,
        15 => Instruction::GetGlobal(reader.u32()?),
        16 => Instruction::DefGlobal {
            name: reader.u32()?,
            metadata: reader.option_u16()?,
        },
        17 => Instruction::SetGlobal(reader.u32()?),
        18 => Instruction::VarGlobal(reader.u32()?),
        19 => Instruction::DeclareGlobal(reader.u32()?),
        20 => Instruction::DefStruct {
            name: reader.u32()?,
            fields: reader.u32()?,
        },
        21 => {
            return Err(
                "bytecode artifact uses retired StructField opcode 21; rebuild required".into(),
            )
        }
        22 => Instruction::InstanceOf,
        23 => Instruction::MakeMultiArity {
            name: reader.u32()?,
            count: reader.byte()?,
        },
        24 => Instruction::Return,
        25 => {
            return Err(
                "bytecode artifact uses retired PrimitiveLocalConst opcode 25; rebuild required"
                    .into(),
            )
        }
        26 => Instruction::Await,
        27 => Instruction::HostCall,
        28 => Instruction::Dup,
        29 => Instruction::BuildVector(reader.u16()?),
        30 => Instruction::BuildMap(reader.u16()?),
        31 => Instruction::BuildSet(reader.u16()?),
        32 => Instruction::DefMacro {
            name: reader.u32()?,
            metadata: reader.option_u16()?,
        },
        33 => Instruction::BuildList(reader.u16()?),
        34 => Instruction::ConcatList(reader.u16()?),
        35 => Instruction::ToVector,
        37 => {
            return Err(
                "bytecode artifact uses retired PrimitiveValue opcode 37; rebuild required".into(),
            )
        }
        38 => Instruction::BuiltinValue(reader.u32()?),
        39 => Instruction::DynamicBind(reader.u32()?),
        40 => Instruction::DynamicUnbind(reader.u32()?),
        41 => Instruction::DefProtocol(reader.u32()?),
        42 => Instruction::ExtendType(reader.u32()?),
        43 => Instruction::DefMulti(reader.u32()?),
        44 => Instruction::DefMethod(reader.u32()?),
        45 => Instruction::DotCall {
            method: reader.u32()?,
            argc: reader.byte()?,
        },
        46 => Instruction::Yield,
        47 => Instruction::DefMutable {
            name: reader.u32()?,
            fields: reader.u32()?,
        },
        48 => Instruction::MutableFieldGet(reader.u32()?),
        49 => Instruction::MutableFieldSet(reader.u32()?),
        50 => Instruction::IntrinsicCall {
            target: reader.u32()?,
            argc: reader.byte()?,
        },
        51 => Instruction::IntrinsicValue(reader.u32()?),
        52 => Instruction::ProtocolCall {
            target: reader.u32()?,
            argc: reader.byte()?,
        },
        53 => Instruction::NamespaceValue(reader.u32()?),
        54 => Instruction::NamespaceOperation(reader.u32()?),
        _ => return Err("bytecode artifact contains an unknown opcode".into()),
    })
}

fn write_metadata(out: &mut Writer, metadata: &Metadata) -> Result<(), String> {
    out.len(metadata.entries().len())?;
    for (key, value) in metadata.entries() {
        write_metadata_value(out, key)?;
        write_metadata_value(out, value)?;
    }
    Ok(())
}

fn read_metadata(reader: &mut Reader<'_>) -> Result<Rc<Metadata>, String> {
    let entries =
        reader.many(|reader| Ok((read_metadata_value(reader)?, read_metadata_value(reader)?)))?;
    Ok(Metadata::new(entries))
}

fn write_metadata_value(out: &mut Writer, value: &MetadataValue) -> Result<(), String> {
    use MetadataValue::*;
    match value {
        Nil => out.byte(0),
        Boolean(v) => {
            out.byte(1);
            out.byte(u8::from(*v));
        }
        Number(v) => {
            out.byte(2);
            out.i64(*v);
        }
        Float(v) => {
            if !v.is_finite() {
                return Err("non-finite number".into());
            }
            out.byte(3);
            out.u64(v.to_bits());
        }
        BigInteger(v) => {
            if let Some(value) = v.to_i64() {
                out.byte(2);
                out.i64(value);
            } else {
                out.byte(4);
                out.string(&v.to_string())?;
            }
        }
        Character(v) => {
            out.byte(6);
            out.u32(*v as u32);
        }
        Regex(v) => {
            out.byte(7);
            out.string(v)?;
        }
        Tagged(tag, value) => {
            out.byte(8);
            out.string(tag)?;
            write_metadata_value(out, value)?;
        }
        String(v) => {
            out.byte(9);
            out.string(v)?;
        }
        Keyword(v) => {
            out.byte(10);
            out.string(v.as_str())?;
        }
        Symbol(v) => {
            out.byte(11);
            out.string(v.as_str())?;
        }
        Vector(values) => {
            out.byte(12);
            write_metadata_values(out, values)?;
        }
        List(values) => {
            out.byte(13);
            write_metadata_values(out, values)?;
        }
        Set(values) => {
            out.byte(14);
            write_metadata_values(out, values)?;
        }
        Map(values) => {
            out.byte(15);
            out.len(values.len())?;
            for (k, v) in values {
                write_metadata_value(out, k)?;
                write_metadata_value(out, v)?;
            }
        }
    }
    Ok(())
}

fn write_metadata_values(out: &mut Writer, values: &[MetadataValue]) -> Result<(), String> {
    out.len(values.len())?;
    for value in values {
        write_metadata_value(out, value)?;
    }
    Ok(())
}

fn read_metadata_value(reader: &mut Reader<'_>) -> Result<MetadataValue, String> {
    Ok(match reader.byte()? {
        0 => MetadataValue::Nil,
        1 => MetadataValue::Boolean(reader.boolean()?),
        2 => MetadataValue::Number(reader.i64()?),
        3 => {
            let value = f64::from_bits(reader.u64()?);
            if !value.is_finite() {
                return Err("non-finite number".into());
            }
            MetadataValue::Float(value)
        }
        4 => {
            let value = BigInt::parse_bytes(reader.string()?.as_bytes(), 10)
                .ok_or("invalid metadata big integer")?;
            value
                .to_i64()
                .map(MetadataValue::Number)
                .unwrap_or(MetadataValue::BigInteger(value))
        }
        5 => return Err("unsupported metadata tag: decimal".into()),
        6 => MetadataValue::Character(
            char::from_u32(reader.u32()?).ok_or("invalid metadata character")?,
        ),
        7 => MetadataValue::Regex(reader.string()?),
        8 => MetadataValue::Tagged(reader.string()?, Box::new(read_metadata_value(reader)?)),
        9 => MetadataValue::String(reader.string()?),
        10 => MetadataValue::Keyword(Keyword::from(reader.string()?)),
        11 => MetadataValue::Symbol(Symbol::from(reader.string()?)),
        12 => MetadataValue::Vector(reader.many(read_metadata_value)?),
        13 => MetadataValue::List(reader.many(read_metadata_value)?),
        14 => MetadataValue::Set(reader.many(read_metadata_value)?),
        15 => MetadataValue::Map(
            reader.many(|r| Ok((read_metadata_value(r)?, read_metadata_value(r)?)))?,
        ),
        _ => return Err("bytecode artifact contains unknown metadata".into()),
    })
}

fn write_schema_map(out: &mut Writer, schemas: &HashMap<String, SchemaType>) -> Result<(), String> {
    let mut names = schemas.keys().collect::<Vec<_>>();
    names.sort();
    out.len(names.len())?;
    for name in names {
        out.string(name)?;
        write_schema_type(out, &schemas[name])?;
    }
    Ok(())
}

fn read_schema_map(reader: &mut Reader<'_>) -> Result<HashMap<String, SchemaType>, String> {
    let entries = reader.many(|reader| Ok((reader.string()?, read_schema_type(reader)?)))?;
    let mut schemas = HashMap::with_capacity(entries.len());
    for (name, schema) in entries {
        if schemas.insert(name.clone(), schema).is_some() {
            return Err(format!(
                "bytecode artifact contains duplicate schema {name}"
            ));
        }
    }
    Ok(schemas)
}

fn write_schema_type(out: &mut Writer, schema: &SchemaType) -> Result<(), String> {
    match schema {
        SchemaType::Primitive(name) => {
            out.byte(0);
            out.string(name)?;
        }
        SchemaType::Reference(name) => {
            out.byte(1);
            out.string(name)?;
        }
        SchemaType::Union(types) => {
            out.byte(2);
            write_schema_types(out, types)?;
        }
        SchemaType::Vector(item) => {
            out.byte(3);
            write_schema_type(out, item)?;
        }
        SchemaType::Set(item) => {
            out.byte(10);
            write_schema_type(out, item)?;
        }
        SchemaType::Tuple(items) => {
            out.byte(4);
            write_schema_types(out, items)?;
        }
        SchemaType::Map(fields) => {
            // Keep tag 5 byte-for-byte compatible with existing artifacts.
            // Property-aware fields use tag 12 so older schema maps remain readable.
            let property_aware = fields.iter().any(|field| field.properties.is_some());
            out.byte(if property_aware { 12 } else { 5 });
            out.len(fields.len())?;
            for field in fields {
                write_schema_form(out, &field.name)?;
                if property_aware {
                    match &field.properties {
                        Some(properties) => {
                            out.byte(1);
                            write_schema_form(out, properties)?;
                        }
                        None => out.byte(0),
                    }
                }
                write_schema_type(out, &field.value_type)?;
            }
        }
        SchemaType::Struct {
            name,
            mutable,
            fields,
        } => {
            out.byte(13);
            out.string(name)?;
            out.byte(u8::from(*mutable));
            let property_aware = fields.iter().any(|field| field.properties.is_some());
            out.byte(u8::from(property_aware));
            out.len(fields.len())?;
            for field in fields {
                write_schema_form(out, &field.name)?;
                if property_aware {
                    match &field.properties {
                        Some(properties) => {
                            out.byte(1);
                            write_schema_form(out, properties)?;
                        }
                        None => out.byte(0),
                    }
                }
                write_schema_type(out, &field.value_type)?;
            }
        }
        SchemaType::WithProperties { schema, properties } => {
            out.byte(11);
            write_schema_type(out, schema)?;
            write_schema_form(out, properties)?;
        }
        SchemaType::Function(arities) => {
            out.byte(6);
            out.len(arities.len())?;
            for arity in arities {
                write_schema_types(out, &arity.fixed)?;
                match &arity.rest {
                    Some(rest) => {
                        out.byte(1);
                        write_schema_type(out, rest)?;
                    }
                    None => out.byte(0),
                }
                write_schema_type(out, &arity.output)?;
            }
        }
        SchemaType::Enum(values) => {
            out.byte(7);
            write_schema_forms(out, values)?;
        }
        SchemaType::Extension { head, arguments } => {
            out.byte(8);
            out.string(head)?;
            write_schema_forms(out, arguments)?;
        }
        SchemaType::Unknown(surface) => {
            out.byte(9);
            write_schema_form(out, surface)?;
        }
    }
    Ok(())
}

fn read_schema_type(reader: &mut Reader<'_>) -> Result<SchemaType, String> {
    Ok(match reader.byte()? {
        0 => SchemaType::Primitive(reader.string()?),
        1 => SchemaType::Reference(reader.string()?),
        2 => SchemaType::Union(reader.many(read_schema_type)?),
        3 => SchemaType::Vector(Box::new(read_schema_type(reader)?)),
        4 => SchemaType::Tuple(reader.many(read_schema_type)?),
        5 => SchemaType::Map(reader.many(|reader| {
            Ok(SchemaField {
                name: read_schema_form(reader)?,
                properties: None,
                value_type: read_schema_type(reader)?,
            })
        })?),
        6 => SchemaType::Function(reader.many(|reader| {
            let fixed = reader.many(read_schema_type)?;
            let rest = if reader.boolean()? {
                Some(Box::new(read_schema_type(reader)?))
            } else {
                None
            };
            Ok(FunctionSchema {
                fixed,
                rest,
                output: Box::new(read_schema_type(reader)?),
            })
        })?),
        7 => SchemaType::Enum(read_schema_forms(reader)?),
        8 => SchemaType::Extension {
            head: reader.string()?,
            arguments: read_schema_forms(reader)?,
        },
        9 => SchemaType::Unknown(read_schema_form(reader)?),
        10 => SchemaType::Set(Box::new(read_schema_type(reader)?)),
        11 => SchemaType::WithProperties {
            schema: Box::new(read_schema_type(reader)?),
            properties: read_schema_form(reader)?,
        },
        12 => SchemaType::Map(reader.many(|reader| {
            let name = read_schema_form(reader)?;
            let properties = if reader.boolean()? {
                Some(read_schema_form(reader)?)
            } else {
                None
            };
            Ok(SchemaField {
                name,
                properties,
                value_type: read_schema_type(reader)?,
            })
        })?),
        13 => {
            let name = reader.string()?;
            let mutable = reader.boolean()?;
            let property_aware = reader.boolean()?;
            SchemaType::Struct {
                name,
                mutable,
                fields: reader.many(|reader| {
                    let name = read_schema_form(reader)?;
                    let properties = if property_aware {
                        if reader.boolean()? {
                            Some(read_schema_form(reader)?)
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                    Ok(SchemaField {
                        name,
                        properties,
                        value_type: read_schema_type(reader)?,
                    })
                })?,
            }
        }
        _ => return Err("bytecode artifact contains unknown schema type".into()),
    })
}

fn write_schema_types(out: &mut Writer, types: &[SchemaType]) -> Result<(), String> {
    out.len(types.len())?;
    for schema in types {
        write_schema_type(out, schema)?;
    }
    Ok(())
}

fn write_schema_forms(out: &mut Writer, forms: &[crate::kernel::Form]) -> Result<(), String> {
    out.len(forms.len())?;
    for form in forms {
        write_schema_form(out, form)?;
    }
    Ok(())
}

fn read_schema_forms(reader: &mut Reader<'_>) -> Result<Vec<crate::kernel::Form>, String> {
    reader.many(read_schema_form)
}

fn write_schema_form(out: &mut Writer, form: &crate::kernel::Form) -> Result<(), String> {
    out.string(&form.to_string())
}

fn read_schema_form(reader: &mut Reader<'_>) -> Result<crate::kernel::Form, String> {
    crate::kernel::parse(&reader.string()?)
        .map_err(|error| format!("bytecode artifact contains invalid schema form: {error}"))
}

#[derive(Default)]
struct Writer {
    bytes: Vec<u8>,
}
impl Writer {
    fn byte(&mut self, value: u8) {
        self.bytes.push(value);
    }
    fn u16(&mut self, value: u16) {
        self.bytes.extend_from_slice(&value.to_be_bytes());
    }
    fn u32(&mut self, value: u32) {
        self.bytes.extend_from_slice(&value.to_be_bytes());
    }
    fn u64(&mut self, value: u64) {
        self.bytes.extend_from_slice(&value.to_be_bytes());
    }
    fn i64(&mut self, value: i64) {
        self.bytes.extend_from_slice(&value.to_be_bytes());
    }
    fn usize32(&mut self, value: usize) -> Result<(), String> {
        self.u32(u32::try_from(value).map_err(|_| "bytecode field is too large")?);
        Ok(())
    }
    fn len(&mut self, value: usize) -> Result<(), String> {
        self.usize32(value)
    }
    fn bytes(&mut self, value: &[u8]) -> Result<(), String> {
        self.len(value.len())?;
        self.bytes.extend_from_slice(value);
        Ok(())
    }
    fn string(&mut self, value: &str) -> Result<(), String> {
        self.bytes(value.as_bytes())
    }
    fn option_string(&mut self, value: Option<&str>) -> Result<(), String> {
        match value {
            Some(v) => {
                self.byte(1);
                self.string(v)?;
            }
            None => self.byte(0),
        };
        Ok(())
    }
    fn option_u16(&mut self, value: Option<u16>) {
        match value {
            Some(v) => {
                self.byte(1);
                self.u16(v);
            }
            None => self.byte(0),
        }
    }
    fn option_u32(&mut self, value: Option<u32>) {
        match value {
            Some(v) => {
                self.byte(1);
                self.u32(v);
            }
            None => self.byte(0),
        }
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    cursor: usize,
}
impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, cursor: 0 }
    }
    fn take(&mut self, size: usize) -> Result<&'a [u8], String> {
        let end = self
            .cursor
            .checked_add(size)
            .ok_or("bytecode artifact length overflow")?;
        if end > self.bytes.len() {
            return Err("bytecode artifact is truncated".into());
        }
        let value = &self.bytes[self.cursor..end];
        self.cursor = end;
        Ok(value)
    }
    fn byte(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn boolean(&mut self) -> Result<bool, String> {
        match self.byte()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err("bytecode artifact contains invalid boolean".into()),
        }
    }
    fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn i64(&mut self) -> Result<i64, String> {
        Ok(i64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn bytes(&mut self) -> Result<&'a [u8], String> {
        let size = self.u32()? as usize;
        self.take(size)
    }
    fn string(&mut self) -> Result<String, String> {
        String::from_utf8(self.bytes()?.to_vec())
            .map_err(|_| "bytecode artifact contains invalid UTF-8".into())
    }
    fn option_string(&mut self) -> Result<Option<String>, String> {
        if self.boolean()? {
            Ok(Some(self.string()?))
        } else {
            Ok(None)
        }
    }
    fn option_u16(&mut self) -> Result<Option<u16>, String> {
        if self.boolean()? {
            Ok(Some(self.u16()?))
        } else {
            Ok(None)
        }
    }
    fn option_u32(&mut self) -> Result<Option<u32>, String> {
        if self.boolean()? {
            Ok(Some(self.u32()?))
        } else {
            Ok(None)
        }
    }
    fn many<T>(
        &mut self,
        mut read: impl FnMut(&mut Reader<'a>) -> Result<T, String>,
    ) -> Result<Vec<T>, String> {
        let size = self.u32()? as usize;
        let mut values = Vec::with_capacity(size.min(4096));
        for _ in 0..size {
            values.push(read(self)?);
        }
        Ok(values)
    }
    fn finish(&self) -> Result<(), String> {
        if self.cursor == self.bytes.len() {
            Ok(())
        } else {
            Err("bytecode artifact has trailing payload bytes".into())
        }
    }
}

#[cfg(test)]
#[path = "artifact/tests.rs"]
mod tests;
