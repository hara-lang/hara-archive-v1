use sha2::{Digest, Sha256};
use wasm_encoder::Module;

use crate::vm::{decode_program, encode_program, FunctionId, Instruction, Program};

use super::bridge::{self, TargetDescriptor, TargetKind};
use super::codegen::compile_program;
use super::ir::lower_function;

const MAGIC: &[u8; 4] = b"HNW0";
pub const HNW_ABI_VERSION: u16 = 0;

#[derive(Debug, Clone)]
pub struct NativeArtifact {
    pub abi_version: u16,
    pub program: Program,
    pub wasm: Vec<u8>,
    pub functions: Vec<(FunctionId, u16)>,
    pub capabilities: Vec<bool>,
    pub targets: Vec<TargetDescriptor>,
    pub operation_registry_digest: [u8; 32],
}

pub fn compile_artifact(program: &Program) -> Result<Vec<u8>, String> {
    let hbc = encode_program(program)?;
    let mut capabilities = native_function_capabilities(program);
    let wasm = match compile_program(program) {
        Ok(wasm) => wasm,
        Err(_) => {
            let mut native_program = program.clone();
            for (id, native) in capabilities.iter().enumerate() {
                if !native {
                    replace_with_fallback_stub(&mut native_program.functions[id]);
                }
            }
            match compile_program(&native_program) {
                Ok(wasm) => wasm,
                Err(_) => {
                    // An empty module is still a valid HNW0 container, but it
                    // must not advertise native entry points that the module
                    // does not contain. Every function therefore remains on
                    // the retained HBC path when the sanitized module cannot
                    // be emitted.
                    capabilities.fill(false);
                    Module::new().finish()
                }
            }
        }
    };
    let mut payload = Vec::new();
    put_u16(&mut payload, HNW_ABI_VERSION);
    put_u16(
        &mut payload,
        u16::try_from(program.functions.len()).map_err(|_| "too many HNW0 functions")?,
    );
    for (id, function) in program.functions.iter().enumerate() {
        put_u16(&mut payload, id as u16);
        put_u16(&mut payload, function.arity);
    }
    for native in &capabilities {
        payload.push(u8::from(*native));
    }
    let targets = bridge::target_table();
    bridge::validate_target_table(&targets)?;
    put_u16(
        &mut payload,
        u16::try_from(targets.len()).map_err(|_| "too many HNW0 targets")?,
    );
    for target in &targets {
        put_u16(&mut payload, target.id);
        payload.push(target.kind.wire());
        put_u16(&mut payload, target.arity.unwrap_or(u16::MAX));
        let symbol = target.symbol.as_bytes();
        put_u16(
            &mut payload,
            u16::try_from(symbol.len()).map_err(|_| "HNW0 target symbol is too long")?,
        );
        payload.extend_from_slice(symbol);
    }
    payload.extend_from_slice(&bridge::operation_registry_digest());
    put_bytes(&mut payload, &hbc)?;
    put_bytes(&mut payload, &wasm)?;
    let digest = Sha256::digest(&payload);
    let mut output = MAGIC.to_vec();
    put_u32(
        &mut output,
        u32::try_from(payload.len()).map_err(|_| "HNW0 artifact is too large")?,
    );
    output.extend_from_slice(&payload);
    output.extend_from_slice(&digest);
    Ok(output)
}

