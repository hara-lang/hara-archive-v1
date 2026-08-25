//! The synchronous target-call ABI shared by Whole-Wasm hosts.
//!
//! Generated code carries an artifact-local target id. The HNW0 artifact also
//! carries the descriptor table, so native and browser hosts validate and
//! dispatch the same target inventory without maintaining numeric switches.

use sha2::{Digest, Sha256};

pub const SLOT_BYTES: u32 = 16;
pub const MAX_SLOTS: u32 = 64;
pub const HEAP_BASE: u32 = SLOT_BYTES * MAX_SLOTS;

pub const SLOT_HANDLE: u32 = 0;
pub const SLOT_I64: u32 = 1;
pub const SLOT_BOOL: u32 = 2;
pub const SLOT_NIL: u32 = 3;
pub const SLOT_CONSTANT: u32 = 4;

pub const RESULT_HANDLE: i64 = 0;
pub const RESULT_I64: i64 = 1;
pub const RESULT_BOOL: i64 = 2;

const VECTOR_CONSTRUCT: &str = "hara.whole-wasm/vector";
const MAP_CONSTRUCT: &str = "hara.whole-wasm/map";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TargetKind {
    Protocol = 0,
    Native = 1,
    VectorConstruct = 2,
    MapConstruct = 3,
}

impl TargetKind {
    pub const fn wire(self) -> u8 {
        self as u8
    }

    pub fn from_wire(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Protocol),
            1 => Some(Self::Native),
            2 => Some(Self::VectorConstruct),
            3 => Some(Self::MapConstruct),
            _ => None,
        }
    }
}

/// The bounded operation inventory emitted by the current HNW0 compiler.
///
/// The inventory describes HNW0 capability, while protocol/native registries
/// remain authoritative for operation identity and declaration validity. It
/// is deliberately data-shaped: compiler code asks for an operation by its
/// canonical key and the artifact carries the resulting local id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationDeclaration {
    pub key: String,
    pub kind: TargetKind,
    pub arity: Option<u16>,
}

static OPERATIONS: std::sync::OnceLock<Vec<OperationDeclaration>> = std::sync::OnceLock::new();

pub fn operation_declarations() -> &'static [OperationDeclaration] {
    OPERATIONS
        .get_or_init(|| {
            let mut operations = vec![
                OperationDeclaration {
                    key: MAP_CONSTRUCT.to_owned(),
                    kind: TargetKind::MapConstruct,
                    arity: None,
                },
                OperationDeclaration {
                    key: VECTOR_CONSTRUCT.to_owned(),
                    kind: TargetKind::VectorConstruct,
                    arity: None,
                },
            ];

            for declaration in crate::core::native_declarations() {
                for method in declaration.whole_wasm_methods {
                    operations.push(OperationDeclaration {
                        key: format!(
                            "{}.{}/{}",
                            declaration.namespace, declaration.name, method.name
                        ),
                        kind: TargetKind::Native,
                        arity: Some(method.arity),
                    });
                }
            }

            for declaration in crate::lang::protocol::protocol_declarations() {
                for method in declaration.methods.iter().filter(|method| method.whole_wasm) {
                    let (minimum, _) = method.arity.range();
                    operations.push(OperationDeclaration {
                        key: format!("{}/{}", declaration.runtime_name(), method.name),
                        kind: TargetKind::Protocol,
                        arity: Some(
                            u16::try_from(minimum)
                                .expect("whole-Wasm protocol arity fits u16"),
                        ),
                    });
                }
            }

            operations
        })
        .as_slice()
}

pub fn operation_id(key: &str) -> Result<i64, String> {
    validate_operation_declarations()?;
    operation_declarations()
        .iter()
        .position(|operation| operation.key == key)
        .map(|id| i64::try_from(id).expect("HNW0 operation inventory fits i64"))
        .ok_or_else(|| format!("whole-Wasm operation is not declared: {key}"))
}

