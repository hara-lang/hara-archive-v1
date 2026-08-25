//! Whole-function WebAssembly compiler.
//!
//! Unlike the tracing experiment, this tier consumes a complete validated
//! bytecode [`Program`](crate::vm::Program), constructs typed basic-block IR,
//! and emits a deterministic portable Wasm module. HNW0 retains the HBC0
//! program as the semantic fallback and binds it to the generated module.

#[path = "whole_wasm/artifact.rs"]
mod artifact;
#[path = "whole_wasm/bridge.rs"]
mod bridge;
#[cfg(target_arch = "wasm32")]
#[path = "whole_wasm/browser.rs"]
mod browser;
#[path = "whole_wasm/call_boundary.rs"]
mod call_boundary;
#[path = "whole_wasm/codegen.rs"]
mod codegen;
#[path = "whole_wasm/handles.rs"]
mod handles;
#[cfg(not(target_arch = "wasm32"))]
#[path = "whole_wasm/hta_boundary.rs"]
mod hta_boundary;
#[path = "whole_wasm/ir.rs"]
mod ir;
#[path = "whole_wasm/reps.rs"]
mod reps;
#[cfg(not(target_arch = "wasm32"))]
#[path = "whole_wasm/runtime.rs"]
mod runtime;
#[path = "whole_wasm/ssa.rs"]
pub mod ssa;

pub use artifact::{compile_artifact, decode_artifact, NativeArtifact, HNW_ABI_VERSION};

