//! Native event, inspection, and execution-control contracts.
//!
//! The hub is owned by a Hara [`crate::Runtime`]. It registers trusted
//! instruments and real execution targets without exposing either target
//! implementation or an ambient Hara-level authority. Producer probes attach
//! to the authoritative evaluator and HBC machine in this module family.

pub mod conformance;
#[cfg(all(feature = "bytecode-vm", feature = "bytecode-instrumentation"))]
mod hbc;
mod hub;
mod interpreter;
mod model;
mod native;
mod native_identity;

#[cfg(all(feature = "bytecode-vm", feature = "bytecode-instrumentation"))]
pub use hbc::{hbc_capabilities, HbcBoundary, HbcTarget};
pub use hub::{
    ControlLease, DeliveredEvent, DispatchReport, EventAccess, EventBatch, EventProjection,
    InstrumentationAttachment, InstrumentationError, InstrumentationHub, PortableProjection,
    ProducerEvent, SessionCleanup,
};
pub use interpreter::{interpreter_capabilities, InterpreterBoundary, InterpreterTarget};
pub use model::{
    Capability, EventDelivery, EventEnvelope, EventKind, EventLocation, EventMask, EventPhase,
    InstrumentDirective, InstrumentFilter, InstrumentHandle, InstrumentMode,
    InstrumentRegistration, ProjectionLimits, ProjectionRequest, RuntimeBackend, SourceSpan,
    TargetDescriptor, TargetHandle, TargetKind, INSTRUMENTATION_EVENT_SCHEMA,
    INSTRUMENTATION_PROTOCOL,
};
pub use native::{
    NativeAttachment, NativeControlLease, NativeInstrumentHandle, NativeInstrumentation,
    NativeInstrumentationError, NativeTargetHandle,
};

#[cfg(test)]
mod conformance_tests;
#[cfg(test)]
mod delivery_tests;
#[cfg(test)]
mod tests;
