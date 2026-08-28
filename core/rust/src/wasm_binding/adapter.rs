#![cfg(not(target_arch = "wasm32"))]

use sha2::{Digest, Sha256};
use wasm_encoder::{
    ConstExpr, EntityType, ExportKind, ExportSection, Function, FunctionSection, GlobalSection,
    GlobalType, ImportSection, Instruction, MemArg, MemorySection, MemoryType, Module, TypeSection,
    ValType,
};

use crate::kernel::Form;

use super::{inspect_direct, BindingFunction, HaraValueType, WasmInterface, WasmValueType};

pub const ADAPTER_MANIFEST_SCHEMA: &str = "hara.wasm-adapter/0-alpha";
const ADAPTER_TARGET: &str = "core.v1-forward";
const HTA_ADAPTER_TARGET: &str = "hta.v1";
const LIBRARY_IMPORT_MODULE: &str = "hara/library";

/// A deterministic adapter module and the manifest describing its composition.
///
/// The first adapter revision is deliberately a scalar forwarding boundary:
/// the adapter imports the verified library exports under one stable module
/// name and exports the Hara-facing names. Rich memory and HTA lifecycle
/// operations remain explicit follow-up revisions rather than guessed from
/// machine-level values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterArtifact {
    pub bytes: Vec<u8>,
    pub manifest: String,
    pub module_digest: String,
    pub interface_digest: String,
    pub adapter_digest: String,
}

/// Generate the portable scalar adapter for a verified library/interface pair.
///
/// Inspection only parses the module bytes. It never instantiates the wrapped
/// library or runs its start function.
pub fn generate_adapter(
    module_bytes: &[u8],
    interface: &WasmInterface,
) -> Result<AdapterArtifact, String> {
    if interface.exports.iter().any(|export| export.asynchronous) {
        return Err(
            "wasm-adapter/feature-unsupported: asynchronous exports require the HTA adapter".into(),
        );
    }
    let inspection = inspect_direct(module_bytes)?;
    if inspection.start.is_some() {
        return Err("wasm-adapter/start-denied: wrapped module declares a start function".into());
    }
    interface.verify_direct(&inspection)?;
    let exports = ordered_exports(interface)?;

    let bytes = emit_forwarder(&exports)?;
    let module_digest = digest(module_bytes);
    let interface_digest = interface.digest();
    let adapter_digest = digest(&bytes);
    let manifest = adapter_manifest(
        interface,
        &module_digest,
        &interface_digest,
        &adapter_digest,
        &exports,
    );

    Ok(AdapterArtifact {
        bytes,
        manifest,
        module_digest,
        interface_digest,
        adapter_digest,
    })
}

/// Generate the HTA package adapter for scalar bindings.
///
/// The adapter owns the HTA task/event boundary and imports only the verified
/// library functions. Memory and handle lowering remain separate binding
/// revisions; silently treating those values as scalars would violate the
/// interface ownership contract.
pub fn generate_hta_adapter(
    module_bytes: &[u8],
    interface: &WasmInterface,
) -> Result<AdapterArtifact, String> {
    let inspection = inspect_direct(module_bytes)?;
    if inspection.start.is_some() {
        return Err("wasm-adapter/start-denied: wrapped module declares a start function".into());
    }
    verify_hta_scalar(interface, &inspection)?;
    let exports = ordered_exports(interface)?;
    let bytes = emit_hta_forwarder(&exports)?;
    let module_digest = digest(module_bytes);
    let interface_digest = interface.digest();
    let adapter_digest = digest(&bytes);
    let manifest = hta_adapter_manifest(
        interface,
        &module_digest,
        &interface_digest,
        &adapter_digest,
        &exports,
    );

    Ok(AdapterArtifact {
        bytes,
        manifest,
        module_digest,
        interface_digest,
        adapter_digest,
    })
}

fn ordered_exports(interface: &WasmInterface) -> Result<Vec<BindingFunction>, String> {
    let mut exports = interface.exports.clone();
    exports.sort_by(|left, right| left.name.cmp(&right.name));
    for pair in exports.windows(2) {
        if pair[0].name == pair[1].name {
            return Err(format!(
                "wasm-adapter/export-ambiguous: duplicate Hara export {}",
                pair[0].name
            ));
        }
    }
    Ok(exports)
}

