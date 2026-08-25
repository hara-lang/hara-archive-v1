//! The synchronous target-call ABI shared by Whole-Wasm hosts.
//!
//! Generated code carries an artifact-local target id. The HNW0 artifact also
//! carries the descriptor table, so native and browser hosts validate and
//! dispatch the same target inventory without maintaining numeric switches.

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

/// Declaration of every target understood by generated HNW0 modules.
///
/// Code generation refers to these declarations directly. The artifact wire
/// table is derived from the same list, so host dispatch cannot silently
/// acquire a target that the compiler does not know how to emit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    MapConstruct,
    VectorConstruct,
    NativeNumber,
    ProtocolAssoc,
    ProtocolCount,
    ProtocolLookup,
    ProtocolNth,
}

impl Target {
    pub const ALL: &[Self] = &[
        Self::MapConstruct,
        Self::VectorConstruct,
        Self::NativeNumber,
        Self::ProtocolAssoc,
        Self::ProtocolCount,
        Self::ProtocolLookup,
        Self::ProtocolNth,
    ];

    pub const fn symbol(self) -> &'static str {
        match self {
            Self::MapConstruct => MAP_CONSTRUCT,
            Self::VectorConstruct => VECTOR_CONSTRUCT,
            Self::NativeNumber => "std.native.Base/number?",
            Self::ProtocolAssoc => "std.protocol.iassoc.IAssoc/assoc",
            Self::ProtocolCount => "std.protocol.icount.ICount/count",
            Self::ProtocolLookup => "std.protocol.ilookup.ILookup/lookup",
            Self::ProtocolNth => "std.protocol.inth.INth/nth",
        }
    }

    pub const fn kind(self) -> TargetKind {
        match self {
            Self::MapConstruct => TargetKind::MapConstruct,
            Self::VectorConstruct => TargetKind::VectorConstruct,
            Self::NativeNumber => TargetKind::Native,
            Self::ProtocolAssoc
            | Self::ProtocolCount
            | Self::ProtocolLookup
            | Self::ProtocolNth => TargetKind::Protocol,
        }
    }

    pub const fn arity(self) -> Option<u16> {
        match self {
            Self::MapConstruct | Self::VectorConstruct => None,
            Self::NativeNumber | Self::ProtocolCount => Some(1),
            Self::ProtocolLookup | Self::ProtocolNth => Some(2),
            Self::ProtocolAssoc => Some(3),
        }
    }

    pub fn id(self) -> i64 {
        Self::ALL
            .iter()
            .position(|candidate| *candidate == self)
            .map(|id| i64::try_from(id).expect("target inventory fits i64"))
            .expect("target is present in its declaration table")
    }

    pub fn from_symbol(symbol: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|target| target.symbol() == symbol)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetDescriptor {
    pub id: u16,
    pub symbol: String,
    pub kind: TargetKind,
    pub arity: Option<u16>,
}

pub fn target_table() -> Vec<TargetDescriptor> {
    Target::ALL
        .iter()
        .enumerate()
        .map(|(id, target)| TargetDescriptor {
            id: u16::try_from(id).expect("target inventory fits u16"),
            symbol: target.symbol().to_owned(),
            kind: target.kind(),
            arity: target.arity(),
        })
        .collect()
}

pub fn validate_target_table(targets: &[TargetDescriptor]) -> Result<(), String> {
    if targets.len() != Target::ALL.len() {
        return Err("native artifact target table is incomplete".into());
    }
    for (expected_id, (actual, expected)) in targets.iter().zip(Target::ALL).enumerate() {
        if actual.id != u16::try_from(expected_id).expect("target inventory fits u16")
            || actual.symbol != expected.symbol()
            || actual.kind != expected.kind()
            || actual.arity != expected.arity()
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
    fn target_ids_are_derived_from_the_canonical_table() {
        assert_eq!(Target::ProtocolCount.id(), 4);
        assert_eq!(
            Target::from_symbol("std.protocol.icount.ICount/count"),
            Some(Target::ProtocolCount)
        );
        assert_eq!(
            Target::ProtocolCount.symbol(),
            "std.protocol.icount.ICount/count"
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