/// Stable identity for the operation inventory and the declarations it
/// references. HNW0 hosts compare this digest before executing an artifact.
pub fn operation_registry_digest() -> [u8; 32] {
    let mut canonical = Vec::new();
    for operation in operation_declarations() {
        canonical.extend_from_slice(operation.key.as_bytes());
        canonical.push(0);
        canonical.push(operation.kind.wire());
        canonical.extend_from_slice(&operation.arity.unwrap_or(u16::MAX).to_be_bytes());
    }
    let digest = Sha256::digest(canonical);
    let mut result = [0u8; 32];
    result.copy_from_slice(&digest);
    result
}

fn validate_operation_declarations() -> Result<(), String> {
    for operation in operation_declarations() {
        match operation.kind {
            TargetKind::MapConstruct if operation.key == MAP_CONSTRUCT => {}
            TargetKind::VectorConstruct if operation.key == VECTOR_CONSTRUCT => {}
            TargetKind::MapConstruct | TargetKind::VectorConstruct => {
                return Err(format!(
                    "whole-Wasm structural operation has invalid key: {}",
                    operation.key
                ));
            }
            TargetKind::Native => validate_native_operation(operation)?,
            TargetKind::Protocol => validate_protocol_operation(operation)?,
        }
    }
    Ok(())
}

fn validate_native_operation(operation: &OperationDeclaration) -> Result<(), String> {
    let name = operation
        .key
        .strip_prefix("std.native/")
        .or_else(|| operation.key.strip_prefix("std.native."))
        .ok_or_else(|| format!("native operation is not canonical: {}", operation.key))?;
    let (native, method) = name
        .split_once('/')
        .ok_or_else(|| format!("native operation has no method: {}", operation.key))?;
    let declared = crate::core::native_declarations()
        .iter()
        .find(|declaration| declaration.name == native)
        .ok_or_else(|| format!("native operation is not declared: {}", operation.key))?;
    if !declared.method(method) {
        return Err(format!("native method is not declared: {}", operation.key));
    }
    let declared_operation = declared
        .whole_wasm_method(method)
        .ok_or_else(|| format!("native method is not in the HNW0 registry: {}", operation.key))?;
    if operation.arity != Some(declared_operation.arity) {
        return Err(format!(
            "HNW0 native operation arity does not match declaration: {}",
            operation.key
        ));
    }
    Ok(())
}