pub fn verify_hta_scalar(
    interface: &WasmInterface,
    inspection: &super::DirectWasmInspection,
) -> Result<(), String> {
    if interface.memory.is_some() {
        return Err(
            "wasm-adapter/feature-unsupported: memory lowering requires a later HTA revision"
                .into(),
        );
    }
    if !interface.capabilities.is_empty()
        || interface
            .exports
            .iter()
            .any(|export| !export.capabilities.is_empty())
    {
        return Err(
            "wasm-adapter/capability-denied: scalar adapters cannot require host capabilities"
                .into(),
        );
    }
    if interface.exports.iter().any(|export| {
        export.errors.is_some()
            || export
                .arguments
                .iter()
                .any(|argument| argument.hara_type.direct_wasm_type().is_none())
            || interface
                .exports
                .iter()
                .any(|candidate| candidate.returns.hara_type.direct_wasm_type().is_none())
    }) {
        return Err(
            "wasm-adapter/feature-unsupported: non-scalar and error mappings require a later HTA revision"
                .into(),
        );
    }
    let discovered = inspection
        .direct_exports()
        .map_err(|error| format!("wasm-adapter/module-incompatible: {error}"))?
        .into_iter()
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut raw_names = std::collections::BTreeSet::new();
    for export in &interface.exports {
        if !raw_names.insert(export.wasm_export.as_str()) {
            return Err(format!(
                "wasm-adapter/export-ambiguous: multiple Hara exports map to {}",
                export.wasm_export
            ));
        }
        let found = discovered.get(&export.wasm_export).ok_or_else(|| {
            format!(
                "wasm-adapter/export-missing: {} maps to absent Wasm export {}",
                export.name, export.wasm_export
            )
        })?;
        let expected = crate::extension::ExtensionExport {
            arguments: export
                .arguments
                .iter()
                .map(|argument| argument.wasm_type.as_keyword().to_owned())
                .collect(),
            returns: export.returns.wasm_type.as_keyword().to_owned(),
            asynchronous: false,
            raw_export: None,
        };
        if found != &expected {
            return Err(format!(
                "wasm-adapter/signature-mismatch: {} -> {} expected {:?}, found {:?}",
                export.name, export.wasm_export, expected, found
            ));
        }
    }
    Ok(())
}

fn emit_forwarder(exports: &[BindingFunction]) -> Result<Vec<u8>, String> {
    let mut module = Module::new();
    let mut types = TypeSection::new();

    for export in exports {
        types.function(
            export
                .arguments
                .iter()
                .map(|argument| val_type(argument.wasm_type)),
            result_types(export.returns.wasm_type),
        );
    }
    module.section(&types);

    let mut imports = ImportSection::new();
    for (index, export) in exports.iter().enumerate() {
        imports.import(
            LIBRARY_IMPORT_MODULE,
            &export.wasm_export,
            EntityType::Function(index as u32),
        );
    }
    module.section(&imports);

    let mut functions = FunctionSection::new();
    for index in 0..exports.len() {
        functions.function(index as u32);
    }
    module.section(&functions);

    let mut exports_section = ExportSection::new();
    let import_count = exports.len() as u32;
    for (index, export) in exports.iter().enumerate() {
        exports_section.export(&export.name, ExportKind::Func, import_count + index as u32);
    }
    module.section(&exports_section);

    let mut code = wasm_encoder::CodeSection::new();
    for (index, export) in exports.iter().enumerate() {
        let mut function = Function::new([]);
        for argument in 0..export.arguments.len() {
            function.instruction(&Instruction::LocalGet(argument as u32));
        }
        function.instruction(&Instruction::Call(index as u32));
        function.instruction(&Instruction::End);
        code.function(&function);
    }
    module.section(&code);
    Ok(module.finish())
}

fn emit_hta_forwarder(exports: &[BindingFunction]) -> Result<Vec<u8>, String> {
    let mut module = Module::new();
    let mut types = TypeSection::new();
    for export in exports {
        types.function(
            export
                .arguments
                .iter()
                .map(|argument| val_type(argument.wasm_type)),
            result_types(export.returns.wasm_type),
        );
    }
    let import_type_count = exports.len() as u32;
    let lifecycle_types = [
        ([ValType::I32].as_slice(), [ValType::I32].as_slice()),
        ([ValType::I32, ValType::I32].as_slice(), [].as_slice()),
        ([].as_slice(), [ValType::I32].as_slice()),
        (
            [ValType::I32, ValType::I32].as_slice(),
            [ValType::I64].as_slice(),
        ),
        ([].as_slice(), [ValType::I64].as_slice()),
        (
            [ValType::I32, ValType::I32].as_slice(),
            [ValType::I32].as_slice(),
        ),
        ([ValType::I64].as_slice(), [ValType::I32].as_slice()),
        ([ValType::I64].as_slice(), [ValType::I32].as_slice()),
        (
            [ValType::I32, ValType::I32].as_slice(),
            [ValType::I32].as_slice(),
        ),
    ];
    for (arguments, results) in lifecycle_types {
        types.function(arguments.iter().copied(), results.iter().copied());
    }
    module.section(&types);

    let mut imports = ImportSection::new();
    for (index, export) in exports.iter().enumerate() {
        imports.import(
            LIBRARY_IMPORT_MODULE,
            &export.wasm_export,
            EntityType::Function(index as u32),
        );
    }
    module.section(&imports);

    let alloc = import_type_count;
    let dealloc = alloc + 1;
    let abi_version = alloc + 2;
    let start = alloc + 3;
    let next_event = alloc + 4;
    let deliver = alloc + 5;
    let cancel = alloc + 6;
    let drop_task = alloc + 7;
    let release = alloc + 8;
    let mut functions = FunctionSection::new();
    for type_index in import_type_count..import_type_count + 9 {
        functions.function(type_index);
    }
    module.section(&functions);

    let mut memories = MemorySection::new();
    memories.memory(MemoryType {
        minimum: 2,
        maximum: Some(1024),
        memory64: false,
        shared: false,
    });
    module.section(&memories);

    let mut globals = GlobalSection::new();
    globals.global(
        GlobalType {
            val_type: ValType::I32,
            mutable: true,
        },
        &ConstExpr::i32_const(1024),
    );
    for value in [0, 0, 0, 1, 0] {
        globals.global(
            GlobalType {
                val_type: ValType::I32,
                mutable: true,
            },
            &ConstExpr::i32_const(value),
        );
    }
    module.section(&globals);

    let mut exports_section = ExportSection::new();
    for (name, index) in [
        ("hta_alloc", alloc),
        ("hta_dealloc", dealloc),
        ("hta_abi_version", abi_version),
        ("hta_start", start),
        ("hta_next_event", next_event),
        ("hta_deliver", deliver),
        ("hta_cancel", cancel),
        ("hta_drop_task", drop_task),
        ("hta_release", release),
    ] {
        exports_section.export(name, ExportKind::Func, index);
    }
    exports_section.export("memory", ExportKind::Memory, 0);
    module.section(&exports_section);

    let mut code = wasm_encoder::CodeSection::new();
    code.function(&emit_alloc());
    code.function(&emit_noop(&[ValType::I32, ValType::I32], &[]));
    code.function(&emit_abi_version());
    code.function(&emit_start(exports, alloc));
    code.function(&emit_next_event());
    code.function(&emit_noop(&[ValType::I32, ValType::I32], &[ValType::I32]));
    code.function(&emit_noop(&[ValType::I64], &[ValType::I32]));
    code.function(&emit_noop(&[ValType::I64], &[ValType::I32]));
    code.function(&emit_noop(&[ValType::I32, ValType::I32], &[ValType::I32]));
    module.section(&code);
    Ok(module.finish())
}

