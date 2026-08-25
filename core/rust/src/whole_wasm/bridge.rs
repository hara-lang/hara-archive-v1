//! The synchronous target-call ABI shared by Whole-Wasm hosts.
//!
//! The generated module writes a bounded array of 16-byte slots into its own
//! linear memory and calls one host import.  Keeping the slot format here
//! makes the Wasmtime and browser adapters decode the same values, while the
//! target id keeps protocol/native dispatch out of generated code.

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

pub const TARGET_ASSOC: i64 = 1;
pub const TARGET_LOOKUP: i64 = 2;
pub const TARGET_NUMBER_P: i64 = 3;
pub const TARGET_COUNT: i64 = 4;
pub const TARGET_NTH: i64 = 5;
pub const TARGET_VECTOR_CONSTRUCT: i64 = 100;
pub const TARGET_MAP_CONSTRUCT: i64 = 101;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Slot {
    pub kind: u32,
    pub payload: i64,
}

pub fn target_name(target: i64) -> Option<&'static str> {
    Some(match target {
        TARGET_ASSOC => "std.protocol.iassoc.IAssoc/assoc",
        TARGET_LOOKUP => "std.protocol.ilookup.ILookup/lookup",
        TARGET_NUMBER_P => "std.native.Base/number?",
        TARGET_COUNT => "std.protocol.icount.ICount/count",
        TARGET_NTH => "std.protocol.inth.INth/nth",
        _ => return None,
    })
}

pub fn validate_result_mode(mode: i64) -> Result<(), String> {
    match mode {
        RESULT_HANDLE | RESULT_I64 | RESULT_BOOL => Ok(()),
        _ => Err(format!("whole-Wasm bridge has invalid result mode {mode}")),
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
            return Err(format!("whole-Wasm bridge has invalid slot kind {}", slot.kind));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_ids_are_canonical_and_bounded() {
        assert_eq!(target_name(TARGET_ASSOC), Some("std.protocol.iassoc.IAssoc/assoc"));
        assert_eq!(target_name(99), None);
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
