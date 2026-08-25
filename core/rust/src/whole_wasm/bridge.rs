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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TargetSpec {
    pub symbol: &'static str,
    pub kind: TargetKind,
    pub arity: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetDescriptor {
    pub id: u16,
    pub symbol: String,
    pub kind: TargetKind,
    pub arity: Option<u16>,
}

// Keep the inventory sorted by canonical symbol. IDs are artifact-local and
// derived from this order; the artifact table remains the wire-level source
// consumed by both host implementations.
const TARGETS: &[TargetSpec] = &[
    TargetSpec {
        symbol: MAP_CONSTRUCT,
        kind: TargetKind::MapConstruct,
        arity: None,
    },
    TargetSpec {
        symbol: VECTOR_CONSTRUCT,
        kind: TargetKind::VectorConstruct,
        arity: None,
    },
    TargetSpec {
        symbol: "std.native.Base/number?",
        kind: TargetKind::Native,
        arity: Some(1),
    },
    TargetSpec {
        symbol: "std.protocol.iassoc.IAssoc/assoc",
        kind: TargetKind::Protocol,
        arity: Some(3),
    },
    TargetSpec {
        symbol: "std.protocol.icount.ICount/count",
        kind: TargetKind::Protocol,
        arity: Some(1),
    },
    TargetSpec {
        symbol: "std.protocol.ilookup.ILookup/lookup",
        kind: TargetKind::Protocol,
        arity: Some(2),
    },
    TargetSpec {
        symbol: "std.protocol.inth.INth/nth",
        kind: TargetKind::Protocol,
        arity: Some(2),
    },
];

pub fn target_id(symbol: &str) -> i64 {
    TARGETS
        .iter()
        .position(|target| target.symbol == symbol)
        .map(|id| i64::try_from(id).expect("target inventory fits i64"))
        .unwrap_or_else(|| panic!("unknown Whole-Wasm target {symbol}"))
}

pub fn target_spec(target: i64) -> Option<TargetSpec> {
    usize::try_from(target)
        .ok()
        .and_then(|index| TARGETS.get(index).copied())
}

pub fn target_table() -> Vec<TargetDescriptor> {
    TARGETS
        .iter()
        .enumerate()
        .map(|(id, target)| TargetDescriptor {
            id: u16::try_from(id).expect("target inventory fits u16"),
            symbol: target.symbol.to_owned(),
            kind: target.kind,
            arity: target.arity,
        })
        .collect()
}

pub fn validate_target_table(targets: &[TargetDescriptor]) -> Result<(), String> {
    if targets.len() != TARGETS.len() {
        return Err("native artifact target table is incomplete".into());
    }
    for (expected_id, (actual, expected)) in targets.iter().zip(TARGETS).enumerate() {
        if actual.id != u16::try_from(expected_id).expect("target inventory fits u16")
            || actual.symbol != expected.symbol
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
    fn target_ids_are_derived_from_the_canonical_table() {
        assert_eq!(target_id("std.protocol.icount.ICount/count"), 4);
        assert_eq!(target_spec(4).unwrap().arity, Some(1));
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