fn emit_alloc() -> Function {
    let mut function = Function::new([(1, ValType::I32)]);
    function.instruction(&Instruction::GlobalGet(0));
    function.instruction(&Instruction::LocalSet(1));
    function.instruction(&Instruction::GlobalGet(0));
    function.instruction(&Instruction::LocalGet(0));
    function.instruction(&Instruction::I32Add);
    function.instruction(&Instruction::GlobalSet(0));
    function.instruction(&Instruction::LocalGet(1));
    function.instruction(&Instruction::End);
    function
}

fn emit_noop(parameters: &[ValType], results: &[ValType]) -> Function {
    let mut function = Function::new([]);
    for (index, _) in parameters.iter().enumerate() {
        let _ = index;
    }
    if let Some(result) = results.first() {
        function.instruction(match result {
            ValType::I32 => &Instruction::I32Const(0),
            ValType::I64 => &Instruction::I64Const(0),
            _ => unreachable!("HTA lifecycle uses only integer results"),
        });
    }
    function.instruction(&Instruction::End);
    function
}

fn emit_abi_version() -> Function {
    let mut function = Function::new([]);
    function.instruction(&Instruction::I32Const(1));
    function.instruction(&Instruction::End);
    function
}

fn emit_start(exports: &[BindingFunction], alloc: u32) -> Function {
    let mut function = Function::new([(1, ValType::I32), (1, ValType::I64), (1, ValType::I64)]);
    function.instruction(&Instruction::I32Const(64));
    function.instruction(&Instruction::Call(alloc));
    function.instruction(&Instruction::LocalSet(2));
    function.instruction(&Instruction::I32Const(0));
    function.instruction(&Instruction::GlobalSet(1));
    function.instruction(&Instruction::I32Const(0));
    function.instruction(&Instruction::GlobalSet(2));
    function.instruction(&Instruction::I32Const(0));
    function.instruction(&Instruction::GlobalSet(5));

    for (index, export) in exports.iter().enumerate() {
        let operation = export.operation.as_deref().unwrap_or(&export.name);
        let name = operation.as_bytes();
        let mut checks = Vec::new();
        for (offset, byte) in b"HTA0".iter().enumerate() {
            checks.push(byte_check(0, offset as u32, *byte));
        }
        for (offset, byte) in [(4, 9), (9, 4), (14 + name.len(), 9)] {
            checks.push(byte_check(0, offset as u32, byte));
        }
        for (offset, value) in [
            (5, 0),
            (6, 0),
            (7, 0),
            (8, 2),
            (10, ((name.len() as u32 >> 24) & 0xff) as u8),
            (11, ((name.len() as u32 >> 16) & 0xff) as u8),
            (12, ((name.len() as u32 >> 8) & 0xff) as u8),
            (13, (name.len() as u32 & 0xff) as u8),
            (15 + name.len(), 0),
            (16 + name.len(), 0),
            (17 + name.len(), 0),
            (18 + name.len(), export.arguments.len() as u8),
        ] {
            checks.push(byte_check(0, offset as u32, value));
        }
        for (offset, byte) in name.iter().enumerate() {
            checks.push(byte_check(0, 14 + offset as u32, *byte));
        }
        checks.push(vec![
            Instruction::LocalGet(1),
            Instruction::I32Const(expected_frame_size(export, name.len()) as i32),
            Instruction::I32Eq,
        ]);
        for (index, check) in checks.into_iter().enumerate() {
            for instruction in check {
                function.instruction(&instruction);
            }
            if index != 0 {
                function.instruction(&Instruction::I32And);
            }
        }
        function.instruction(&Instruction::If(wasm_encoder::BlockType::Empty));
        function.instruction(&Instruction::I32Const(1));
        function.instruction(&Instruction::GlobalSet(5));
        let mut offset = 19 + name.len() as u32;
        for argument in &export.arguments {
            decode_argument(&mut function, argument, offset);
            offset += encoded_size(argument);
        }
        function.instruction(&Instruction::Call(index as u32));
        encode_result(&mut function, &export.returns);
        function.instruction(&Instruction::End);
    }

    function.instruction(&Instruction::GlobalGet(5));
    function.instruction(&Instruction::If(wasm_encoder::BlockType::Result(
        ValType::I64,
    )));
    function.instruction(&Instruction::LocalGet(2));
    function.instruction(&Instruction::GlobalSet(1));
    store_byte(&mut function, 2, 0, b'H');
    store_byte(&mut function, 2, 1, b'T');
    store_byte(&mut function, 2, 2, b'A');
    store_byte(&mut function, 2, 3, b'0');
    store_byte(&mut function, 2, 4, 9);
    store_i32_constant(&mut function, 2, 5, 3);
    store_byte(&mut function, 2, 9, 3);
    store_i64_constant(&mut function, 2, 10, 0);
    store_byte(&mut function, 2, 18, 3);
    store_i64_constant(&mut function, 2, 19, 1);
    store_byte_global(&mut function, 2, 27, 3);
    function.instruction(&Instruction::GlobalGet(4));
    function.instruction(&Instruction::I32Const(27));
    function.instruction(&Instruction::I32Add);
    function.instruction(&Instruction::GlobalSet(2));
    for offset in 0..8 {
        store_i64_byte_from_local(&mut function, 2, 4, 28 + offset, 7 - offset);
    }
    function.instruction(&Instruction::I64Const(1));
    function.instruction(&Instruction::Else);
    function.instruction(&Instruction::I64Const(0));
    function.instruction(&Instruction::End);
    function.instruction(&Instruction::End);
    function
}