pub fn decode_artifact(bytes: &[u8]) -> Result<NativeArtifact, String> {
    if !bytes.starts_with(MAGIC) {
        return Err("native artifact has invalid magic".into());
    }
    if bytes.len() < 8 + 32 {
        return Err("native artifact is truncated".into());
    }
    let length = u32::from_be_bytes(bytes[4..8].try_into().unwrap()) as usize;
    let end = 8usize
        .checked_add(length)
        .ok_or("native artifact length overflow")?;
    if end.checked_add(32) != Some(bytes.len()) {
        return Err("native artifact length mismatch".into());
    }
    let payload = &bytes[8..end];
    if Sha256::digest(payload).as_slice() != &bytes[end..] {
        return Err("native artifact checksum mismatch".into());
    }
    let mut reader = Reader {
        bytes: payload,
        offset: 0,
    };
    let abi_version = reader.u16()?;
    if abi_version != HNW_ABI_VERSION {
        return Err(format!("unsupported HNW ABI version {abi_version}"));
    }
    let count = usize::from(reader.u16()?);
    let mut functions = Vec::with_capacity(count);
    for expected in 0..count {
        let id = reader.u16()?;
        let arity = reader.u16()?;
        if usize::from(id) != expected {
            return Err("native artifact function table is not canonical".into());
        }
        functions.push((id, arity));
    }
    let capabilities = reader
        .take(count)?
        .iter()
        .map(|native| match native {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err("native artifact capability table is not canonical".into()),
        })
        .collect::<Result<Vec<_>, String>>()?;
    let target_count = usize::from(reader.u16()?);
    let mut targets = Vec::with_capacity(target_count);
    for _ in 0..target_count {
        let id = reader.u16()?;
        let kind = TargetKind::from_wire(reader.take(1)?[0])
            .ok_or("native artifact target kind is invalid")?;
        let arity = match reader.u16()? {
            u16::MAX => None,
            value => Some(value),
        };
        let symbol_length = usize::from(reader.u16()?);
        let symbol = String::from_utf8(reader.take(symbol_length)?.to_vec())
            .map_err(|_| "native artifact target symbol is not UTF-8")?;
        targets.push(TargetDescriptor {
            id,
            symbol,
            kind,
            arity,
        });
    }
    bridge::validate_target_table(&targets)?;
    let operation_registry_digest: [u8; 32] = reader
        .take(32)?
        .try_into()
        .expect("operation registry digest has fixed size");
    if operation_registry_digest != bridge::operation_registry_digest() {
        return Err("native artifact operation registry digest mismatch".into());
    }
    let program = decode_program(reader.bytes()?)?;
    let wasm = reader.bytes()?.to_vec();
    reader.finish()?;
    if wasm.get(..4) != Some(b"\0asm") {
        return Err("native artifact contains invalid Wasm".into());
    }
    if program.functions.len() != functions.len()
        || program
            .functions
            .iter()
            .zip(&functions)
            .any(|(function, (_, arity))| function.arity != *arity)
    {
        return Err("native artifact function metadata mismatch".into());
    }
    Ok(NativeArtifact {
        abi_version,
        program,
        wasm,
        functions,
        capabilities,
        targets,
        operation_registry_digest,
    })
}

/// Returns the native eligibility of each validated HBC function. The result
/// is intentionally per function: callers can keep using native functions
/// while unsupported functions execute through the retained HBC program.
pub(crate) fn native_function_capabilities(program: &Program) -> Vec<bool> {
    let mut capabilities = program
        .functions
        .iter()
        .enumerate()
        .map(|(id, function)| {
            !has_unrepresentable_integer_constant(program, function)
                && lower_function(program, id as FunctionId, function).is_ok()
        })
        .collect::<Vec<_>>();
    loop {
        let previous = capabilities.clone();
        for (id, function) in program.functions.iter().enumerate() {
            if !previous[id] {
                continue;
            }
            let depends_on_unsupported =
                function.code.iter().any(|instruction| match instruction {
                    Instruction::CallStatic { prototype, .. }
                    | Instruction::Closure { prototype, .. } => previous
                        .get(usize::from(*prototype))
                        .is_some_and(|native| !native),
                    _ => false,
                });
            if depends_on_unsupported {
                capabilities[id] = false;
            }
        }
        if capabilities == previous {
            return capabilities;
        }
    }
}

fn has_unrepresentable_integer_constant(
    program: &Program,
    function: &crate::vm::FunctionPrototype,
) -> bool {
    function.code.iter().any(|instruction| {
        let Instruction::Constant(index) = instruction else {
            return false;
        };
        program
            .constants
            .get(*index as usize)
            .is_some_and(crate::numeric::is_big_integer_value)
    })
}

fn replace_with_fallback_stub(function: &mut crate::vm::FunctionPrototype) {
    function.async_function = false;
    function.variadic = false;
    function.capture_count = 0;
    function.max_stack = 1;
    function.handlers.clear();
    function.code = vec![Instruction::Nil, Instruction::Return];
}

fn put_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_be_bytes());
}
fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_be_bytes());
}
fn put_bytes(out: &mut Vec<u8>, bytes: &[u8]) -> Result<(), String> {
    put_u32(
        out,
        u32::try_from(bytes.len()).map_err(|_| "HNW0 section is too large")?,
    );
    out.extend_from_slice(bytes);
    Ok(())
}

struct Reader<'a> {
    bytes: &'a [u8],
    offset: usize,
}
impl<'a> Reader<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8], String> {
        let end = self
            .offset
            .checked_add(count)
            .ok_or("native artifact offset overflow")?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or("native artifact is truncated")?;
        self.offset = end;
        Ok(value)
    }
    fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn bytes(&mut self) -> Result<&'a [u8], String> {
        let n = self.u32()? as usize;
        self.take(n)
    }
    fn finish(self) -> Result<(), String> {
        if self.offset == self.bytes.len() {
            Ok(())
        } else {
            Err("native artifact has trailing payload".into())
        }
    }
}
