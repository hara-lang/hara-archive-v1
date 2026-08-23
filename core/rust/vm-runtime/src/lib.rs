//! VM-only Hara execution surface.
//!
//! This crate intentionally exposes artifact verification, preparation and
//! execution, but no source or HALC compiler entry points.

pub use hara_runtime::core::Value;
pub use hara_runtime::vm::{
    decode_program, disassemble, execute_program, execute_program_with_globals, prepare_call,
    validate, FunctionId, FunctionPrototype, Instruction, Machine, PreparedCall, Program,
    ValidationError, VmError, VmFiber, VmFiberState, VmOutcome,
};

use std::rc::Rc;

pub fn load(bytes: &[u8]) -> Result<Rc<Program>, String> {
    decode_program(bytes).map(Rc::new)
}

pub fn execute(bytes: &[u8]) -> Result<Value, String> {
    let program = load(bytes)?;
    execute_program(program).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unverified_input_without_a_compiler_dependency() {
        assert_eq!(
            load(b"not-hbc").unwrap_err(),
            "bytecode artifact has invalid magic"
        );
    }
}