fn expected_frame_size(export: &BindingFunction, operation_length: usize) -> u32 {
    19 + operation_length as u32 + export.arguments.iter().map(encoded_size).sum::<u32>()
}

fn store_i32_constant(function: &mut Function, pointer: u32, offset: u32, value: i32) {
    for byte in 0..4 {
        function.instruction(&Instruction::LocalGet(pointer));
        function.instruction(&Instruction::I32Const((offset + byte) as i32));
        function.instruction(&Instruction::I32Add);
        function.instruction(&Instruction::I32Const((value >> ((3 - byte) * 8)) & 0xff));
        function.instruction(&Instruction::I32Store8(MemArg {
            offset: 0,
            align: 0,
            memory_index: 0,
        }));
    }
}

fn byte_check(pointer: u32, offset: u32, value: u8) -> Vec<Instruction<'static>> {
    let mut instructions = Vec::with_capacity(3);
    load_byte(&mut instructions, pointer, offset);
    instructions.push(Instruction::I32Const(i32::from(value)));
    instructions.push(Instruction::I32Eq);
    instructions
}

fn emit_next_event() -> Function {
    let mut function = Function::new([(1, ValType::I32), (1, ValType::I32)]);
    function.instruction(&Instruction::GlobalGet(1));
    function.instruction(&Instruction::LocalTee(0));
    function.instruction(&Instruction::I32Eqz);
    function.instruction(&Instruction::If(wasm_encoder::BlockType::Result(
        ValType::I64,
    )));
    function.instruction(&Instruction::I64Const(0));
    function.instruction(&Instruction::Else);
    function.instruction(&Instruction::GlobalGet(2));
    function.instruction(&Instruction::LocalSet(1));
    function.instruction(&Instruction::I32Const(0));
    function.instruction(&Instruction::GlobalSet(1));
    function.instruction(&Instruction::LocalGet(0));
    function.instruction(&Instruction::I64ExtendI32U);
    function.instruction(&Instruction::I64Const(32));
    function.instruction(&Instruction::I64Shl);
    function.instruction(&Instruction::LocalGet(1));
    function.instruction(&Instruction::I64ExtendI32U);
    function.instruction(&Instruction::I64Or);
    function.instruction(&Instruction::End);
    function.instruction(&Instruction::End);
    function
}

fn decode_argument(function: &mut Function, argument: &super::BindingParameter, offset: u32) {
    match argument.hara_type {
        HaraValueType::Boolean => {
            load_byte_into(function, 0, offset);
            function.instruction(&Instruction::I32Const(2));
            function.instruction(&Instruction::I32Eq);
        }
        HaraValueType::I32 | HaraValueType::I64 => {
            load_i64_be(function, 0, offset + 1);
            if argument.wasm_type == WasmValueType::I32 {
                function.instruction(&Instruction::I32WrapI64);
            }
        }
        HaraValueType::F32 | HaraValueType::F64 => {
            load_i64_be(function, 0, offset + 1);
            function.instruction(&Instruction::F64ReinterpretI64);
            if argument.wasm_type == WasmValueType::F32 {
                function.instruction(&Instruction::F32DemoteF64);
            }
        }
        _ => unreachable!("non-scalar HTA arguments are rejected before emission"),
    }
}

