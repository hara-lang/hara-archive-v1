#![cfg(all(feature = "direct-native", not(target_arch = "wasm32")))]

use hara_wasm::direct_native::NativeEngine;
use hara_wasm::native_cli::RuntimeBroker;
use hara_wasm::project;
use hara_wasm::Runtime;
use std::path::PathBuf;

fn enable_native(runtime: &mut Runtime) {
    runtime
        .set_execution_backend("direct-native")
        .expect("direct-native backend must be available in this test build");
}

#[test]
fn native_cli_broker_evaluates_without_tracing() {
    let broker = RuntimeBroker::start_with_backend(None, false, false, false, "direct-native")
        .expect("native CLI broker must accept the native backend");
    assert_eq!(broker.eval("ROOT", "(+ 40 2)").unwrap(), "42");
}

#[test]
fn runtime_backend_executes_ordinary_hara_functions_in_the_vm() {
    let mut runtime = Runtime::core();
    assert_eq!(runtime.execution_backend(), "interpreter");
    enable_native(&mut runtime);
    assert_eq!(runtime.execution_backend(), "direct-native");
    runtime
        .eval_direct_native("(defn increment [value] (+ value 1))")
        .unwrap();
    assert_eq!(
        runtime
            .eval_direct_native("(let [value 20] (+ value 22))")
            .unwrap(),
        "42"
    );
    assert_eq!(runtime.eval_direct_native("(increment 41)").unwrap(), "42");
    let telemetry = runtime.native_execution_telemetry();
    assert!(telemetry.bytecode_functions > 0);
    assert!(telemetry.bytecode_instructions > 0);
    assert!(telemetry.native_target_calls > 0);
}

#[test]
fn direct_native_preserves_the_long_bigint_boundary() {
    let mut runtime = Runtime::core();
    enable_native(&mut runtime);
    assert_eq!(
        runtime
            .eval_direct_native("[(std.native.Base/long? 9223372036854775807) (std.native.Base/type 9223372036854775808)]")
            .unwrap(),
        "[true :std.native.BigInteger]"
    );
    assert_eq!(
        runtime
            .eval_direct_native("(+ 9223372036854775807 1)")
            .unwrap(),
        "9223372036854775808"
    );
}

#[test]
fn direct_native_keeps_the_disj_core_intrinsic_available() {
    let mut runtime = Runtime::core();
    enable_native(&mut runtime);
    assert_eq!(
        runtime
            .eval_direct_native("(disj #{:removed} :removed)")
            .unwrap(),
        "#{}"
    );
}

#[test]
fn direct_native_guest_protocols_dispatch_without_tree_evaluator_reentry() {
    let mut runtime = Runtime::core();
    enable_native(&mut runtime);
    assert_eq!(
        runtime
            .eval_direct_native(
                "(do (defstruct Box [value]) \
                     (defprotocol BoxOps (read [self])) \
                     (extend-type Box BoxOps (read [self] (:value self))) \
                     (read (Box 42)))",
            )
            .unwrap(),
        "42"
    );
}

#[test]
fn bytes_new_matches_the_bytes_constructor_contract() {
    let mut runtime = Runtime::core();
    enable_native(&mut runtime);
    assert_eq!(
        runtime
            .eval_direct_native(
                "[(Bytes/count (Bytes/new 1 2 -3 255)) (Bytes/get (Bytes/new 1 2 -3) 2)]"
            )
            .unwrap(),
        "[4 253]"
    );
    let error = runtime
        .eval_direct_native("(Bytes/new 256)")
        .expect_err("Bytes/new must reject values outside the byte range");
    assert!(
        error.contains("bytes expects a value in the range -128..255"),
        "{error}"
    );
}