fn validate_protocol_operation(operation: &OperationDeclaration) -> Result<(), String> {
    let (namespace, method) = operation
        .key
        .split_once('/')
        .ok_or_else(|| format!("protocol operation has no method: {}", operation.key))?;
    let protocol = crate::lang::protocol::protocol_declarations()
        .iter()
        .find(|declaration| declaration.runtime_name() == namespace)
        .ok_or_else(|| format!("protocol operation is not declared: {}", operation.key))?;
    let method_declaration = protocol
        .method(method)
        .ok_or_else(|| format!("protocol method is not declared: {}", operation.key))?;
    if !method_declaration.whole_wasm {
        return Err(format!(
            "protocol method is not in the HNW0 registry: {}",
            operation.key
        ));
    }
    let (minimum, _) = method_declaration.arity.range();
    if operation.arity != Some(u16::try_from(minimum).map_err(|_| {
        format!("protocol operation arity is too large: {}", operation.key)
    })?) {
        return Err(format!(
            "HNW0 operation arity does not match protocol declaration: {}",
            operation.key
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetDescriptor {
    pub id: u16,
    pub symbol: String,
    pub kind: TargetKind,
    pub arity: Option<u16>,
}

pub fn target_table() -> Vec<TargetDescriptor> {
    operation_declarations()
        .iter()
        .enumerate()
        .map(|(id, operation)| TargetDescriptor {
            id: u16::try_from(id).expect("target inventory fits u16"),
            symbol: operation.key.to_owned(),
            kind: operation.kind,
            arity: operation.arity,
        })
        .collect()
}

pub fn validate_target_table(targets: &[TargetDescriptor]) -> Result<(), String> {
    validate_operation_declarations()?;
    if targets.len() != operation_declarations().len() {
        return Err("native artifact target table is incomplete".into());
    }
    for (expected_id, (actual, expected)) in targets
        .iter()
        .zip(operation_declarations())
        .enumerate()
    {
        if actual.id != u16::try_from(expected_id).expect("target inventory fits u16")
            || actual.symbol != expected.key.as_str()
            || actual.kind != expected.kind
            || actual.arity != expected.arity
        {
            return Err("native artifact target table is not canonical".into());
        }
    }
    Ok(())
}

pub fn validate_target_call(
    target: &TargetDescriptor,
    argc: usize,
    result_mode: i64,
) -> Result<(), String> {
    if !matches!(target.kind, TargetKind::Protocol | TargetKind::Native) {
        return Err(format!(
            "whole-Wasm target is not callable: {}",
            target.symbol
        ));
    }
    if target.arity.is_some_and(|arity| usize::from(arity) != argc) {
        return Err(format!(
            "whole-Wasm target {} expects {} arguments, got {argc}",
            target.symbol,
            target.arity.expect("checked above")
        ));
    }
    validate_result_mode(result_mode)
}

pub fn validate_value_construct(target: &TargetDescriptor, argc: usize) -> Result<(), String> {
    match target.kind {
        TargetKind::VectorConstruct => Ok(()),
        TargetKind::MapConstruct if argc % 2 == 0 => Ok(()),
        TargetKind::MapConstruct => Err("whole-Wasm map construction needs key/value pairs".into()),
        TargetKind::Protocol | TargetKind::Native => Err(format!(
            "whole-Wasm target is not a value constructor: {}",
            target.symbol
        )),
    }
}

pub fn validate_result_mode(mode: i64) -> Result<(), String> {
    match mode {
        RESULT_HANDLE | RESULT_I64 | RESULT_BOOL => Ok(()),
        _ => Err(format!("whole-Wasm bridge has invalid result mode {mode}")),
    }
}

pub fn result_mode_name(mode: i64) -> Option<&'static str> {
    match mode {
        RESULT_HANDLE => Some("handle"),
        RESULT_I64 => Some("i64"),
        RESULT_BOOL => Some("bool"),
        _ => None,
    }
}

pub fn validate_slots(slots: &[Slot]) -> Result<(), String> {
    if slots.len() > usize::try_from(MAX_SLOTS).expect("constant fits usize") {
        return Err(format!(
            "whole-Wasm bridge supports at most {MAX_SLOTS} arguments"
        ));
    }
    for slot in slots {
        if !matches!(
            slot.kind,
            SLOT_HANDLE | SLOT_I64 | SLOT_BOOL | SLOT_NIL | SLOT_CONSTANT
        ) {
            return Err(format!(
                "whole-Wasm bridge has invalid slot kind {}",
                slot.kind
            ));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Slot {
    pub kind: u32,
    pub payload: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_ids_are_derived_from_runtime_declarations() {
        assert!(validate_operation_declarations().is_ok());
        assert_eq!(
            operation_id("std.protocol.icount.ICount/count"),
            Ok(4)
        );
        assert_eq!(target_table().len(), 7);
        assert!(validate_target_table(&target_table()).is_ok());
    }

    #[test]
    fn target_calls_validate_kind_arity_and_result_mode() {
        let target = target_table().remove(4);
        assert!(validate_target_call(&target, 1, RESULT_I64).is_ok());
        assert!(validate_target_call(&target, 2, RESULT_I64).is_err());
        assert!(validate_target_call(&target, 1, 99).is_err());
    }

    #[test]
    fn slots_remain_bounded_and_typed() {
        assert_eq!(HEAP_BASE, 1024);
        assert!(validate_slots(&[Slot {
            kind: SLOT_NIL,
            payload: 0,
        }])
        .is_ok());
        assert!(validate_slots(&[Slot {
            kind: 99,
            payload: 0,
        }])
        .is_err());
    }
}