fn encode_result(function: &mut Function, result: &super::BindingResult) {
    match result.hara_type {
        HaraValueType::Boolean => {
            function.instruction(&Instruction::I64ExtendI32S);
            function.instruction(&Instruction::LocalSet(3));
            function.instruction(&Instruction::LocalGet(3));
            function.instruction(&Instruction::I32WrapI64);
            function.instruction(&Instruction::I32Const(1));
            function.instruction(&Instruction::I32Add);
            function.instruction(&Instruction::GlobalSet(3));
            function.instruction(&Instruction::I32Const(1));
            function.instruction(&Instruction::GlobalSet(4));
        }
        HaraValueType::I32 => {
            function.instruction(&Instruction::I64ExtendI32S);
            function.instruction(&Instruction::LocalSet(3));
            set_result_metadata(function, 3, 9);
        }
        HaraValueType::I64 => {
            function.instruction(&Instruction::LocalSet(3));
            set_result_metadata(function, 3, 9);
        }
        HaraValueType::F32 => {
            function.instruction(&Instruction::F64PromoteF32);
            function.instruction(&Instruction::I64ReinterpretF64);
            function.instruction(&Instruction::LocalSet(3));
            set_result_metadata(function, 15, 9);
        }
        HaraValueType::F64 => {
            function.instruction(&Instruction::I64ReinterpretF64);
            function.instruction(&Instruction::LocalSet(3));
            set_result_metadata(function, 15, 9);
        }
        HaraValueType::Void => {
            function.instruction(&Instruction::I64Const(0));
            function.instruction(&Instruction::LocalSet(3));
            set_result_metadata(function, 0, 1);
        }
        _ => unreachable!("non-scalar HTA results are rejected before emission"),
    }
    function.instruction(&Instruction::LocalGet(3));
    function.instruction(&Instruction::LocalSet(4));
}

fn set_result_metadata(function: &mut Function, tag: i32, size: i32) {
    function.instruction(&Instruction::I32Const(tag));
    function.instruction(&Instruction::GlobalSet(3));
    function.instruction(&Instruction::I32Const(size));
    function.instruction(&Instruction::GlobalSet(4));
}

fn encoded_size(argument: &super::BindingParameter) -> u32 {
    match argument.hara_type {
        HaraValueType::Boolean => 1,
        _ => 9,
    }
}

fn load_byte(instructions: &mut Vec<Instruction<'static>>, pointer: u32, offset: u32) {
    instructions.push(Instruction::LocalGet(pointer));
    instructions.push(Instruction::I32Const(offset as i32));
    instructions.push(Instruction::I32Add);
    instructions.push(Instruction::I32Load8U(MemArg {
        offset: 0,
        align: 0,
        memory_index: 0,
    }));
}

fn load_i64_be(function: &mut Function, pointer: u32, offset: u32) {
    function.instruction(&Instruction::I64Const(0));
    for byte in 0..8 {
        load_byte_into(function, pointer, offset + byte);
        function.instruction(&Instruction::I64ExtendI32U);
        function.instruction(&Instruction::I64Const(i64::from((7 - byte) * 8)));
        function.instruction(&Instruction::I64Shl);
        function.instruction(&Instruction::I64Or);
    }
}

fn load_byte_into(function: &mut Function, pointer: u32, offset: u32) {
    function.instruction(&Instruction::LocalGet(pointer));
    function.instruction(&Instruction::I32Const(offset as i32));
    function.instruction(&Instruction::I32Add);
    function.instruction(&Instruction::I32Load8U(MemArg {
        offset: 0,
        align: 0,
        memory_index: 0,
    }));
}

fn store_byte(function: &mut Function, pointer: u32, offset: u32, value: u8) {
    function.instruction(&Instruction::LocalGet(pointer));
    function.instruction(&Instruction::I32Const(offset as i32));
    function.instruction(&Instruction::I32Add);
    function.instruction(&Instruction::I32Const(i32::from(value)));
    function.instruction(&Instruction::I32Store8(MemArg {
        offset: 0,
        align: 0,
        memory_index: 0,
    }));
}

fn store_byte_global(function: &mut Function, pointer: u32, offset: u32, global: u32) {
    function.instruction(&Instruction::LocalGet(pointer));
    function.instruction(&Instruction::I32Const(offset as i32));
    function.instruction(&Instruction::I32Add);
    function.instruction(&Instruction::GlobalGet(global));
    function.instruction(&Instruction::I32Store8(MemArg {
        offset: 0,
        align: 0,
        memory_index: 0,
    }));
}

fn store_i64_constant(function: &mut Function, pointer: u32, offset: u32, value: i64) {
    function.instruction(&Instruction::I64Const(value));
    function.instruction(&Instruction::LocalSet(3));
    for byte in 0..8 {
        store_i64_byte_from_local(function, pointer, 3, offset + byte, 7 - byte);
    }
}

