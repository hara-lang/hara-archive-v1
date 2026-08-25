use std::collections::{BTreeMap, BTreeSet};

pub const INSTRUMENTATION_PROTOCOL: &str = "hara.instrumentation/0-alpha";
pub const INSTRUMENTATION_EVENT_SCHEMA: &str = "hara.instrumentation.event/0-alpha";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum InstrumentMode {
    Passive,
    Control,
    Transform,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TargetKind {
    Interpreter,
    Hbc,
    WholeWasm,
}

impl TargetKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Interpreter => "interpreter",
            Self::Hbc => "hbc",
            Self::WholeWasm => "whole-wasm",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RuntimeBackend(String);

impl RuntimeBackend {
    pub fn new(value: impl Into<String>) -> Result<Self, &'static str> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err("runtime backend must be non-empty");
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Capability {
    EventSemanticBoundary,
    EventInstruction,
    EventCall,
    EventException,
    EventEffect,
    EventSuspension,
    EventLifecycle,
    InspectSourceLocation,
    InspectCurrentFrame,
    InspectFrames,
    InspectLocals,
    InspectStack,
    InspectValuePreview,
    InspectSnapshot,
    ControlPause,
    ControlSingleStep,
    ControlResume,
    ControlSettle,
    ControlTerminate,
    TransformHalc,
    TransformHbc,
    RetransformHalc,
    RetransformHbc,
}

impl Capability {
    pub const fn is_control(self) -> bool {
        matches!(
            self,
            Self::ControlPause
                | Self::ControlSingleStep
                | Self::ControlResume
                | Self::ControlSettle
                | Self::ControlTerminate
        )
    }

    pub const fn is_transform(self) -> bool {
        matches!(
            self,
            Self::TransformHalc | Self::TransformHbc | Self::RetransformHalc | Self::RetransformHbc
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum EventKind {
    SemanticBoundary = 0,
    InstructionExecute = 1,
    CallEnter = 2,
    CallReturn = 3,
    ExceptionRaise = 4,
    ExceptionUnwind = 5,
    VarSet = 6,
    FieldSet = 7,
    PromiseSuspend = 8,
    PromiseResume = 9,
    MachineSuspend = 10,
    MachineResume = 11,
    ExecutionTerminal = 12,
    ProtocolCall = 13,
}

impl EventKind {
    pub const fn required_capability(self) -> Capability {
        match self {
            Self::SemanticBoundary => Capability::EventSemanticBoundary,
            Self::InstructionExecute => Capability::EventInstruction,
            Self::CallEnter | Self::CallReturn => Capability::EventCall,
            Self::ExceptionRaise | Self::ExceptionUnwind => Capability::EventException,
            Self::VarSet | Self::FieldSet => Capability::EventEffect,
            Self::PromiseSuspend
            | Self::PromiseResume
            | Self::MachineSuspend
            | Self::MachineResume => Capability::EventSuspension,
            Self::ExecutionTerminal => Capability::EventLifecycle,
            Self::ProtocolCall => Capability::EventSemanticBoundary,
        }
    }

    pub const fn supports_target(self, target: TargetKind) -> bool {
        match target {
            TargetKind::Interpreter => matches!(
                self,
                Self::SemanticBoundary
                    | Self::CallEnter
                    | Self::CallReturn
                    | Self::ExceptionRaise
                    | Self::VarSet
                    | Self::FieldSet
                    | Self::PromiseSuspend
                    | Self::PromiseResume
                    | Self::ExecutionTerminal
            ),
            TargetKind::Hbc => matches!(
                self,
                Self::InstructionExecute
                    | Self::CallEnter
                    | Self::CallReturn
                    | Self::ExceptionUnwind
                    | Self::MachineSuspend
                    | Self::MachineResume
                    | Self::ExecutionTerminal
            ),
            TargetKind::WholeWasm => matches!(self, Self::ProtocolCall | Self::ExecutionTerminal),
        }
    }

    const fn bit(self) -> u64 {
        1_u64 << (self as u8)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EventMask(u64);

impl EventMask {
    pub const fn empty() -> Self {
        Self(0)
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub const fn contains(self, event: EventKind) -> bool {
        self.0 & event.bit() != 0
    }

    pub fn insert(&mut self, event: EventKind) {
        self.0 |= event.bit();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProjectionLimits {
    pub max_items: usize,
    pub max_depth: usize,
    pub max_bytes: usize,
}

impl Default for ProjectionLimits {
    fn default() -> Self {
        Self {
            max_items: 256,
            max_depth: 16,
            max_bytes: 64 * 1024,
        }
    }
}

impl ProjectionLimits {
    pub const fn is_bounded(self) -> bool {
        self.max_items > 0 && self.max_depth > 0 && self.max_bytes > 0
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProjectionRequest {
    pub source_location: bool,
    pub current_frame: Option<ProjectionLimits>,
    pub frames: Option<ProjectionLimits>,
    pub locals: Option<ProjectionLimits>,
    pub stack: Option<ProjectionLimits>,
    pub value_preview: Option<ProjectionLimits>,
    pub machine_snapshot: Option<ProjectionLimits>,
}

impl ProjectionRequest {
    pub fn is_bounded(&self) -> bool {
        [
            self.current_frame,
            self.frames,
            self.locals,
            self.stack,
            self.value_preview,
            self.machine_snapshot,
        ]
        .into_iter()
        .flatten()
        .all(ProjectionLimits::is_bounded)
    }

    pub fn required_capabilities(&self) -> BTreeSet<Capability> {
        let mut required = BTreeSet::new();
        if self.source_location {
            required.insert(Capability::InspectSourceLocation);
        }
        for (projection, capability) in [
            (self.current_frame, Capability::InspectCurrentFrame),
            (self.frames, Capability::InspectFrames),
            (self.locals, Capability::InspectLocals),
            (self.stack, Capability::InspectStack),
            (self.value_preview, Capability::InspectValuePreview),
            (self.machine_snapshot, Capability::InspectSnapshot),
        ] {
            if projection.is_some() {
                required.insert(capability);
            }
        }
        required
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventDelivery {
    Callback,
    Queue { capacity: usize },
}

impl Default for EventDelivery {
    fn default() -> Self {
        Self::Queue { capacity: 256 }
    }
}

impl EventDelivery {
    pub const fn is_bounded(&self) -> bool {
        match self {
            Self::Callback => true,
            Self::Queue { capacity } => *capacity > 0,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InstrumentFilter {
    pub session_id: Option<String>,
    pub target_ids: BTreeSet<String>,
    pub target_kinds: BTreeSet<TargetKind>,
    pub backends: BTreeSet<RuntimeBackend>,
}

impl InstrumentFilter {
    pub fn matches(&self, target: &TargetDescriptor) -> bool {
        self.session_id
            .as_ref()
            .map_or(true, |session| session == &target.session_id)
            && (self.target_ids.is_empty() || self.target_ids.contains(&target.target_id))
            && (self.target_kinds.is_empty() || self.target_kinds.contains(&target.kind))
            && (self.backends.is_empty() || self.backends.contains(&target.backend))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstrumentRegistration {
    pub instrument_id: String,
    pub session_id: String,
    pub mode: InstrumentMode,
    pub capabilities: BTreeSet<Capability>,
    pub events: BTreeSet<EventKind>,
    pub filter: InstrumentFilter,
    pub projection: ProjectionRequest,
    pub delivery: EventDelivery,
}

impl InstrumentRegistration {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.instrument_id.trim().is_empty() {
            return Err("instrument id must be non-empty");
        }
        if self.session_id.trim().is_empty() {
            return Err("instrument session id must be non-empty");
        }
        if !self.projection.is_bounded() {
            return Err("instrument projections must be bounded");
        }
        if !self.delivery.is_bounded() {
            return Err("queued event delivery must have positive capacity");
        }
        if self
            .events
            .iter()
            .any(|event| !self.capabilities.contains(&event.required_capability()))
        {
            return Err("event subscriptions require their event capability");
        }
        if !self
            .projection
            .required_capabilities()
            .is_subset(&self.capabilities)
        {
            return Err("instrument projections require their inspection capability");
        }
        if self
            .filter
            .session_id
            .as_ref()
            .is_some_and(|session_id| session_id.trim().is_empty())
            || self
                .filter
                .target_ids
                .iter()
                .any(|target_id| target_id.trim().is_empty())
        {
            return Err("instrument filters cannot contain empty ids");
        }
        if self.mode != InstrumentMode::Control
            && self
                .capabilities
                .iter()
                .copied()
                .any(Capability::is_control)
        {
            return Err("only control instruments can request control capabilities");
        }
        if self.mode != InstrumentMode::Transform
            && self
                .capabilities
                .iter()
                .copied()
                .any(Capability::is_transform)
        {
            return Err("only transform instruments can request transform capabilities");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetDescriptor {
    pub target_id: String,
    pub session_id: String,
    pub kind: TargetKind,
    pub backend: RuntimeBackend,
    pub capabilities: BTreeSet<Capability>,
}

impl TargetDescriptor {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.target_id.trim().is_empty() {
            return Err("target id must be non-empty");
        }
        if self.session_id.trim().is_empty() {
            return Err("target session id must be non-empty");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InstrumentHandle {
    instrument_id: String,
    generation: u64,
}

impl InstrumentHandle {
    pub(crate) fn new(instrument_id: String, generation: u64) -> Self {
        Self {
            instrument_id,
            generation,
        }
    }

    pub fn instrument_id(&self) -> &str {
        &self.instrument_id
    }

    pub const fn generation(&self) -> u64 {
        self.generation
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TargetHandle {
    target_id: String,
    generation: u64,
}

impl TargetHandle {
    pub(crate) fn new(target_id: String, generation: u64) -> Self {
        Self {
            target_id,
            generation,
        }
    }

    pub fn target_id(&self) -> &str {
        &self.target_id
    }

    pub const fn generation(&self) -> u64 {
        self.generation
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventPhase {
    Live,
    Replay,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstrumentDirective {
    Continue,
    Suspend,
    StepNext,
    Terminate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceSpan {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EventLocation {
    pub source_id: Option<String>,
    pub form_path: Option<Vec<usize>>,
    pub span: Option<SourceSpan>,
    pub function: Option<String>,
    pub instruction_pointer: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventEnvelope<D = BTreeMap<String, String>> {
    pub schema: String,
    pub protocol: String,
    pub instrument_id: String,
    pub runtime: RuntimeBackend,
    pub session_id: String,
    pub target_id: String,
    pub target_kind: TargetKind,
    pub generation: u64,
    pub sequence: u64,
    pub phase: EventPhase,
    pub event: EventKind,
    pub location: Option<EventLocation>,
    pub data: D,
}

impl<D> EventEnvelope<D> {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.schema != INSTRUMENTATION_EVENT_SCHEMA {
            return Err("unsupported instrumentation event schema");
        }
        if self.protocol != INSTRUMENTATION_PROTOCOL {
            return Err("unsupported instrumentation protocol");
        }
        if self.instrument_id.trim().is_empty()
            || self.session_id.trim().is_empty()
            || self.target_id.trim().is_empty()
        {
            return Err("instrument, session, and target ids must be non-empty");
        }
        if !self.event.supports_target(self.target_kind) {
            return Err("event kind is not supported by target kind");
        }
        if let Some(location) = &self.location {
            if self.target_kind == TargetKind::Interpreter && location.instruction_pointer.is_some()
            {
                return Err("interpreter events cannot claim an instruction pointer");
            }
            if self.target_kind == TargetKind::Hbc
                && self.event == EventKind::InstructionExecute
                && location.form_path.is_some()
            {
                return Err("HBC instruction events cannot claim an AST form path");
            }
            if location
                .span
                .as_ref()
                .is_some_and(|span| span.end < span.start)
            {
                return Err("event source span end precedes its start");
            }
        }
        Ok(())
    }
}