#[test]
fn runtime_direct_native_loader_handles_require_inside_a_native_frame() {
    let mut runtime = Runtime::core();
    runtime.register_resource(
        "example.direct-late-dependency",
        "(ns example.direct-late-dependency) (defn increment [value] (+ value 1))",
    );
    enable_native(&mut runtime);

    runtime
        .eval_direct_native("(require [example.direct-late-dependency :as dependency :lazy true])")
        .unwrap();
    runtime
        .eval_direct_native(
            "(defn load-late [value] (require [example.direct-late-dependency]) (dependency/increment value))",
        )
        .unwrap();
    assert_eq!(runtime.eval_direct_native("(load-late 41)").unwrap(), "42");
}

#[test]
fn direct_native_preserves_multimethods_across_evaluations_and_loads() {
    let mut runtime = Runtime::core();
    runtime.register_resource(
        "example.direct-multimethod",
        "(ns example.direct-multimethod)
         (defmulti classify (fn [value] value))
         (defmethod classify :ok [value] 42)",
    );
    enable_native(&mut runtime);

    runtime
        .eval_direct_native("(defmulti local-classify (fn [value] value))")
        .unwrap();
    runtime
        .eval_direct_native("(defmethod local-classify :ok [value] 41)")
        .unwrap();
    assert_eq!(
        runtime.eval_direct_native("(local-classify :ok)").unwrap(),
        "41"
    );

    runtime
        .eval_direct_native("(require [example.direct-multimethod :as example])")
        .unwrap();
    assert_eq!(
        runtime
            .eval_direct_native("(example/classify :ok)")
            .unwrap(),
        "42"
    );
}

#[test]
fn direct_native_compiles_async_try_finally_with_recur() {
    let runtime = Runtime::core();
    runtime
        .compile_bytecode(
            "(fn [resolve reject] (try (if true resolve reject) (catch Exception error error)))",
        )
        .expect("catch bindings must remain visible in nested functions");
    let program = runtime
        .compile_bytecode(
            "(fn ^:async [] (try (loop [] (do (std.foundation.coroutine/yield 1) (recur))) (finally nil)))",
        )
        .expect("async try/finally must compile to a direct program");
    let function = &program.functions[1];
    assert!(function.async_function);
    assert!(function
        .handlers
        .iter()
        .any(|handler| handler.finally.is_some()));
    assert!(function
        .code
        .iter()
        .any(|instruction| matches!(instruction, hara_wasm::vm::Instruction::Yield)));
}

#[test]
fn direct_native_keeps_unbound_reads_catchable_at_runtime() {
    let mut runtime = Runtime::core();
    enable_native(&mut runtime);
    assert_eq!(
        runtime
            .eval_direct_native("(try (missing-value) (catch Throwable error true))")
            .unwrap(),
        "true"
    );
}

#[test]
fn direct_native_preserves_exception_source_spans_without_marker_symbols() {
    let mut runtime = Runtime::core();
    enable_native(&mut runtime);
    assert_eq!(
        runtime
            .eval_direct_native(
                r#"(try
  (throw
    (ex :host {:value 1} :ex/message "boom"))
  (catch error
    (let [provenance (ex-provenance error)]
      [(:ex/created-at provenance)
       (:ex/throws provenance)])))"#,
            )
            .unwrap(),
        "[{:resource nil :column 5 :namespace \"user\" :line 3} [{:resource nil :column 3 :namespace \"user\" :line 2}]]"
    );
}

#[test]
fn async_direct_native_functions_preserve_the_promise_return_shape() {
    let mut runtime = Runtime::core();
    enable_native(&mut runtime);
    assert_eq!(
        runtime
            .eval_direct_native("(do (defn ^:async answer [] 42) (answer))")
            .unwrap(),
        "<promise>"
    );
    assert_eq!(
        runtime
            .eval_direct_native("(std.protocol.ideref.IDeref/deref (answer))")
            .unwrap(),
        "42"
    );
}

