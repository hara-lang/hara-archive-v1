use crate::{core, Runtime};
use std::collections::BTreeSet;

const CORPUS: &str = include_str!(
    "../../../../hara-specs-registry/01-lang/001-language/draft/conformance/fixtures/native_behavioral.hal"
);

fn corpus_methods() -> BTreeSet<String> {
    let mut runtime = Runtime::new();
    let value = runtime
        .eval_native_value(&format!("{CORPUS}\n(native-method-keys)"))
        .expect("native corpus keys must evaluate");
    let core::Value::Vector(values) = value else {
        panic!("native-method-keys must return a vector");
    };
    let methods = values
        .iter()
        .map(core::Value::display)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        values.len(),
        methods.len(),
        "duplicate native corpus method"
    );
    assert!(!methods.is_empty(), "native corpus must not be empty");
    methods
}

fn live_methods() -> BTreeSet<String> {
    core::NATIVE_TYPES
        .iter()
        .flat_map(|(native_type, methods)| {
            methods
                .iter()
                .map(move |method| format!("{native_type}/{method}"))
        })
        .collect()
}

fn closure_pass(methods: &BTreeSet<String>) -> bool {
    let literal = methods
        .iter()
        .map(|method| format!("'{method}"))
        .collect::<Vec<_>>()
        .join(" ");
    let mut runtime = Runtime::new();
    runtime
        .eval_text(&format!(
            "{CORPUS}\n(get (native-closure-report [{literal}]) :pass)"
        ))
        .expect("native closure report must evaluate")
        == "true"
}

fn calibration_value(name: &str, field: &str) -> core::Value {
    let mut runtime = Runtime::new();
    runtime
        .eval_native_value(&format!(
            "{CORPUS}\n(get (get native-calibration-snippets :{name}) :{field})"
        ))
        .expect("native calibration snippet must evaluate")
}

fn calibration_source(name: &str) -> String {
    match calibration_value(name, "source") {
        core::Value::String(source) => source,
        value => panic!(
            "native calibration snippet must be a string: {}",
            value.display()
        ),
    }
}

fn calibration_expected(name: &str) -> String {
    calibration_value(name, "expected").display()
}

#[test]
fn specs_owned_native_corpus_closes_over_live_inventory_and_rejects_drift() {
    let mut runtime = Runtime::new();
    assert_eq!(
        "true",
        runtime
            .eval_text(&format!("{CORPUS}\n(native-corpus-valid?)"))
            .expect("native corpus validation must evaluate")
    );
    eprintln!(
        "native behavioral classifications {}",
        runtime
            .eval_text(&format!("{CORPUS}\n(native-classification-summary)"))
            .expect("native classification summary must evaluate")
    );

    let classified = corpus_methods();
    let live = live_methods();
    assert_eq!(live, classified);
    assert!(closure_pass(&live));

    let first = classified
        .iter()
        .next()
        .expect("classified native method")
        .clone();
    let mut removed = classified.clone();
    removed.remove(&first);
    assert!(!closure_pass(&removed));

    let mut added = classified.clone();
    added.insert("Unclassified/addition".to_owned());
    assert!(!closure_pass(&added));

    let mut renamed = classified;
    renamed.remove(&first);
    renamed.insert(format!("{first}-renamed"));
    assert!(!closure_pass(&renamed));
}

#[test]
fn evaluator_runs_every_specs_owned_classification_boundary_and_profile() {
    let methods = corpus_methods();
    let mut runtime = Runtime::new();
    let results = runtime
        .eval_text(&format!("{CORPUS}\n(native-method-results)"))
        .expect("shared native behavioral corpus must evaluate");
    assert!(!results.contains(":pass false"), "{results}");
    assert_eq!(methods.len(), results.matches(":pass true").count());

    let mut runtime = Runtime::new();
    assert_eq!(
        "true",
        runtime
            .eval_text(&format!(
                "{CORPUS}\n(every? (fn [case] (= true (get case :pass))) (native-boundary-results))"
            ))
            .expect("portable native boundary results must evaluate")
    );
    assert_eq!(
        "true",
        runtime
            .eval_text(&format!(
                "{CORPUS}\n(let [report (native-profile-report)] (and (= 0 (get report :failed)) (= (+ (get report :passed) (get report :failed) (get report :skipped)) (+ (get report :portable) (get report :capability-specific) (get report :inventory-only)))))"
            ))
            .expect("native profile accounting must evaluate")
    );
}

#[test]
fn evaluator_and_bytecode_use_the_specs_owned_calibration_probe() {
    let probe = calibration_source("evaluator-compiler");
    let expected = calibration_expected("evaluator-compiler");
    let mut runtime = Runtime::new();
    let interpreted = runtime
        .eval_text(&probe)
        .expect("evaluator native probe must run");
    let compiled = runtime
        .eval_bytecode_native(&probe)
        .expect("bytecode native probe must run");
    assert_eq!(expected, interpreted);
    assert_eq!(expected, compiled);
}

#[test]
fn rust_runs_the_specs_owned_identity_fast_path_calibration() {
    let source = calibration_source("base-identity-fast-paths");
    let expected = calibration_expected("base-identity-fast-paths");
    let mut runtime = Runtime::new();
    assert_eq!(
        expected,
        runtime
            .eval_text(&source)
            .expect("identity fast-path calibration must run")
    );
}
