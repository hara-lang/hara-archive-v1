//! Corpus-driven conformance tests for `hara-specs-registry/01-lang/010-bytecode/draft/hal-bytecode-vm.edn`.
//!
//! Every case in `hara-specs-registry/01-lang/010-bytecode/draft/conformance/bytecode-vm.edn` is
//! executed exactly once:
//!
//! - `:display` — both the tree-walking evaluator and the VM must display
//!   exactly the expected string.
//! - `:error-category` — both paths must fail in the expected normalized
//!   category (see `super::error_category`).
//! - `:compile-error` — the VM compile path must fail with a message
//!   containing the marker; the tree evaluator is not consulted, because
//!   the spec makes compile-time rejection the canonical behavior the
//!   evaluator converges to later.

use super::{error_category, eval_source};
use crate::kernel::{self, Form};
use crate::Runtime;

fn entry<'a>(entries: &'a [(Form, Form)], key: &str) -> Option<&'a Form> {
    entries
        .iter()
        .find_map(|(candidate, value)| match candidate {
            Form::Keyword(name) if name == key => Some(value),
            _ => None,
        })
}

fn required<'a>(entries: &'a [(Form, Form)], key: &str, id: &str) -> &'a Form {
    entry(entries, key).unwrap_or_else(|| panic!(":{id} missing :{key}"))
}

#[test]
fn bytecode_vm_conformance_corpus() {
    let corpus = std::fs::read_to_string(crate::spec_registry::require(
        "01-lang/010-bytecode/draft/conformance/bytecode-vm.edn",
    ))
    .expect("bytecode VM conformance corpus is readable");
    let manifest = kernel::parse_forms(&corpus)
        .expect("bytecode VM conformance corpus parses")
        .remove(0);
    let Form::Map(manifest) = manifest else {
        panic!("bytecode VM conformance corpus must be a map")
    };
    match entry(&manifest, "runtime") {
        Some(Form::String(runtime)) => assert_eq!(runtime, "hara-bytecode-vm"),
        _ => panic!("corpus :runtime must be \"hara-bytecode-vm\""),
    }
    let Some(Form::Vector(cases)) = entry(&manifest, "cases") else {
        panic!("corpus :cases must be a vector")
    };
    // Guard against a truncated or emptied corpus.
    assert!(cases.len() >= 60, "corpus has only {} cases", cases.len());

    let mut display_cases = 0;
    let mut error_cases = 0;
    let mut compile_cases = 0;
    for case in cases {
        let Form::Map(case) = case else {
            panic!("every conformance case must be a map")
        };
        let id = match required(case, "id", "?") {
            Form::Keyword(name) => name.clone(),
            _ => panic!("case :id must be a keyword"),
        };
        let Form::String(source) = required(case, "source", &id) else {
            panic!(":{id} :source must be a string")
        };
        let Form::Map(expect) = required(case, "expect", &id) else {
            panic!(":{id} :expect must be a map")
        };
        if let Some(expectation) = entry(expect, "display") {
            display_cases += 1;
            let Form::String(expected) = expectation else {
                panic!(":{id} :display must be a string")
            };
            let reference = Runtime::new()
                .eval_native(source)
                .unwrap_or_else(|error| panic!(":{id} reference failed: {error}"));
            let vm = eval_source(source)
                .map(|value| value.display())
                .unwrap_or_else(|error| panic!(":{id} vm failed: {error}"));
            assert_eq!(&reference, expected, ":{id} reference display");
            assert_eq!(&vm, expected, ":{id} vm display");
        } else if let Some(expectation) = entry(expect, "error-category") {
            error_cases += 1;
            let Form::String(expected) = expectation else {
                panic!(":{id} :error-category must be a string")
            };
            let reference = Runtime::new()
                .eval_native(source)
                .expect_err(&format!(":{id} reference must fail"));
            let vm = eval_source(source).expect_err(&format!(":{id} vm must fail"));
            assert_eq!(error_category(&reference), expected, ":{id} reference");
            assert_eq!(error_category(&vm), expected, ":{id} vm");
        } else if let Some(expectation) = entry(expect, "compile-error") {
            compile_cases += 1;
            let Form::String(marker) = expectation else {
                panic!(":{id} :compile-error must be a string")
            };
            let error = Runtime::new()
                .compile_bytecode(source)
                .err()
                .unwrap_or_else(|| panic!(":{id} compiled successfully"));
            assert!(
                error.to_string().contains(marker.as_str()),
                ":{id} compile error `{error}` lacks marker `{marker}`"
            );
        } else {
            panic!(":{id} has an unknown :expect shape")
        }
    }
    assert_eq!(
        display_cases + error_cases + compile_cases,
        cases.len(),
        "every case ran exactly one expectation"
    );
    assert!(display_cases > 0 && error_cases > 0 && compile_cases > 0);
}