#[test]
fn direct_native_evaluates_dynamic_runtime_forms_without_falling_back() {
    let mut runtime = Runtime::core();
    enable_native(&mut runtime);
    assert_eq!(
        runtime
            .eval_direct_native("(Runtime/eval '(+ 1 2))")
            .unwrap(),
        "3"
    );
    assert_eq!(
        runtime
            .eval_direct_native("(Runtime/load-string \"(+ 19 23)\")")
            .unwrap(),
        "42"
    );
    assert_eq!(
        runtime
            .eval_direct_native("(do (Runtime/eval '(def dynamic-value 41)) dynamic-value)")
            .unwrap(),
        "41"
    );
    assert_eq!(
        runtime
            .eval_direct_native(
                "(do
                   (Runtime/eval '(defmulti dynamic-classify (fn [value] value)))
                   (Runtime/eval '(defmethod dynamic-classify :ok [value] 42))
                   (dynamic-classify :ok))",
            )
            .unwrap(),
        "42"
    );
}

#[test]
fn direct_native_eval_native_batches_macroexpansion_validation() {
    let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let project = project::read(&project_root).expect("core project must load");
    let catalog = project::source_catalog(&project).expect("core source catalog must load");
    let mut runtime = Runtime::core();
    runtime.register_source_catalog(&catalog);
    runtime
        .bootstrap_source_foundation()
        .expect("source Foundation must bootstrap");
    enable_native(&mut runtime);
    let source =
        "(ns example.direct-macro (:require [std.lib.collection :as collection]))
         [(try (do (macroexpand (quote (collection/select {:a 1} :a))) false) (catch Throwable error true))
          (try (do (macroexpand (quote (collection/select {} [:walk]))) false) (catch Throwable error true))
          (try (do (macroexpand (quote (collection/transform {} [:walk :a] inc))) false) (catch Throwable error true))
          (try (do (macroexpand (quote (collection/select {} [1.5]))) false) (catch Throwable error true))]";
    let result = runtime.eval_native(source);
    assert_eq!(result.unwrap(), "[true true true true]");
    assert_eq!(runtime.native_execution_telemetry().invocations, 1);
}

#[test]
fn direct_native_backend_executes_persisted_bytecode_artifacts() {
    let mut runtime = Runtime::core();
    enable_native(&mut runtime);
    let artifact = runtime
        .compile_bytecode_artifact("(+ 20 22)")
        .expect("bytecode artifact must compile");
    assert_eq!(runtime.eval_bytecode_artifact(&artifact).unwrap(), "42");
}

#[test]
fn native_engine_shares_telemetry_but_runtime_namespaces_are_isolated() {
    let engine = NativeEngine::new();
    let mut first = Runtime::with_native_engine(engine.clone());
    enable_native(&mut first);
    first
        .eval_direct_native("(defn shared [value] (+ value 1))")
        .unwrap();
    assert_eq!(
        first
            .eval_direct_native("(def isolated 41) isolated")
            .unwrap(),
        "41"
    );
    let first_telemetry = first.native_execution_telemetry();

    let mut second = Runtime::with_native_engine(engine);
    enable_native(&mut second);
    second
        .eval_direct_native("(defn shared [value] (+ value 1))")
        .unwrap();
    assert_eq!(
        second
            .eval_direct_native("(def isolated 42) isolated")
            .unwrap(),
        "42"
    );
    let after_definitions = second.native_execution_telemetry();
    assert_eq!(first.eval_direct_native("isolated").unwrap(), "41");
    let before_repeat = second.native_execution_telemetry();
    assert_eq!(
        second
            .eval_direct_native("(def isolated 42) isolated")
            .unwrap(),
        "42"
    );
    let after_repeat = second.native_execution_telemetry();
    assert!(
        after_repeat.bytecode_functions > before_repeat.bytecode_functions,
        "each native entry validates its bytecode unit"
    );
    assert!(
        after_definitions.bytecode_functions >= first_telemetry.bytecode_functions,
        "the shared engine should retain cumulative bytecode telemetry"
    );
    assert!(
        second.native_execution_telemetry().invocations > first_telemetry.invocations,
        "shared engine should record both runtime owners"
    );
}