fn store_i64_byte_from_local(
    function: &mut Function,
    pointer: u32,
    local: u32,
    offset: u32,
    shift_bytes: u32,
) {
    function.instruction(&Instruction::LocalGet(pointer));
    function.instruction(&Instruction::I32Const(offset as i32));
    function.instruction(&Instruction::I32Add);
    function.instruction(&Instruction::LocalGet(local));
    function.instruction(&Instruction::I64Const(i64::from(shift_bytes * 8)));
    function.instruction(&Instruction::I64ShrU);
    function.instruction(&Instruction::I32WrapI64);
    function.instruction(&Instruction::I32Store8(MemArg {
        offset: 0,
        align: 0,
        memory_index: 0,
    }));
}

fn adapter_manifest(
    interface: &WasmInterface,
    module_digest: &str,
    interface_digest: &str,
    adapter_digest: &str,
    exports: &[BindingFunction],
) -> String {
    let exports = exports
        .iter()
        .map(|export| {
            Form::Map(vec![
                (keyword("hara/name"), symbol(&export.name)),
                (keyword("wasm/export"), string(&export.wasm_export)),
            ])
        })
        .collect();
    Form::Map(vec![
        (keyword("schema"), string(ADAPTER_MANIFEST_SCHEMA)),
        (keyword("target"), keyword(ADAPTER_TARGET)),
        (keyword("namespace"), symbol(&interface.namespace)),
        (
            keyword("composition"),
            Form::Map(vec![
                (keyword("import-module"), string(LIBRARY_IMPORT_MODULE)),
                (keyword("library"), string(&interface.module)),
            ]),
        ),
        (
            keyword("inputs"),
            Form::Map(vec![
                (keyword("module-digest"), string(module_digest)),
                (keyword("interface-digest"), string(interface_digest)),
            ]),
        ),
        (keyword("adapter-digest"), string(adapter_digest)),
        (
            keyword("tool"),
            Form::Map(vec![
                (keyword("name"), string("hara-wasm-bindgen")),
                (keyword("version"), string(env!("CARGO_PKG_VERSION"))),
            ]),
        ),
        (keyword("exports"), Form::Vector(exports)),
    ])
    .to_string()
}

fn hta_adapter_manifest(
    interface: &WasmInterface,
    module_digest: &str,
    interface_digest: &str,
    adapter_digest: &str,
    exports: &[BindingFunction],
) -> String {
    let exports = exports
        .iter()
        .map(|export| {
            let mut fields = vec![
                (keyword("hara/name"), symbol(&export.name)),
                (keyword("wasm/export"), string(&export.wasm_export)),
                (keyword("async"), Form::Bool(true)),
            ];
            if let Some(operation) = export.operation.as_deref() {
                fields.push((keyword("operation"), string(operation)));
            }
            Form::Map(fields)
        })
        .collect();
    Form::Map(vec![
        (keyword("schema"), string(ADAPTER_MANIFEST_SCHEMA)),
        (keyword("target"), keyword(HTA_ADAPTER_TARGET)),
        (keyword("namespace"), symbol(&interface.namespace)),
        (
            keyword("composition"),
            Form::Map(vec![
                (keyword("import-module"), string(LIBRARY_IMPORT_MODULE)),
                (keyword("library"), string(&interface.module)),
            ]),
        ),
        (
            keyword("inputs"),
            Form::Map(vec![
                (keyword("module-digest"), string(module_digest)),
                (keyword("interface-digest"), string(interface_digest)),
                (keyword("ir-digest"), string(interface_digest)),
            ]),
        ),
        (keyword("adapter-digest"), string(adapter_digest)),
        (
            keyword("tool"),
            Form::Map(vec![
                (keyword("name"), string("hara-wasm-bindgen")),
                (keyword("version"), string(env!("CARGO_PKG_VERSION"))),
                (keyword("digest"), string(&tool_digest())),
            ]),
        ),
        (keyword("exports"), Form::Vector(exports)),
    ])
    .to_string()
}

fn result_types(value: WasmValueType) -> Vec<ValType> {
    match value {
        WasmValueType::Void => Vec::new(),
        value => vec![val_type(value)],
    }
}

fn val_type(value: WasmValueType) -> ValType {
    match value {
        WasmValueType::I32 => ValType::I32,
        WasmValueType::I64 => ValType::I64,
        WasmValueType::F32 => ValType::F32,
        WasmValueType::F64 => ValType::F64,
        WasmValueType::Void => panic!("void is not a parameter type"),
    }
}

fn digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn tool_digest() -> String {
    digest(format!("hara-wasm-bindgen@{}", env!("CARGO_PKG_VERSION")).as_bytes())
}

fn keyword(value: &str) -> Form {
    Form::Keyword(value.to_owned())
}

fn symbol(value: &str) -> Form {
    Form::Symbol(value.to_owned())
}

