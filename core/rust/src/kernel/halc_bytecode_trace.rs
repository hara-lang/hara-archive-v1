//! Explicit, non-executing HALC-to-bytecode handoff observations.
//!
//! The source trace remains available in encoder-only builds. When the
//! production bytecode compiler is present, this adapter lowers the decoded
//! HALC module, validates the resulting program, and round-trips its HBC
//! artifact. When it is absent, the final stage is retained as an explicit
//! `unsupported` observation. Neither path executes module code.

use super::halc_source_trace::{trace_halc_source_with_limits, HalcSourceTraceLimits};
use super::halc_trace::{
    HalcArtifactTrace, HalcTraceEvent, HalcTraceEvidence, HalcTraceStatus, HalcTraceValue,
};

pub const HALC_BYTECODE_HANDOFF_STAGE: &str = "handoff/bytecode";

pub fn trace_halc_source_to_bytecode(
    id: impl Into<String>,
    namespace: &str,
    resource: &str,
    source: &str,
) -> HalcArtifactTrace {
    trace_halc_source_to_bytecode_with_limits(
        id,
        namespace,
        resource,
        source,
        HalcSourceTraceLimits::default(),
    )
}

pub fn trace_halc_source_to_bytecode_with_limits(
    id: impl Into<String>,
    namespace: &str,
    resource: &str,
    source: &str,
    limits: HalcSourceTraceLimits,
) -> HalcArtifactTrace {
    let mut trace = trace_halc_source_with_limits(id, namespace, resource, source, limits);
    if trace.status == HalcTraceStatus::Error {
        return trace;
    }
    append_handoff(&mut trace, namespace, resource, source);
    trace
}

fn append_event(
    trace: &mut HalcArtifactTrace,
    status: HalcTraceStatus,
    evidence: HalcTraceEvidence,
    error: Option<String>,
) {
    let id = trace.events.len() as u64 + 1;
    trace.events.push(HalcTraceEvent {
        id,
        sequence: id,
        stage: HALC_BYTECODE_HANDOFF_STAGE,
        status,
        evidence,
        error,
    });
}

fn string(value: impl Into<String>) -> HalcTraceValue {
    HalcTraceValue::String(value.into())
}

fn integer(value: usize) -> HalcTraceValue {
    HalcTraceValue::Integer(value as u64)
}

fn boolean(value: bool) -> HalcTraceValue {
    HalcTraceValue::Boolean(value)
}

fn base_handoff_evidence(namespace: &str, resource: &str) -> HalcTraceEvidence {
    let mut evidence = HalcTraceEvidence::new();
    evidence.insert("handoff/module-namespace".into(), string(namespace));
    evidence.insert("handoff/module-resource".into(), string(resource));
    evidence.insert("handoff/fallback".into(), boolean(false));
    evidence.insert("handoff/executed".into(), boolean(false));
    evidence
}

fn unsupported_handoff_evidence(namespace: &str, resource: &str) -> HalcTraceEvidence {
    let mut evidence = base_handoff_evidence(namespace, resource);
    evidence.insert("handoff/supported".into(), boolean(false));
    evidence.insert("handoff/status".into(), string("unsupported"));
    evidence.insert(
        "handoff/reason".into(),
        string("bytecode-vm feature disabled"),
    );
    evidence
}

#[cfg(not(feature = "bytecode-vm"))]
fn append_handoff(trace: &mut HalcArtifactTrace, namespace: &str, resource: &str, _source: &str) {
    let evidence = unsupported_handoff_evidence(namespace, resource);
    append_event(trace, HalcTraceStatus::Ok, evidence.clone(), None);
    if let Some(result) = trace.result.as_mut() {
        result.extend(evidence);
    }
}

#[cfg(feature = "bytecode-vm")]
fn append_handoff(trace: &mut HalcArtifactTrace, namespace: &str, resource: &str, source: &str) {
    match compile_handoff(namespace, resource, source) {
        Ok(evidence) => {
            append_event(trace, HalcTraceStatus::Ok, evidence.clone(), None);
            if let Some(result) = trace.result.as_mut() {
                result.extend(evidence);
            }
        }
        Err(failure) => {
            let mut evidence = base_handoff_evidence(namespace, resource);
            evidence.insert("handoff/supported".into(), boolean(true));
            evidence.insert("handoff/status".into(), string("failed"));
            evidence.insert("diagnostic/category".into(), string(failure.category));
            evidence.insert("diagnostic/message".into(), string(&failure.message));
            append_event(
                trace,
                HalcTraceStatus::Error,
                evidence,
                Some(failure.message.clone()),
            );
            trace.status = HalcTraceStatus::Error;
            trace.result = None;
            trace.error = Some(failure.message);
        }
    }
}

#[cfg(feature = "bytecode-vm")]
struct HandoffFailure {
    category: &'static str,
    message: String,
}

#[cfg(feature = "bytecode-vm")]
impl HandoffFailure {
    fn new(category: &'static str, message: impl Into<String>) -> Self {
        Self {
            category,
            message: message.into(),
        }
    }
}

