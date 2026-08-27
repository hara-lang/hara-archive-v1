//! Differential tests: every supported form runs through the existing
//! evaluator (`Runtime::eval_native`) and the bytecode VM, and the
//! results must agree. Successes compare displayed values; failures
//! compare normalized error categories, because the two paths detect some
//! misuse at different stages (compile time vs runtime) and phrase
//! positions differently.

use super::{compile_source_with, error_category, eval_source, execute_program_with_globals};
use crate::core::Value;
use crate::kernel::Form;
use crate::Runtime;
use std::path::PathBuf;
use std::rc::Rc;

/// `runtime` is shared across a test's forms to avoid a std.foundation
/// bootstrap per form (~0.3s each in a debug build, which dominated
/// this suite's runtime). Safe here only because these forms never
/// depend on cross-form namespace state: `def`/`defn` side effects
/// LEAK across `eval_native` calls (a redefined var mutates the shared
/// cell earlier closures captured), so any form that interns globals —
/// or could observe one interned earlier — needs a fresh Runtime. The
/// one interning form below (the defn arity error) is never referenced
/// by later forms.
fn differential(runtime: &mut Runtime, source: &str) {
    let reference = runtime.eval_native(source);
    let registry = crate::embedding_namespace_registry();
    let vm = compile_source_with(source, &registry)
        .map_err(|error| error.to_string())
        .and_then(|program| {
            execute_program_with_globals(Rc::new(program), &registry)
                .map(|value| value.display())
                .map_err(|error| error.to_string())
        });
    match (&reference, &vm) {
        (Ok(expected), Ok(actual)) => {
            assert_eq!(expected, actual, "value divergence for {source}")
        }
        (Err(expected), Err(actual)) => {
            let bytecode_message = actual.split(" [line ").next().unwrap_or(actual);
            if expected != bytecode_message {
                assert_eq!(
                    error_category(expected),
                    error_category(actual),
                    "error category divergence for {source}: {expected} vs {actual}"
                );
            }
        }
        _ => panic!("divergence for {source}: reference {reference:?} vs vm {vm:?}"),
    }
}

fn shared_runtime_corpus_path() -> PathBuf {
    crate::spec_registry::require("01-lang/001-language/draft/conformance/parity/jvm-truffle.edn")
}

fn shared_core_language_corpus_path() -> PathBuf {
    shared_runtime_corpus_path()
        .parent()
        .and_then(std::path::Path::parent)
        .expect("language conformance directory")
        .join("core.edn")
}

fn map_value<'a>(entries: &'a [(Form, Form)], key: &str) -> Option<&'a Form> {
    entries.iter().find_map(|(candidate, value)| {
        matches!(candidate, Form::Keyword(name) if name == key).then_some(value)
    })
}

#[test]
fn shared_jvm_truffle_rust_runtime_corpus_matches() {
    let path = shared_runtime_corpus_path();
    let source = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read shared runtime corpus {}: {error}", path.display()));
    let forms = crate::kernel::parse_forms(&source).expect("parse shared runtime corpus");
    let [Form::Map(root)] = forms.as_slice() else {
        panic!("shared runtime corpus must contain one map");
    };
    let Some(Form::Vector(cases)) = map_value(root, "cases") else {
        panic!("shared runtime corpus must contain :cases");
    };
    let mut compared = 0usize;
    for case in cases {
        let Form::Map(case) = case else {
            panic!("shared runtime corpus cases must be maps");
        };
        if !matches!(map_value(case, "classification"), Some(Form::Keyword(value)) if value == "portable")
        {
            continue;
        }
        let Some(Form::Keyword(id)) = map_value(case, "id") else {
            panic!("shared runtime corpus case is missing :id");
        };
        let Some(Form::String(source)) = map_value(case, "source") else {
            panic!("shared runtime corpus case :{id} is missing :source");
        };
        let mut evaluator = Runtime::new();
        let reference = evaluator.eval_native(source);
        let registry = crate::embedding_namespace_registry();
        let bytecode: Result<String, String> = compile_source_with(source, &registry)
            .map_err(|error| error.to_string())
            .and_then(|program| {
                execute_program_with_globals(Rc::new(program), &registry)
                    .map(|value| value.display())
                    .map_err(|error| error.to_string())
            });
        match (&reference, &bytecode) {
            (Ok(expected), Ok(actual)) => assert_eq!(expected, actual, "corpus case :{id}"),
            (Err(expected), Err(actual)) => assert_eq!(
                error_category(expected),
                error_category(actual),
                "corpus case :{id}: evaluator={expected}, bytecode={actual}"
            ),
            _ => {
                panic!("corpus case :{id} diverged: evaluator={reference:?}, bytecode={bytecode:?}")
            }
        }
        compared += 1;
    }
    assert!(compared >= 30, "shared runtime corpus unexpectedly shrank");
}

