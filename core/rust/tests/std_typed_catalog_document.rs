use hara_wasm::{spec_registry, Runtime};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

const FIXTURE_PATH: &str = "01-lang/011-typed-catalog/draft/conformance/catalog-v2.json";
const STALE_HASH: &str = "sha256:07304c8b522dece5a0fb44ba26a9489435fd8bb79c3f28640733dbfe81ffb65f";
const FORGED_COMPONENT: &str =
    "sha256:0932e3b99be0a918adc4adc939bef7c0966c77a0007b86afd9a47fe732d7f01d";

fn fixture_path() -> PathBuf {
    spec_registry::require(FIXTURE_PATH)
}

fn fixture_text() -> String {
    let path = fixture_path();
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}

fn mutated_fixture(mutate: impl FnOnce(&mut Value)) -> String {
    let mut document: Value =
        serde_json::from_str(&fixture_text()).expect("parse published catalog fixture");
    mutate(&mut document);
    serde_json::to_string_pretty(&document).expect("serialize mutated catalog fixture")
}

fn hara_string(value: &str) -> String {
    serde_json::to_string(value).expect("encode fixture as one Hara string")
}

fn evaluate(source: &str) -> String {
    let source = source.to_owned();
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(move || {
            let mut runtime = Runtime::new();
            for (namespace, resource) in [
                ("std.typed", include_str!("../../lib/src/std/typed.hal")),
                (
                    "std.typed.catalog",
                    include_str!("../../lib/src/std/typed/catalog.hal"),
                ),
                (
                    "std.typed.catalog.document",
                    include_str!("../../lib/src/std/typed/catalog/document.hal"),
                ),
                (
                    "std.typed.explain",
                    include_str!("../../lib/src/std/typed/explain.hal"),
                ),
                (
                    "std.typed.infer",
                    include_str!("../../lib/src/std/typed/infer.hal"),
                ),
                (
                    "std.typed.registry",
                    include_str!("../../lib/src/std/typed/registry.hal"),
                ),
                (
                    "std.typed.schema",
                    include_str!("../../lib/src/std/typed/schema.hal"),
                ),
            ] {
                runtime.register_resource(namespace, resource);
            }
            runtime.eval_native(&source)
        })
        .expect("start Hara runtime thread")
        .join()
        .expect("Hara runtime thread must not panic")
        .unwrap_or_else(|error| panic!("evaluate registry fixture through Hara: {error}"))
}

fn verify_expression(fixture: &str, body: &str) -> String {
    evaluate(&format!(
        "(do \
           (require 'std.typed.catalog.document) \
           (require 'std.typed) \
           (let [verified (std.typed/catalog-document-verify-json {})] {}))",
        hara_string(fixture),
        body
    ))
}

fn rejection_expression(fixture: &str) -> String {
    evaluate(&format!(
        "(do \
           (require 'std.typed.catalog.document) \
           (require 'std.typed) \
           (try \
             (std.typed/catalog-document-verify-json {}) \
             :unexpected-success \
             (catch Throwable error \
               [(:type (ex-data error)) \
                (:finding/type (ex-data error)) \
                (:cause/type (ex-data error))])))",
        hara_string(fixture)
    ))
}

#[test]
fn exact_registry_bytes_round_trip_through_std_typed_catalog() {
    let fixture = fixture_text();
    assert_eq!(
        verify_expression(
            &fixture,
            "(let [report (:verification verified) \
                   admitted (:catalog verified) \
                   account-coordinate (first (:catalog/coordinates report)) \
                   account (std.typed/catalog-resolve admitted account-coordinate) \
                   order (std.typed/catalog-dependency-order admitted)] \
               [(:status report) \
                (:catalog/entry-count report) \
                (:catalog/component-count report) \
                (= account-coordinate (:schema/coordinate account)) \
                (= 4 (count order)) \
                (= (:catalog/component-order report) \
                   (vec \
                    (map :component/id \
                         (std.typed/catalog-dependency-components admitted))))])"
        ),
        "[:verified 4 4 true true true]"
    );
}

#[test]
fn stale_published_hash_is_rejected_by_the_canonical_hara_catalog() {
    let fixture = mutated_fixture(|document| {
        document["catalog/entries"][0]["schema/hash"] = Value::String(STALE_HASH.to_owned());
        document["catalog/entries"][0]["schema/coordinate"][2] =
            Value::String(STALE_HASH.to_owned());
    });
    assert_eq!(
        rejection_expression(&fixture),
        "[:std.typed.catalog/invalid-document \
          :std.typed.catalog.document/catalog-rejected \
          :std.typed.catalog/invalid-catalog]"
    );
}

#[test]
fn stale_exact_dependency_is_rejected_after_catalog_recomputation() {
    let fixture = mutated_fixture(|document| {
        document["catalog/entries"][0]["schema/dependencies"][0][2] =
            Value::String(STALE_HASH.to_owned());
    });
    assert_eq!(
        rejection_expression(&fixture),
        "[:std.typed.catalog/invalid-document \
          :std.typed.catalog.document/schema-dependencies-mismatch \
          nil]"
    );
}

#[test]
fn forged_component_evidence_is_rejected_after_catalog_recomputation() {
    let fixture = mutated_fixture(|document| {
        document["catalog/components"][3]["component/id"] =
            Value::String(FORGED_COMPONENT.to_owned());
        document["catalog/component-order"][3] = Value::String(FORGED_COMPONENT.to_owned());
    });
    assert_eq!(
        rejection_expression(&fixture),
        "[:std.typed.catalog/invalid-document \
          :std.typed.catalog.document/component-evidence-mismatch \
          nil]"
    );
}

#[test]
fn unsupported_hash_epoch_fails_before_catalog_admission() {
    let fixture = mutated_fixture(|document| {
        document["catalog/hash-epoch"] = Value::String("std.typed.schema/catalog-v1".to_owned());
    });
    assert_eq!(
        rejection_expression(&fixture),
        "[:std.typed.catalog/invalid-document \
          :std.typed.catalog.document/hash-epoch-unsupported \
          nil]"
    );
}
