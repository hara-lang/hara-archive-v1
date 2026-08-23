use super::*;
use crate::kernel::halc_trace::{
    HalcArtifactTrace, HalcTraceEvent, HalcTraceStatus, HalcTraceValue,
};

const SOURCE: &str = "(ns demo.handoff) \
                     (def Customer [:map [:id :int]]) \
                     (defn customer-id [customer] customer)";

fn stage<'a>(trace: &'a HalcArtifactTrace, name: &str) -> &'a HalcTraceEvent {
    trace
        .events
        .iter()
        .find(|event| event.stage == name)
        .unwrap_or_else(|| panic!("missing stage {name}"))
}

#[test]
fn handoff_stage_matches_the_portable_halc_contract() {
    assert_eq!(HALC_BYTECODE_HANDOFF_STAGE, "handoff/bytecode");
}

#[cfg(feature = "bytecode-vm")]
#[test]
fn records_a_validated_non_executing_bytecode_handoff() {
    let trace =
        trace_halc_source_to_bytecode("handoff-1", "demo.handoff", "demo/handoff.hal", SOURCE);
    assert_eq!(trace.status, HalcTraceStatus::Ok);
    assert_eq!(
        trace.events.last().map(|event| event.stage),
        Some(HALC_BYTECODE_HANDOFF_STAGE)
    );

    let handoff = stage(&trace, HALC_BYTECODE_HANDOFF_STAGE);
    assert_eq!(handoff.status, HalcTraceStatus::Ok);
    assert_eq!(
        handoff.evidence.get("handoff/supported"),
        Some(&HalcTraceValue::Boolean(true))
    );
    assert_eq!(
        handoff.evidence.get("handoff/status"),
        Some(&HalcTraceValue::String("ready".to_owned()))
    );
    assert_eq!(
        handoff.evidence.get("handoff/fallback"),
        Some(&HalcTraceValue::Boolean(false))
    );
    assert_eq!(
        handoff.evidence.get("handoff/executed"),
        Some(&HalcTraceValue::Boolean(false))
    );
    assert_eq!(
        handoff.evidence.get("handoff/module-namespace"),
        Some(&HalcTraceValue::String("demo.handoff".to_owned()))
    );
    assert_eq!(
        handoff.evidence.get("handoff/module-resource"),
        Some(&HalcTraceValue::String("demo/handoff.hal".to_owned()))
    );
    let result = trace.result.as_ref().expect("handoff result");
    assert_eq!(result.get("handoff/source-hash"), result.get("source/hash"));
    assert_eq!(
        handoff.evidence.get("handoff/program-namespace"),
        Some(&HalcTraceValue::String("demo.handoff".to_owned()))
    );
    assert_eq!(
        handoff.evidence.get("handoff/program-validated"),
        Some(&HalcTraceValue::Boolean(true))
    );
    assert_eq!(
        handoff.evidence.get("handoff/artifact-decodable"),
        Some(&HalcTraceValue::Boolean(true))
    );
    assert!(matches!(
        handoff.evidence.get("handoff/artifact-bytes"),
        Some(HalcTraceValue::Integer(bytes)) if *bytes > 0
    ));
    assert!(matches!(
        handoff.evidence.get("handoff/artifact-digest"),
        Some(HalcTraceValue::String(digest)) if digest.len() == 64
    ));
}

#[cfg(feature = "bytecode-vm")]
#[test]
fn unsupported_bytecode_forms_fail_at_the_handoff_without_fallback() {
    let trace = trace_halc_source_to_bytecode(
        "unsupported",
        "demo.unsupported",
        "demo/unsupported.hal",
        "(ns demo.unsupported) (require 'demo.other)",
    );
    assert_eq!(trace.status, HalcTraceStatus::Error);
    assert!(trace.result.is_none());
    let handoff = stage(&trace, HALC_BYTECODE_HANDOFF_STAGE);
    assert_eq!(handoff.status, HalcTraceStatus::Error);
    assert_eq!(
        handoff.evidence.get("handoff/supported"),
        Some(&HalcTraceValue::Boolean(true))
    );
    assert_eq!(
        handoff.evidence.get("handoff/status"),
        Some(&HalcTraceValue::String("failed".to_owned()))
    );
    assert_eq!(
        handoff.evidence.get("handoff/fallback"),
        Some(&HalcTraceValue::Boolean(false))
    );
    assert_eq!(
        handoff.evidence.get("diagnostic/category"),
        Some(&HalcTraceValue::String(
            "bytecode/unsupported-form".to_owned()
        ))
    );
    assert!(trace
        .error
        .as_deref()
        .is_some_and(|message| message.contains("unsupported operator: require")));
}

#[cfg(feature = "bytecode-vm")]
#[test]
fn bytecode_handoff_does_not_evaluate_top_level_forms() {
    let trace = trace_halc_source_to_bytecode(
        "no-execution",
        "demo.noexec",
        "demo/noexec.hal",
        "(ns demo.noexec) \
         (defn explode [] (throw \"must not execute\")) \
         (explode)",
    );
    assert_eq!(trace.status, HalcTraceStatus::Ok);
    let handoff = stage(&trace, HALC_BYTECODE_HANDOFF_STAGE);
    assert_eq!(
        handoff.evidence.get("handoff/executed"),
        Some(&HalcTraceValue::Boolean(false))
    );
}

#[test]
fn encoder_only_handoff_is_explicitly_unsupported() {
    let evidence = unsupported_handoff_evidence("demo.encoder-only", "demo/encoder_only.hal");
    assert_eq!(
        evidence.get("handoff/supported"),
        Some(&HalcTraceValue::Boolean(false))
    );
    assert_eq!(
        evidence.get("handoff/status"),
        Some(&HalcTraceValue::String("unsupported".to_owned()))
    );
    assert_eq!(
        evidence.get("handoff/fallback"),
        Some(&HalcTraceValue::Boolean(false))
    );
    assert_eq!(
        evidence.get("handoff/executed"),
        Some(&HalcTraceValue::Boolean(false))
    );
}