#[test]
fn supported_forms_match_the_existing_evaluator() {
    let sources = [
        // Required by the issue.
        "42",
        "(+ 19 23)",
        "(if (< 19 20) 42 0)",
        "(let [x 19 y 23] (+ x y))",
        "(loop [i 0 acc 0] (if (< i 5000) (recur (+ i 1) (+ acc (mod i 17))) acc))",
        // Literals.
        "nil",
        "true",
        "false",
        "-9223372036854775808",
        "9223372036854775807",
        "1.5",
        "\"hello world\"",
        ":keyword",
        ":hara/namespaced",
        "\\newline",
        "\\a",
        "()",
        "1 2 3",
        // Arithmetic and comparisons, including edge semantics.
        "(+ 1 2 3 4 5)",
        "(+ 7)",
        "(- 10 1 2 3)",
        "(* 2 3 4)",
        "(/ 100 4 5)",
        "(mod 100 7)",
        "(mod -7 3)",
        "(< 1 2 3 4)",
        "(<= 1 1 2)",
        "(> 4 3 2 1)",
        "(>= 3 3 2)",
        "(= 42 42 42)",
        "(= 1 1.0)",
        "(= nil nil)",
        "(= :a :a)",
        "(= \"x\" \"x\" \"y\")",
        // Truthiness.
        "(if 0 1 2)",
        "(if \"\" 1 2)",
        "(if nil 1)",
        "(if false 1)",
        // Locals.
        "(let [x 1] (let [x 2] x))",
        "(let [x 1] (do (let [x 2] x) x))",
        "(let [x 1 y (+ x 1) z (+ y 1)] z)",
        "(let [x 1 x (+ x 1)] x)",
        "(do 1 2 3)",
        "(do)",
        "(let [a 1 b 2 c 3] (+ a (- b c)))",
        // Loops.
        "(loop [i 0] (if (< i 0) (recur (+ i 1)) i))",
        "(loop [i 0] (if (< i 1) (recur (+ i 1)) i))",
        "(loop [i 0 acc 1] (if (< i 10) (recur (+ i 1) (* acc 2)) acc))",
        "(loop [x 0 y 1] (if (< x 5) (recur (+ x 1) (+ x y)) y))",
        "(loop [x 1 y 2 n 0] (if (< n 3) (recur y x (+ n 1)) (- x y)))",
        "(loop [i 0] (let [next (+ i 1)] (if (< i 5) (do (recur next)) i)))",
        "(loop [i 0 t 0] (if (< i 4) (recur (+ i 1) (+ t (loop [j 0 s 0] (if (< j 3) (recur (+ j 1) (+ s (* i j))) s)))) t))",
        "(loop [i 0] (if (>= i 3) i (recur (+ i 2))))",
        "(loop [] 7)",
        "(loop [i 0] 1 2)",
        "(loop [i 0] (+ i 1) i)",
        // Functions, closures, and defn lowering.
        "((fn [x] x) 1)",
        "((fn [x y] (+ x y)) 19 23)",
        "((fn [] 42))",
        "(let [f (fn [x] (+ x 1))] (f 41))",
        "(let [x 19] ((fn [y] (+ x y)) 23))",
        "(let [x 1 f (fn [] x)] (let [x 2] (+ (f) x)))",
        "(((fn [x] (fn [y] (+ x y))) 19) 23)",
        "(loop [i 0 acc 0] (if (< i 5) (recur (+ i 1) ((fn [x] (+ x i)) acc)) acc))",
        "(do (defn f [x] (+ x 1)) (f 41))",
        "(do (defn f [x] (+ x 1)) (defn f [x] (+ x 2)) (f 40))",
        "(do (defn g [x] (* x 2)) (defn h [x] (+ (g x) 1)) (h 20))",
        "(do (defn countdown [n] (if (< n 1) 0 (+ 1 (countdown (- n 1))))) (countdown 100))",
        "(defn f [x] (+ x 1)) (f 41)",
        // Exceptions (issue #203).
        "(try (throw 41) (catch Exception error (+ error 1)))",
        "(try (throw :failed) (catch error error))",
        "(try (throw 7) (catch e (+ e 1)))",
        "(try (throw 41) (catch Exception a 41) (catch Exception b 42))",
        "(try (throw 41) (catch :problem/value error 0) (catch Exception error (+ error 1)))",
        "(try 7 (catch Exception e 0))",
        "(try (/ 1 0) (catch Exception error error))",
        "(try 42 (finally 0))",
        "(try 42 43 (finally 0 1))",
        "(try (throw 41) (catch Exception error (+ error 1)) (finally 0))",
        "(try (try (throw :original) (finally 0)) (catch Exception e e))",
        "(try (try (throw 41) (catch :problem/value error 0) (finally 0)) (catch Exception error (+ error 1)))",
        "(try ((fn [] (throw 41))) (catch Exception e (+ e 1)))",
        "(try ((fn [] (/ 1 0))) (catch Exception e e))",
        "((fn [] (try (throw 1) (catch Exception e 42))))",
        "(loop [i 0] (try (if (< i 3) (recur (+ i 1)) i) (catch Exception e -1)))",
        "(loop [i 0] (try (throw 1) (catch Exception e (if (< i 3) (recur (+ i 1)) i))))",
    ];
    let mut runtime = Runtime::new();
    for source in sources {
        differential(&mut runtime, source);
    }
}