#[cfg(feature = "bytecode-vm")]
fn compile_handoff(
    namespace: &str,
    resource: &str,
    source: &str,
) -> Result<HalcTraceEvidence, HandoffFailure> {
    use sha2::{Digest, Sha256};

    let forms = super::parse_forms(source)
        .map_err(|error| HandoffFailure::new("bytecode/source-replay", error.to_string()))?;
    let halc_artifact = super::halc::encode_halc_module(namespace, resource, source, forms)
        .map_err(|error| HandoffFailure::new("bytecode/halc-encode", error))?;
    let module = super::halc::decode_halc(&halc_artifact)
        .map_err(|error| HandoffFailure::new("bytecode/halc-decode", error))?;
    let registry = compiler_registry();
    let program =
        crate::vm::compiler::compile_halc_module(&module, &registry).map_err(|error| {
            HandoffFailure::new(compile_failure_category(error.kind()), error.to_string())
        })?;
    crate::vm::validate::validate(&program)
        .map_err(|error| HandoffFailure::new("bytecode/validation", error.to_string()))?;

    if program.namespace.as_deref() != Some(module.namespace.as_str()) {
        return Err(HandoffFailure::new(
            "bytecode/provenance",
            "HALC and bytecode namespaces differ",
        ));
    }

    let bytecode_artifact = crate::vm::artifact::encode_program(&program)
        .map_err(|error| HandoffFailure::new("bytecode/artifact-encode", error))?;
    let decoded = crate::vm::artifact::decode_program(&bytecode_artifact)
        .map_err(|error| HandoffFailure::new("bytecode/artifact-decode", error))?;
    if decoded.namespace.as_deref() != Some(module.namespace.as_str()) {
        return Err(HandoffFailure::new(
            "bytecode/provenance",
            "decoded bytecode namespace differs from HALC",
        ));
    }

    let instruction_count = program
        .functions
        .iter()
        .map(|function| function.code.len())
        .sum();
    let handler_count = program
        .functions
        .iter()
        .map(|function| function.handlers.len())
        .sum();
    let source_positions = program
        .functions
        .iter()
        .map(|function| function.source_map.len())
        .sum();
    let artifact_hash = Sha256::digest(&bytecode_artifact);
    let artifact_digest = hex(artifact_hash.as_ref());

    let mut evidence = base_handoff_evidence(&module.namespace, &module.resource);
    evidence.insert("handoff/supported".into(), boolean(true));
    evidence.insert("handoff/status".into(), string("ready"));
    evidence.insert("handoff/compiler".into(), string("vm/compile-halc-module"));
    evidence.insert(
        "handoff/source-hash".into(),
        string(hex(&module.source_hash)),
    );
    evidence.insert(
        "handoff/program-namespace".into(),
        string(decoded.namespace.unwrap_or_default()),
    );
    evidence.insert(
        "handoff/program-entry".into(),
        integer(program.entry as usize),
    );
    evidence.insert(
        "handoff/program-functions".into(),
        integer(program.functions.len()),
    );
    evidence.insert(
        "handoff/program-constants".into(),
        integer(program.constants.len()),
    );
    evidence.insert(
        "handoff/program-instructions".into(),
        integer(instruction_count),
    );
    evidence.insert("handoff/program-handlers".into(), integer(handler_count));
    evidence.insert("handoff/source-positions".into(), integer(source_positions));
    evidence.insert(
        "handoff/schema-definitions".into(),
        integer(program.schema_types.len()),
    );
    evidence.insert(
        "handoff/schema-functions".into(),
        integer(program.function_types.len()),
    );
    evidence.insert("handoff/program-validated".into(), boolean(true));
    evidence.insert("handoff/artifact-decodable".into(), boolean(true));
    evidence.insert(
        "handoff/artifact-bytes".into(),
        integer(bytecode_artifact.len()),
    );
    evidence.insert("handoff/artifact-digest".into(), string(artifact_digest));
    Ok(evidence)
}

#[cfg(feature = "bytecode-vm")]
fn compiler_registry() -> super::namespace::NamespaceRegistry<crate::core::Value> {
    crate::core::minimal_namespace_registry()
}

#[cfg(feature = "bytecode-vm")]
fn compile_failure_category(kind: crate::vm::error::CompileErrorKind) -> &'static str {
    match kind {
        crate::vm::error::CompileErrorKind::Parse => "bytecode/parse",
        crate::vm::error::CompileErrorKind::UnsupportedForm => "bytecode/unsupported-form",
        crate::vm::error::CompileErrorKind::UnboundSymbol => "bytecode/unbound-symbol",
        crate::vm::error::CompileErrorKind::Arity => "bytecode/arity",
        crate::vm::error::CompileErrorKind::Recur => "bytecode/recur",
        crate::vm::error::CompileErrorKind::InvalidEffect => "bytecode/invalid-effect",
        crate::vm::error::CompileErrorKind::Limit => "bytecode/limit",
        crate::vm::error::CompileErrorKind::Internal => "bytecode/internal",
    }
}

#[cfg(feature = "bytecode-vm")]
fn hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write;
        let _ = write!(&mut output, "{byte:02x}");
    }
    output
}

#[cfg(test)]
mod tests;