fn string(value: &str) -> Form {
    Form::String(value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wasm_binding::inspect_direct;

    const ADD: &[u8] =
        b"\0asm\x01\0\0\0\x01\x07\x01\x60\x02\x7e\x7e\x01\x7e\x03\x02\x01\0\x07\x07\x01\x03add\0\0\x0a\x09\x01\x07\0\x20\0\x20\x01\x7c\x0b";
    const START: &[u8] = b"\0asm\x01\0\0\0\x08\x01\0";

    fn interface() -> WasmInterface {
        WasmInterface::parse(
            r#"
            (wasm/interface
             {:schema "hara.wasm-interface/0-alpha"
              :namespace math.scalar
              :module "math.wasm"
              :exports
              {sum {:wasm/export "add"
                    :arguments [{:name left :hara/type :i64 :wasm/type :i64}
                                {:name right :hara/type :i64 :wasm/type :i64}]
                    :returns {:hara/type :i64 :wasm/type :i64}}}})
            "#,
            "fixture",
        )
        .unwrap()
    }

    fn async_interface() -> WasmInterface {
        WasmInterface::parse(
            r#"
            (wasm/interface
             {:schema "hara.wasm-interface/0-alpha"
              :namespace math.scalar
              :module "math.wasm"
              :exports
              {sum {:wasm/export "add"
                    :async true
                    :arguments [{:name left :hara/type :i64 :wasm/type :i64}
                                {:name right :hara/type :i64 :wasm/type :i64}]
                    :returns {:hara/type :i64 :wasm/type :i64}}}})
            "#,
            "fixture",
        )
        .unwrap()
    }

    fn multi_interface() -> WasmInterface {
        WasmInterface::parse(
            r#"
            (wasm/interface
             {:schema "hara.wasm-interface/0-alpha"
              :namespace math.scalar
              :module "math.wasm"
              :exports
              {difference {:wasm/export "sub"
                           :arguments [{:name left :hara/type :i64 :wasm/type :i64}
                                       {:name right :hara/type :i64 :wasm/type :i64}]
                           :returns {:hara/type :i64 :wasm/type :i64}}
               sum {:wasm/export "add"
                    :arguments [{:name left :hara/type :i64 :wasm/type :i64}
                                {:name right :hara/type :i64 :wasm/type :i64}]
                    :returns {:hara/type :i64 :wasm/type :i64}}}})
            "#,
            "fixture",
        )
        .unwrap()
    }

    fn multi_library() -> Vec<u8> {
        let mut module = Module::new();
        let mut types = TypeSection::new();
        types.function([ValType::I64, ValType::I64], [ValType::I64]);
        module.section(&types);

        let mut functions = FunctionSection::new();
        functions.function(0);
        functions.function(0);
        module.section(&functions);

        let mut exports = ExportSection::new();
        exports.export("add", ExportKind::Func, 0);
        exports.export("sub", ExportKind::Func, 1);
        module.section(&exports);

        let mut code = wasm_encoder::CodeSection::new();
        for instruction in [Instruction::I64Add, Instruction::I64Sub] {
            let mut function = Function::new([]);
            function.instruction(&Instruction::LocalGet(0));
            function.instruction(&Instruction::LocalGet(1));
            function.instruction(&instruction);
            function.instruction(&Instruction::End);
            code.function(&function);
        }
        module.section(&code);
        module.finish()
    }

    #[test]
    fn adapter_is_deterministic_and_records_all_input_digests() {
        let interface = interface();
        let first = generate_adapter(ADD, &interface).unwrap();
        let second = generate_adapter(ADD, &interface).unwrap();
        assert_eq!(first, second);
        assert!(first.manifest.contains("hara.wasm-adapter/0-alpha"));
        assert!(first.manifest.contains(":module-digest"));
        assert!(first.manifest.contains(":interface-digest"));
        assert!(first.manifest.contains(":adapter-digest"));
    }

    #[test]
    fn adapter_exports_hara_names_and_imports_exact_library_names() {
        let artifact = generate_adapter(ADD, &interface()).unwrap();
        let inspection = inspect_direct(&artifact.bytes).unwrap();
        assert_eq!(inspection.imports[0].module, "hara/library");
        assert_eq!(inspection.imports[0].name, "add");
        assert_eq!(inspection.exports[0].name, "sum");
        assert_eq!(
            inspection.exports[0].signature.arguments,
            vec!["i64", "i64"]
        );
        assert_eq!(inspection.exports[0].signature.returns, "i64");
    }

    #[test]
    fn adapter_forwards_calls_when_composed_with_the_wrapped_library() {
        let artifact = generate_adapter(ADD, &interface()).unwrap();
        let engine = wasmtime::Engine::default();
        let library = wasmtime::Module::new(&engine, ADD).unwrap();
        let adapter = wasmtime::Module::new(&engine, &artifact.bytes).unwrap();
        let mut store = wasmtime::Store::new(&engine, ());
        let library_instance = wasmtime::Instance::new(&mut store, &library, &[]).unwrap();
        let add = library_instance.get_func(&mut store, "add").unwrap();
        let adapter_instance =
            wasmtime::Instance::new(&mut store, &adapter, &[add.into()]).unwrap();
        let sum = adapter_instance
            .get_typed_func::<(i64, i64), i64>(&mut store, "sum")
            .unwrap();

        assert_eq!(sum.call(&mut store, (19, 23)).unwrap(), 42);
    }

    #[test]
    fn multi_export_adapter_forwards_each_import_with_its_declared_signature() {
        let library_bytes = multi_library();
        let artifact = generate_adapter(&library_bytes, &multi_interface()).unwrap();
        let engine = wasmtime::Engine::default();
        let library = wasmtime::Module::new(&engine, &library_bytes).unwrap();
        let adapter = wasmtime::Module::new(&engine, &artifact.bytes).unwrap();
        let mut store = wasmtime::Store::new(&engine, ());
        let library_instance = wasmtime::Instance::new(&mut store, &library, &[]).unwrap();
        let add = library_instance.get_func(&mut store, "add").unwrap();
        let sub = library_instance.get_func(&mut store, "sub").unwrap();
        let adapter_instance =
            wasmtime::Instance::new(&mut store, &adapter, &[sub.into(), add.into()]).unwrap();
        let difference = adapter_instance
            .get_typed_func::<(i64, i64), i64>(&mut store, "difference")
            .unwrap();
        let sum = adapter_instance
            .get_typed_func::<(i64, i64), i64>(&mut store, "sum")
            .unwrap();

        assert_eq!(difference.call(&mut store, (23, 19)).unwrap(), 4);
        assert_eq!(sum.call(&mut store, (19, 23)).unwrap(), 42);
    }

    #[test]
    fn adapter_output_order_is_canonical_for_constructed_interfaces() {
        let mut interface = multi_interface();
        interface.exports.reverse();
        let first = generate_adapter(&multi_library(), &interface).unwrap();
        interface.exports.reverse();
        let second = generate_adapter(&multi_library(), &interface).unwrap();

        assert_eq!(first.bytes, second.bytes);
    }

    #[test]
    fn malformed_or_richer_interfaces_are_rejected_before_generation() {
        let interface = WasmInterface::parse(
            r#"
            {:schema "hara.wasm-interface/0-alpha"
             :namespace codec.echo
             :module "echo.wasm"
             :memory {:export "memory" :allocate "alloc"}
             :exports
             {echo {:wasm/export "echo"
                    :arguments [{:name input :hara/type :bytes :wasm/type :i32
                                 :lower [:pointer :length] :ownership :borrowed}]
                    :returns {:hara/type :bytes :wasm/type :i64
                              :lift :packed-i64 :ownership :callee}}}}
            "#,
            "fixture",
        )
        .unwrap();
        let error = generate_adapter(ADD, &interface).unwrap_err();
        assert!(error.contains("memory requires"));
    }

    #[test]
    fn start_functions_are_rejected_during_static_validation() {
        let error = generate_adapter(START, &interface()).unwrap_err();
        assert!(error.starts_with("wasm-adapter/start-denied"));
        let error = generate_hta_adapter(START, &async_interface()).unwrap_err();
        assert!(error.starts_with("wasm-adapter/start-denied"));
    }

    #[test]
    fn hta_adapter_dispatches_a_scalar_request_and_emits_a_terminal_event() {
        let artifact = generate_hta_adapter(ADD, &async_interface()).unwrap();
        let engine = wasmtime::Engine::default();
        let library = wasmtime::Module::new(&engine, ADD).unwrap();
        let adapter = wasmtime::Module::new(&engine, &artifact.bytes).unwrap();
        let mut store = wasmtime::Store::new(&engine, ());
        let library_instance = wasmtime::Instance::new(&mut store, &library, &[]).unwrap();
        let add = library_instance.get_func(&mut store, "add").unwrap();
        let adapter_instance =
            wasmtime::Instance::new(&mut store, &adapter, &[add.into()]).unwrap();
        let memory = adapter_instance.get_memory(&mut store, "memory").unwrap();
        let alloc = adapter_instance
            .get_typed_func::<i32, i32>(&mut store, "hta_alloc")
            .unwrap();
        let start = adapter_instance
            .get_typed_func::<(i32, i32), i64>(&mut store, "hta_start")
            .unwrap();
        let next_event = adapter_instance
            .get_typed_func::<(), i64>(&mut store, "hta_next_event")
            .unwrap();
        let request = crate::hta::encode(&crate::core::Value::Vector(
            vec![
                crate::core::Value::String("sum".into()),
                crate::core::Value::Vector(
                    vec![
                        crate::core::Value::Number(19),
                        crate::core::Value::Number(23),
                    ]
                    .into(),
                ),
            ]
            .into(),
        ))
        .unwrap();
        let pointer = alloc.call(&mut store, request.len() as i32).unwrap();
        memory
            .write(&mut store, pointer as usize, &request)
            .unwrap();

        let task = start
            .call(&mut store, (pointer, request.len() as i32))
            .unwrap();
        assert_eq!(task, 1);
        let packed = next_event.call(&mut store, ()).unwrap() as u64;
        let event_pointer = (packed >> 32) as usize;
        let event_size = (packed & u64::from(u32::MAX)) as usize;
        let mut event = vec![0; event_size];
        memory.read(&store, event_pointer, &mut event).unwrap();
        assert_eq!(
            crate::hta::decode_canonical(&event).unwrap(),
            crate::core::Value::Vector(
                vec![
                    crate::core::Value::Number(0),
                    crate::core::Value::Number(1),
                    crate::core::Value::Number(42),
                ]
                .into()
            )
        );
        assert_eq!(next_event.call(&mut store, ()).unwrap(), 0);
    }
}