#[test]
fn structured_error_code_catches_match_in_evaluator_and_bytecode() {
    let source = "(try (throw (std.foundation/ex :file/not-found {:ex/message \"missing\"})) \
           (catch :socket/closed error :wrong) \
           (catch [:file/not-found :file/permission-denied] error :file-error))";
    let mut runtime = Runtime::new();
    assert_eq!(runtime.eval_native(source).unwrap(), ":file-error");

    let registry = crate::embedding_namespace_registry();
    let program = compile_source_with(source, &registry).unwrap();
    assert_eq!(
        execute_program_with_globals(Rc::new(program), &registry)
            .unwrap()
            .display(),
        ":file-error"
    );
    let provenance_source = "(let [exception (std.foundation/ex :test/provenance {})] \
           (try (throw exception) \
             (catch caught (count (:ex/throws (std.foundation/ex-provenance caught))))))";
    assert_eq!(runtime.eval_native(provenance_source).unwrap(), "1");
    let program = compile_source_with(provenance_source, &registry).unwrap();
    assert_eq!(
        execute_program_with_globals(Rc::new(program), &registry)
            .unwrap()
            .display(),
        "1"
    );
}

#[test]
fn supported_form_errors_match_the_existing_evaluator() {
    let sources = [
        "(/ 1 0)",
        "(% 1 0)",
        "(mod 1 0)",
        "(+ 9223372036854775807 1)",
        "(* 4611686018427387904 2)",
        "(- -9223372036854775808 1)",
        "(+ 1 1.5)",
        "(+ \"a\" 1)",
        "(< 1 \"a\")",
        "(< 1)",
        "(= 1)",
        "(+)",
        "(mod)",
        "(if)",
        "(if 1 2 3 4)",
        "(let [x] x)",
        "(let 1 x)",
        "(loop [i] i)",
        "(loop [i 0])",
        "(loop 1 2)",
        "(let)",
        "unknown",
        "(let [x 1] (+ x y))",
        "(recur 1)",
        "(loop [i 0] (recur 1 2))",
        "(+ 1",
        "((fn [x] x) 1 2)",
        "((fn [x] x))",
        "(1 2)",
        "(do (defn f [x y] (+ x y)) (f 1))",
        // Exceptions (issue #203).
        "(throw :failed)",
        "(try (throw 41) (catch :problem/value error 0))",
        "(try (try (throw 41) (catch :problem/value error 0)) (catch :problem/value error 0))",
        "(try 1 (finally (throw 2)))",
        "(try (throw 1) (finally (throw 2)))",
        "(throw)",
        "(try (throw 1) (catch Exception e (throw 2)))",
    ];
    let mut runtime = Runtime::new();
    for source in sources {
        differential(&mut runtime, source);
    }
}