/// Compiles a validated HBC0 artifact into the canonical HNW0 product.
///
/// The bytecode artifact is decoded before lowering so the Whole-Wasm target
/// cannot silently diverge from the HBC product that was selected by the
/// production graph. HNW0 retains the same bytes as its semantic fallback.
pub fn compile_artifact_from_hbc(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let program = crate::vm::decode_program(bytes)?;
    if crate::vm::encode_program(&program)? != bytes {
        return Err("HBC0 artifact is not canonical".into());
    }
    compile_artifact(&program)
}
#[cfg(target_arch = "wasm32")]
pub use browser::WholeWasmHost;
pub use codegen::compile_program;
pub use ir::Rep;
#[cfg(not(target_arch = "wasm32"))]
pub use runtime::NativeModule;
pub use ssa::{
    lower_program, BlockId, SsaBlock, SsaEdge, SsaFunction, SsaOp, SsaProgram, SsaTerminator,
    ValueId,
};

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::{compile_artifact, decode_artifact, NativeModule};
    use crate::vm::compile_source;

    fn module(source: &str) -> NativeModule {
        let program = compile_source(source).expect("source must compile to bytecode");
        let bytes = compile_artifact(&program).expect("bytecode must compile to HNW0");
        NativeModule::load(&bytes).expect("HNW0 must load")
    }

    #[test]
    fn whole_function_loop_executes_without_vm_dispatch() {
        let mut native = module("(loop [i 0 acc 0] (if (< i 5000) (recur (+ i 1) (+ acc i)) acc))");
        assert_eq!(native.call_entry_i64(), Ok(12_497_500));
    }

    #[test]
    fn artifact_is_deterministic_authenticated_and_retains_hbc_fallback() {
        let program = compile_source("(+ 19 23)").unwrap();
        let first = compile_artifact(&program).unwrap();
        assert_eq!(first, compile_artifact(&program).unwrap());
        let decoded = decode_artifact(&first).unwrap();
        assert_eq!(decoded.program.entry, program.entry);
        assert!(decoded.wasm.starts_with(b"\0asm"));
        let mut corrupt = first;
        let index = corrupt.len() / 2;
        corrupt[index] ^= 1;
        assert_eq!(
            decode_artifact(&corrupt).unwrap_err(),
            "native artifact checksum mismatch"
        );
    }

    #[test]
    fn arithmetic_errors_match_hara_semantics() {
        assert_eq!(
            module("(/ 1 0)").call_entry_i64(),
            Err("division by zero".into())
        );
        assert_eq!(
            module("(+ 9223372036854775807 1)").call_entry_value(),
            Ok(crate::core::Value::BigInteger(num_bigint::BigInt::from(
                9223372036854775808_i128
            ),))
        );
        assert_eq!(
            module("(* -9223372036854775808 -1)").call_entry_value(),
            Ok(crate::core::Value::BigInteger(num_bigint::BigInt::from(
                9223372036854775808_i128
            ),))
        );
    }

    #[test]
    fn point_sensitive_representations_preserve_numeric_truthiness() {
        assert_eq!(module("(if 0 19 23)").call_entry_i64(), Ok(19));
        assert_eq!(module("(if false 19 23)").call_entry_i64(), Ok(23));
    }

    #[test]
    fn non_escaping_mutable_arrays_use_wasm_linear_memory() {
        let source = "(let [a (std.native.Arr/new 1 2 3)]
                        (std.native.Arr/set a 1 40)
                        (+ (std.native.Arr/get a 0)
                           (+ (std.native.Arr/get a 1) 1)))";
        assert_eq!(module(source).call_entry_i64(), Ok(42));
    }

    #[test]
    fn recursive_array_parameters_keep_their_linear_memory_representation() {
        let source = "(do
          (defn permute [values n]
            (if (= n 1)
              1
              (loop [i 0 count 0]
                (if (< i n)
                  (let [subtotal (permute values (- n 1))
                        j (if (= (mod n 2) 0) i 0)
                        left (std.native.Arr/get values j)
                        right (std.native.Arr/get values (- n 1))]
                    (std.native.Arr/set values j right)
                    (std.native.Arr/set values (- n 1) left)
                    (recur (+ i 1) (+ count subtotal)))
                  count))))
          (permute (std.native.Arr/new 0 1 2 3 4) 5))";
        assert_eq!(module(source).call_entry_i64(), Ok(120));
    }

    #[test]
    fn wasm_linear_arrays_preserve_bounds_errors() {
        assert_eq!(
            module("(std.native.Arr/get (std.native.Arr/new 1 2) 2)").call_entry_i64(),
            Err("array index out of bounds".into())
        );
        assert_eq!(
            module("(std.native.Arr/get (std.native.Arr/new 1 2) -1)").call_entry_i64(),
            Err("array index out of bounds".into())
        );
    }

    #[test]
    fn fixed_shape_numeric_objects_use_wasm_linear_memory() {
        let source = "(let [o (std.native.Obj/new \"a\" 19 \"b\" 2)]
                        (std.native.Obj/set o \"b\" 23)
                        (+ (std.native.Obj/get o \"a\")
                           (std.native.Obj/get o \"b\")))";
        assert_eq!(module(source).call_entry_i64(), Ok(42));
    }

    #[test]
    fn wasm_linear_objects_report_missing_numeric_keys() {
        assert_eq!(
            module("(std.native.Obj/get (std.native.Obj/new \"a\" 1) \"b\")").call_entry_i64(),
            Err("object key not found".into())
        );
    }

    #[test]
    fn persistent_nested_values_cross_scoped_handles_without_copy_on_read() {
        let source = "(loop [i 0 state {:left [1 2 3] :right {:count 0}} checksum 0]
                        (if (< i 10)
                          (let [next (assoc state :right {:count (+ i 1)})]
                            (recur (+ i 1) next
                                   (+ checksum (get (get next :right) :count))))
                          checksum))";
        assert_eq!(module(source).call_entry_i64(), Ok(55));
    }

    #[test]
    fn persistent_loop_virtualization_preserves_observable_old_versions() {
        let source = "(loop [i 0 state {:right {:count 0}} checksum 0]
                        (if (< i 10)
                          (let [old state
                                next (assoc state :right {:count (+ i 1)})]
                            (recur (+ i 1) next
                                   (+ checksum (get (get old :right) :count))))
                          checksum))";
        assert_eq!(module(source).call_entry_i64(), Ok(45));
    }

    #[test]
    fn persistent_loop_virtualization_materializes_observed_exit_state() {
        let source = "(loop [i 0 state {:right {:count 0}}]
                        (if (< i 10)
                          (recur (+ i 1)
                                 (assoc state :right {:count (+ i 1)}))
                          (get (get state :right) :count)))";
        assert_eq!(module(source).call_entry_i64(), Ok(10));
    }

    #[test]
    fn recursive_tree_calls_compile_as_direct_wasm_calls() {
        let source = "(do
          (defn bench-tree-walk [node]
            (if (std.native.Base/number? node)
              node
              (loop [i 0 acc 0]
                (if (< i (count node))
                  (recur (+ i 1) (+ acc (bench-tree-walk (nth node i))))
                  acc))))
          (let [tree [1 [2 3 4] [5 [6 7] 8]]]
            (loop [i 0 acc 0]
              (if (< i 2)
                (recur (+ i 1) (+ acc (bench-tree-walk tree)))
                acc))))";
        assert_eq!(module(source).call_entry_i64(), Ok(72));
    }

    #[test]
    fn nested_recursive_scalar_calls_keep_i64_parameters() {
        let source = "(do
          (defn tak [x y z]
            (if (<= x y)
              z
              (tak (tak (- x 1) y z)
                   (tak (- y 1) z x)
                   (tak (- z 1) x y))))
          (tak 18 12 6))";
        assert_eq!(module(source).call_entry_i64(), Ok(7));
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "whole_wasm/value_tests.rs"]
mod value_tests;