#[test]
fn recur_tail_tightening_is_a_documented_divergence() {
    // The langspec restricts recur to tail positions; the evaluator
    // detects some violations only at runtime (or, for truthiness
    // misuses, silently). The VM rejects them at compile time. Both
    // paths agree on the cases in the supported corpus above; these
    // cases are where detection timing legitimately differs.
    let reference = Runtime::new().eval_native("(loop [i 0] (+ 1 (recur 2)))");
    assert!(reference.is_err(), "{reference:?}");
    assert!(eval_source("(loop [i 0] (+ 1 (recur 2)))").is_err());
}

/// Reads the shared benchmark corpus and runs every workload whose
/// source is inside the supported subset, exactly as written in
/// `hara-benchmarks/runtime/hara/runtime/workloads.json`.
#[test]
fn shared_benchmark_workloads_match() {
    let path = concat!(
        env!("HARA_SOURCE_ROOT"),
        "/../../../../website/hara-benchmarks/runtime/hara/runtime/workloads.json"
    );
    let text = std::fs::read_to_string(path).expect("workloads.json must exist");
    let parsed = crate::json::read(&text).expect("workloads.json parses");
    let entries = crate::core::map_entries(&parsed).expect("top-level object");
    let workloads = entries
        .iter()
        .find(|(key, _)| matches!(key, Value::String(name) if name == "workloads"))
        .map(|(_, value)| value)
        .expect("workloads key");
    let Value::Vector(workloads) = workloads else {
        panic!("workloads must be a vector")
    };
    let supported = [
        "noop",
        "arithmetic",
        "function-call",
        "persistent-vector",
        "persistent-map",
        "sequence-navigation",
    ];
    let mut seen = Vec::new();
    let mut runtime = Runtime::new();
    for workload in workloads.iter() {
        let fields = crate::core::map_entries(workload).expect("workload object");
        let field = |name: &str| {
            fields
                .iter()
                .find(|(key, _)| matches!(key, Value::String(key) if key == name))
                .and_then(|(_, value)| match value {
                    Value::String(text) => Some(text.clone()),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("workload missing {name}"))
        };
        let id = field("id");
        if !supported.contains(&id.as_str()) {
            continue;
        }
        seen.push(id.clone());
        let source = field("source");
        let expected = field("expected");
        let reference = runtime.eval_native(&source).expect("reference evaluates");
        let vm = eval_source(&source)
            .map(|value| value.display())
            .expect("vm evaluates");
        assert_eq!(reference, expected, "{id} reference mismatch");
        assert_eq!(vm, expected, "{id} vm mismatch");
    }
    assert_eq!(
        seen, supported,
        "corpus must contain the supported workloads"
    );
}

/// Runtime-based differential (issue #223): the VM compiles against the
/// Runtime's namespace registry, so std.foundation vars are visible and
/// globals intern into the runtime. Stateful sources get a fresh
/// Runtime per side — def side effects leak across evals on a shared
/// Runtime by design (REPL semantics).
fn runtime_differential(source: &str) {
    let reference = Runtime::new().eval_native(source);
    let vm = Runtime::new().eval_bytecode_native(source);
    match (&reference, &vm) {
        (Ok(expected), Ok(actual)) => {
            assert_eq!(expected, actual, "value divergence for {source}")
        }
        (Err(expected), Err(actual)) => assert_eq!(
            error_category(expected),
            error_category(actual),
            "error category divergence for {source}: {expected} vs {actual}"
        ),
        _ => panic!("divergence for {source}: reference {reference:?} vs vm {vm:?}"),
    }
}

#[test]
fn clojure_set_conversion_matches_in_both_runtimes() {
    runtime_differential("(= (set [1 2 1]) #{1 2})");
}

#[test]
fn callable_var_namespace_cases_match_shared_spec() {
    fn entry<'a>(
        entries: &'a [(crate::kernel::Form, crate::kernel::Form)],
        key: &str,
    ) -> Option<&'a crate::kernel::Form> {
        entries.iter().find_map(|(candidate, value)| {
            matches!(candidate, crate::kernel::Form::Keyword(name) if name == key).then_some(value)
        })
    }

    let path = shared_core_language_corpus_path()
        .parent()
        .expect("language conformance directory")
        .join("modules.edn");
    let source = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read module corpus {}: {error}", path.display()));
    let manifest = crate::kernel::parse_forms(&source)
        .expect("module conformance corpus parses")
        .remove(0);
    let crate::kernel::Form::Map(manifest) = manifest else {
        panic!("module conformance corpus must be a map")
    };
    let Some(crate::kernel::Form::Vector(cases)) = entry(&manifest, "cases") else {
        panic!("module conformance corpus must declare :cases")
    };

    for id in [
        "namespace/callable-var-precedence",
        "namespace/callable-var-lexical-shadow",
        "namespace/callable-var-late-binding",
        "namespace/referred-var-shadowed",
    ] {
        let case = cases
            .iter()
            .find_map(|case| match case {
                crate::kernel::Form::Map(entries)
                    if matches!(entry(entries, "id"), Some(crate::kernel::Form::Keyword(candidate)) if candidate == id) =>
                {
                    Some(entries)
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("missing shared module case :{id}"));
        let Some(crate::kernel::Form::String(setup)) = entry(case, "setup") else {
            panic!(":{id} must declare string :setup")
        };
        let Some(crate::kernel::Form::String(source)) = entry(case, "source") else {
            panic!(":{id} must declare string :source")
        };
        let Some(crate::kernel::Form::Map(expect)) = entry(case, "expect") else {
            panic!(":{id} must declare :expect")
        };
        let mut runtime = Runtime::new();
        runtime
            .eval_native(setup)
            .unwrap_or_else(|error| panic!(":{id} setup failed: {error}"));
        if let Some(crate::kernel::Form::String(expected)) = entry(expect, "display") {
            assert_eq!(
                runtime
                    .eval_bytecode_native(source)
                    .unwrap_or_else(|error| panic!(":{id} VM failed: {error}")),
                *expected,
                ":{id}"
            );
        } else if let Some(crate::kernel::Form::String(marker)) = entry(expect, "error-contains") {
            let error = runtime
                .eval_bytecode_native(source)
                .expect_err(&format!(":{id} VM must fail"));
            assert!(error.contains(marker), ":{id}: {error}");
        } else {
            panic!(":{id} has unsupported expectation")
        }
    }
}

/// Namespace- and arity-dependent cases from the normative core-language corpus
/// (`hara-specs-registry/01-lang/001-language/draft/conformance/core.edn`),
/// deferred by milestones 2-3 until globals existed (issue #223).
#[test]
fn core_language_namespace_corpus_cases_match() {
    fn entry<'a>(
        entries: &'a [(crate::kernel::Form, crate::kernel::Form)],
        key: &str,
    ) -> Option<&'a crate::kernel::Form> {
        entries.iter().find_map(|(candidate, value)| {
            matches!(candidate, crate::kernel::Form::Keyword(name) if name == key).then_some(value)
        })
    }

    let supported = [
        "error/catch-order",
        "error/unmatched-catch",
        "error/finally-normal",
        "error/finally-unwind",
        "runtime/set-var-root",
        "compiler/declare-private",
        "definition/doc-metadata",
        "definition/arglists-metadata",
        "function/variadic-arity",
        "function/multiple-arities",
    ];
    let path = shared_core_language_corpus_path();
    let source = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read core-language corpus {}: {error}", path.display()));
    let manifest = crate::kernel::parse_forms(&source)
        .expect("Core-language conformance corpus parses")
        .remove(0);
    let crate::kernel::Form::Map(manifest) = manifest else {
        panic!("Core-language conformance corpus must be a map")
    };
    let Some(crate::kernel::Form::Vector(cases)) = entry(&manifest, "cases") else {
        panic!("Core-language conformance corpus must declare :cases")
    };
    for case in cases {
        let crate::kernel::Form::Map(case) = case else {
            panic!("core-language cases must be maps")
        };
        let Some(crate::kernel::Form::Keyword(id)) = entry(case, "id") else {
            panic!("core-language case must declare :id")
        };
        let is_namespace = matches!(
            entry(case, "class"),
            Some(crate::kernel::Form::Keyword(class)) if class == "namespace"
        );
        if !is_namespace && !supported.contains(&id.as_str()) {
            continue;
        }
        let Some(crate::kernel::Form::String(source)) = entry(case, "source") else {
            panic!(":{id} must declare string :source")
        };
        if is_namespace {
            let Some(crate::kernel::Form::Map(expect)) = entry(case, "expect") else {
                panic!(":{id} must declare :expect")
            };
            let actual = Runtime::new().eval_bytecode_native(source);
            if let Some(crate::kernel::Form::String(expected)) = entry(expect, "message") {
                let error = actual.expect_err(&format!(":{id} must fail"));
                assert!(
                    error
                        .to_ascii_lowercase()
                        .contains(&expected.to_ascii_lowercase()),
                    ":{id}: {error}"
                );
            } else if let Some(expected) = entry(expect, "value") {
                assert_eq!(
                    actual.unwrap_or_else(|error| panic!(":{id} failed: {error}")),
                    expected.to_string(),
                    ":{id}"
                );
            } else {
                actual.unwrap_or_else(|error| panic!(":{id} failed: {error}"));
            }
            continue;
        }
        runtime_differential(source);
    }
}

/// A Runtime sees its own interned vars across mixed evaluator/VM evals.
#[test]
fn runtime_globals_interop_issue_223() {
    let mut runtime = Runtime::new();
    assert_eq!(
        runtime.eval_bytecode_native("(defn f [x] (+ x 1))"),
        Ok("#'user/f".into())
    );
    // The tree evaluator sees the var the VM interned...
    assert_eq!(runtime.eval_native("(f 41)"), Ok("42".into()));
    // ...and the VM sees vars the evaluator interned.
    assert_eq!(
        runtime.eval_native("(defn g [x] (+ x 2))"),
        Ok("#'user/g".into())
    );
    assert_eq!(runtime.eval_bytecode_native("(g 40)"), Ok("42".into()));
}
