use serde_json::{json, Value as JsonValue};
use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::rc::Rc;

use crate::core::{self, EvalFiberState, PromiseState, Value};
use crate::instrumentation::{
    interpreter_capabilities, Capability, ControlLease, DeliveredEvent, EventDelivery, EventKind,
    EventPhase, InstrumentDirective, InstrumentFilter, InstrumentHandle, InstrumentMode,
    InstrumentRegistration, InstrumentationError, InstrumentationHub, InterpreterTarget,
    ProjectionLimits, ProjectionRequest, RuntimeBackend, TargetDescriptor, TargetHandle,
    TargetKind,
};
use crate::task::PromiseRejection;
use crate::Runtime;

use super::{
    required_text, LiveBackend, LiveReplacementPolicy, LiveSession, LiveSessionCapabilities,
    LiveSessionCommand, LiveSessionError, LiveSessionOperation, LiveSessionState,
    LiveSessionStatus, LiveSettlement, LiveSource,
};

const EVENT_QUEUE_CAPACITY: usize = 1_024;

#[derive(Clone)]
struct InterpreterRuntimeContext {
    namespace_registry: crate::kernel::NamespaceRegistry<Value>,
    protocols: core::ProtocolRegistry,
    macros: Rc<RefCell<HashMap<(String, String), Rc<core::Function>>>>,
}

impl InterpreterRuntimeContext {
    fn from_runtime(runtime: &Runtime) -> Self {
        Self {
            namespace_registry: runtime.namespace_registry.clone(),
            protocols: runtime.protocols.clone(),
            macros: runtime.macros.clone(),
        }
    }

    fn run<T>(&self, operation: impl FnOnce() -> T) -> T {
        let namespaces = self.namespace_registry.clone();
        let protocols = self.protocols.clone();
        let macros = self.macros.clone();
        core::with_macros(macros, move || {
            core::with_namespace_registry(&namespaces, move || {
                core::with_protocols(&protocols, operation)
            })
        })
    }
}

/// Session-owned LiveSession controller over the authoritative EvalFiber
/// instrumentation target. This adapter owns no alternate evaluator, replay
/// journal, or process-global observation registry.
pub(crate) struct InstrumentedInterpreterLiveSession {
    owner_session_id: String,
    session_id: String,
    source: LiveSource,
    pending_source: Option<LiveSource>,
    generation: u64,
    sequence: u64,
    status: LiveSessionStatus,
    context: InterpreterRuntimeContext,
    initial_environment: HashMap<String, Value>,
    hub: Rc<RefCell<InstrumentationHub>>,
    instrument: Option<InstrumentHandle>,
    target_handle: Option<TargetHandle>,
    lease: Option<ControlLease>,
    target: Option<InterpreterTarget>,
}

impl InstrumentedInterpreterLiveSession {
    pub(crate) fn start(
        runtime: &Runtime,
        owner_session_id: impl Into<String>,
        session_id: impl Into<String>,
        source: LiveSource,
    ) -> Result<Self, LiveSessionError> {
        let owner_session_id = required_text(owner_session_id.into(), "owner session id")?;
        let session_id = required_text(session_id.into(), "session id")?;
        let context = InterpreterRuntimeContext::from_runtime(runtime);
        let initial_environment = runtime.execution.snapshot();
        let hub = runtime.execution.instrumentation_handle();
        let target_id = target_id(&owner_session_id, &session_id);
        let instrument_id = instrument_id(&owner_session_id, &session_id);
        let registration = controller_registration(&owner_session_id, &instrument_id, &target_id);
        let instrument = hub
            .borrow_mut()
            .register(registration)
            .map_err(instrumentation_error)?;
        let mut session = Self {
            owner_session_id,
            session_id,
            source,
            pending_source: None,
            generation: 0,
            sequence: 0,
            status: LiveSessionStatus::Ready,
            context,
            initial_environment,
            hub,
            instrument: Some(instrument),
            target_handle: None,
            lease: None,
            target: None,
        };
        if let Err(error) = session.install_target() {
            session.detach_instrument();
            return Err(error);
        }
        Ok(session)
    }

    fn instrument(&self) -> Result<&InstrumentHandle, LiveSessionError> {
        self.instrument.as_ref().ok_or_else(|| {
            LiveSessionError::new(
                "live-session/disposed",
                "interpreter controller instrument has been detached",
            )
        })
    }

    fn lease(&self) -> Result<&ControlLease, LiveSessionError> {
        self.lease.as_ref().ok_or_else(|| {
            LiveSessionError::new(
                "live-session/disposed",
                "interpreter controller lease has been released",
            )
        })
    }

    fn target(&self) -> Result<&InterpreterTarget, LiveSessionError> {
        self.target.as_ref().ok_or_else(|| {
            LiveSessionError::new(
                "live-session/disposed",
                "interpreter instrumentation target has been disposed",
            )
        })
    }

    fn target_mut(&mut self) -> Result<&mut InterpreterTarget, LiveSessionError> {
        self.target.as_mut().ok_or_else(|| {
            LiveSessionError::new(
                "live-session/disposed",
                "interpreter instrumentation target has been disposed",
            )
        })
    }

    fn install_target(&mut self) -> Result<(), LiveSessionError> {
        let target_id = target_id(&self.owner_session_id, &self.session_id);
        let descriptor = TargetDescriptor {
            target_id,
            session_id: self.owner_session_id.clone(),
            kind: TargetKind::Interpreter,
            backend: RuntimeBackend::new("rust").expect("Rust is a valid backend id"),
            capabilities: interpreter_capabilities(),
        };
        let handle = self
            .hub
            .borrow_mut()
            .register_target(descriptor)
            .map_err(instrumentation_error)?;
        let target = self.context.run(|| {
            let hub = self.hub.borrow();
            InterpreterTarget::start(
                &hub,
                handle.clone(),
                self.source.source_id(),
                self.source.source(),
                self.initial_environment.clone(),
            )
        });
        let target = match target {
            Ok(target) => target,
            Err(error) => {
                let _ = self.hub.borrow_mut().remove_target(&handle);
                return Err(instrumentation_error(error));
            }
        };
        let instrument = self.instrument()?.clone();
        let lease = {
            let mut hub = self.hub.borrow_mut();
            if let Err(error) = hub.attach(&instrument, &handle) {
                let _ = hub.remove_target(&handle);
                return Err(instrumentation_error(error));
            }
            match hub.acquire_control(&instrument, &handle) {
                Ok(lease) => lease,
                Err(error) => {
                    let _ = hub.remove_target(&handle);
                    return Err(instrumentation_error(error));
                }
            }
        };
        self.target_handle = Some(handle);
        self.lease = Some(lease);
        self.target = Some(target);
        self.status = LiveSessionStatus::Ready;
        Ok(())
    }

    fn remove_target(&mut self) {
        self.target.take();
        if let Some(lease) = self.lease.take() {
            let _ = self.hub.borrow_mut().release_control(&lease);
        }
        if let Some(target) = self.target_handle.take() {
            let _ = self.hub.borrow_mut().remove_target(&target);
        }
    }

    fn detach_instrument(&mut self) {
        self.remove_target();
        if let Some(instrument) = self.instrument.take() {
            let _ = self.hub.borrow_mut().detach(&instrument);
        }
    }

    fn restart(&mut self, source: LiveSource) -> Result<JsonValue, LiveSessionError> {
        self.remove_target();
        self.source = source;
        self.pending_source = None;
        self.generation = self.generation.saturating_add(1);
        self.sequence = 0;
        if let Err(error) = self.install_target() {
            self.status = LiveSessionStatus::Failed;
            return Err(error);
        }
        self.snapshot_payload("restart")
    }

    fn reset(&mut self) -> Result<JsonValue, LiveSessionError> {
        let source = self
            .pending_source
            .take()
            .unwrap_or_else(|| self.source.clone());
        self.restart(source)
    }

    fn request_directive(
        &mut self,
        directive: InstrumentDirective,
    ) -> Result<(), LiveSessionError> {
        let lease = self.lease()?.clone();
        self.hub
            .borrow_mut()
            .request_directive(&lease, directive)
            .map_err(instrumentation_error)
    }

    fn step_target(&mut self) -> Result<(), LiveSessionError> {
        let context = self.context.clone();
        let hub = self.hub.clone();
        let target = self.target_mut()?;
        context
            .run(|| target.step(&mut hub.borrow_mut()))
            .map_err(instrumentation_error)?;
        self.sequence = self.sequence.saturating_add(1);
        self.sync_status();
        Ok(())
    }

    fn run_target(&mut self, boundary_limit: usize) -> Result<usize, LiveSessionError> {
        if self.target()?.paused() {
            let lease = self.lease()?.clone();
            let hub = self.hub.clone();
            self.target_mut()?
                .continue_execution(&hub.borrow(), &lease)
                .map_err(instrumentation_error)?;
        }
        let context = self.context.clone();
        let hub = self.hub.clone();
        let target = self.target_mut()?;
        let boundaries = context
            .run(|| target.run(&mut hub.borrow_mut(), boundary_limit))
            .map_err(instrumentation_error)?;
        self.sequence = self.sequence.saturating_add(boundaries.len() as u64);
        self.sync_status();
        Ok(boundaries.len())
    }

    fn settle_target(&mut self, state: PromiseState) -> Result<(), LiveSessionError> {
        let lease = self.lease()?.clone();
        let context = self.context.clone();
        let hub = self.hub.clone();
        let target = self.target_mut()?;
        context
            .run(|| target.settle(&mut hub.borrow_mut(), &lease, state))
            .map_err(instrumentation_error)?;
        self.sequence = self.sequence.saturating_add(1);
        self.sync_status();
        Ok(())
    }

    fn resolve(&mut self, value: JsonValue) -> Result<JsonValue, LiveSessionError> {
        self.authorize_settlement()?;
        let pending = self
            .target()?
            .pending()
            .ok_or_else(|| LiveSessionError::backend("interpreter target is not suspended"))?;
        Ok(JsonValue::Bool(pending.resolve(json_to_value(value)?)))
    }

    fn reject(&mut self, error: JsonValue) -> Result<JsonValue, LiveSessionError> {
        self.authorize_settlement()?;
        let pending = self
            .target()?
            .pending()
            .ok_or_else(|| LiveSessionError::backend("interpreter target is not suspended"))?;
        Ok(JsonValue::Bool(pending.reject_value(json_to_value(error)?)))
    }

    fn authorize_settlement(&self) -> Result<(), LiveSessionError> {
        self.hub
            .borrow()
            .authorize_control(self.lease()?, Capability::ControlSettle)
            .map_err(instrumentation_error)
    }

    fn resume(
        &mut self,
        settlement: Option<LiveSettlement>,
    ) -> Result<JsonValue, LiveSessionError> {
        if matches!(self.target()?.state(), EvalFiberState::Suspended) {
            if let Some(settlement) = settlement {
                let pending = self.target()?.pending().ok_or_else(|| {
                    LiveSessionError::backend("interpreter target has no retained promise")
                })?;
                if !matches!(pending.state(), PromiseState::Pending) {
                    return Err(LiveSessionError::backend(
                        "interpreter promise has already been settled",
                    ));
                }
                match settlement {
                    LiveSettlement::Fulfilled(value) => {
                        pending.resolve(json_to_value(value)?);
                    }
                    LiveSettlement::Rejected(error) => {
                        pending.reject_rejection(PromiseRejection::Value(json_to_value(error)?));
                    }
                }
            }
            let state = self
                .target()?
                .pending()
                .ok_or_else(|| {
                    LiveSessionError::backend("interpreter target has no retained promise")
                })?
                .state();
            if matches!(state, PromiseState::Pending) {
                return Err(LiveSessionError::backend(
                    "interpreter promise remains pending",
                ));
            }
            self.settle_target(state)?;
        } else if self.target()?.paused() {
            let lease = self.lease()?.clone();
            let hub = self.hub.clone();
            self.target_mut()?
                .continue_execution(&hub.borrow(), &lease)
                .map_err(instrumentation_error)?;
            self.sync_status();
        }
        self.snapshot_payload("resume")
    }

    fn cancel(&mut self) -> Result<JsonValue, LiveSessionError> {
        self.request_directive(InstrumentDirective::Terminate)?;
        self.step_target()?;
        self.status = LiveSessionStatus::Cancelled;
        self.pending_source = None;
        Ok(json!({"cancelled": true}))
    }

    fn dispose(&mut self) -> JsonValue {
        if self.status == LiveSessionStatus::Disposed {
            return JsonValue::Bool(false);
        }
        self.detach_instrument();
        self.pending_source = None;
        self.status = LiveSessionStatus::Disposed;
        JsonValue::Bool(true)
    }

    fn sync_status(&mut self) {
        if matches!(
            self.status,
            LiveSessionStatus::Cancelled | LiveSessionStatus::Disposed
        ) {
            return;
        }
        let Ok(target) = self.target() else {
            self.status = LiveSessionStatus::Disposed;
            return;
        };
        self.status = match target.state() {
            EvalFiberState::Running if target.paused() => LiveSessionStatus::Paused,
            EvalFiberState::Running => LiveSessionStatus::Running,
            EvalFiberState::Suspended => LiveSessionStatus::Suspended,
            EvalFiberState::Completed(_) => LiveSessionStatus::Returned,
            EvalFiberState::Failed(_) => LiveSessionStatus::Failed,
            EvalFiberState::Cancelled => LiveSessionStatus::Cancelled,
        };
    }

    fn snapshot_payload(&mut self, operation: &str) -> Result<JsonValue, LiveSessionError> {
        self.sync_status();
        let instrument = self.instrument()?.clone();
        let batch = self
            .hub
            .borrow_mut()
            .drain_events(&instrument)
            .map_err(instrumentation_error)?;
        let events = batch.events.iter().map(event_json).collect::<Vec<_>>();
        Ok(json!({
            "operation": operation,
            "status": self.status.as_str(),
            "sessionId": self.session_id,
            "sourceId": self.source.source_id(),
            "generation": self.generation,
            "sequence": self.sequence,
            "target": self.target_handle.as_ref().map(|target| json!({
                "id": target.target_id(),
                "generation": target.generation(),
            })),
            "events": events,
            "dropped": batch.dropped,
        }))
    }
}

impl LiveSession for InstrumentedInterpreterLiveSession {
    fn state(&self) -> LiveSessionState {
        LiveSessionState {
            session_id: self.session_id.clone(),
            source_id: self.source.source_id().to_owned(),
            generation: self.generation,
            revision: self.source.revision().to_owned(),
            sequence: self.sequence,
            backend: LiveBackend::Interpreter,
            status: self.status,
        }
    }

    fn capabilities(&self) -> LiveSessionCapabilities {
        LiveSessionCapabilities {
            backend: LiveBackend::Interpreter,
            operations: vec![
                LiveSessionOperation::Snapshot,
                LiveSessionOperation::Step,
                LiveSessionOperation::Run,
                LiveSessionOperation::Pause,
                LiveSessionOperation::Resume,
                LiveSessionOperation::Resolve,
                LiveSessionOperation::Reject,
                LiveSessionOperation::Update,
                LiveSessionOperation::Reset,
                LiveSessionOperation::Cancel,
                LiveSessionOperation::Dispose,
            ],
            replacement_policies: vec![
                LiveReplacementPolicy::Restart,
                LiveReplacementPolicy::ReplaceOnNextStart,
            ],
        }
    }

    fn dispatch_command(
        &mut self,
        command: LiveSessionCommand,
    ) -> Result<JsonValue, LiveSessionError> {
        match command {
            LiveSessionCommand::Snapshot => self.snapshot_payload("snapshot"),
            LiveSessionCommand::Step => {
                self.request_directive(InstrumentDirective::StepNext)?;
                self.step_target()?;
                self.snapshot_payload("step")
            }
            LiveSessionCommand::Run { boundary_limit } => {
                let executed = self.run_target(boundary_limit)?;
                let mut payload = self.snapshot_payload("run")?;
                payload["steps"] = JsonValue::from(executed as u64);
                Ok(payload)
            }
            LiveSessionCommand::Call { .. } => Err(LiveSessionError::new(
                "live-session/unsupported-operation",
                "interpreter backend does not support direct function calls",
            )),
            LiveSessionCommand::Pause => {
                self.request_directive(InstrumentDirective::Suspend)?;
                self.step_target()?;
                self.snapshot_payload("pause")
            }
            LiveSessionCommand::Resume { settlement } => self.resume(settlement),
            LiveSessionCommand::Resolve { value } => self.resolve(value),
            LiveSessionCommand::Reject { error } => self.reject(error),
            LiveSessionCommand::Update { source, policy } => match policy {
                LiveReplacementPolicy::Restart => self.restart(source),
                LiveReplacementPolicy::ReplaceOnNextStart => {
                    let revision = source.revision().to_owned();
                    self.pending_source = Some(source);
                    Ok(json!({
                        "accepted": true,
                        "activation": "next-start",
                        "revision": revision,
                    }))
                }
                LiveReplacementPolicy::PreserveRuntime => Err(LiveSessionError::new(
                    "live-session/unsupported-replacement",
                    "interpreter backend does not support preserve-runtime replacement",
                )),
            },
            LiveSessionCommand::Reset => self.reset(),
            LiveSessionCommand::Cancel => self.cancel(),
            LiveSessionCommand::Dispose => Ok(self.dispose()),
        }
    }
}

impl Drop for InstrumentedInterpreterLiveSession {
    fn drop(&mut self) {
        self.detach_instrument();
    }
}

fn controller_registration(
    owner_session_id: &str,
    instrument_id: &str,
    target_id: &str,
) -> InstrumentRegistration {
    let projection = ProjectionRequest {
        source_location: true,
        value_preview: Some(ProjectionLimits::default()),
        machine_snapshot: Some(ProjectionLimits::default()),
        ..ProjectionRequest::default()
    };
    let events = BTreeSet::from([
        EventKind::SemanticBoundary,
        EventKind::CallEnter,
        EventKind::CallReturn,
        EventKind::ExceptionRaise,
        EventKind::VarSet,
        EventKind::FieldSet,
        EventKind::PromiseSuspend,
        EventKind::PromiseResume,
        EventKind::ExecutionTerminal,
    ]);
    InstrumentRegistration {
        instrument_id: instrument_id.into(),
        session_id: owner_session_id.into(),
        mode: InstrumentMode::Control,
        capabilities: interpreter_capabilities(),
        events,
        filter: InstrumentFilter {
            session_id: Some(owner_session_id.into()),
            target_ids: BTreeSet::from([target_id.into()]),
            target_kinds: BTreeSet::from([TargetKind::Interpreter]),
            backends: BTreeSet::from([
                RuntimeBackend::new("rust").expect("Rust is a valid backend id")
            ]),
        },
        projection,
        delivery: EventDelivery::Queue {
            capacity: EVENT_QUEUE_CAPACITY,
        },
    }
}

fn target_id(owner_session_id: &str, live_session_id: &str) -> String {
    format!("live-session/{owner_session_id}/{live_session_id}")
}

fn instrument_id(owner_session_id: &str, live_session_id: &str) -> String {
    format!("live-session-controller/{owner_session_id}/{live_session_id}")
}

fn json_to_value(value: JsonValue) -> Result<Value, LiveSessionError> {
    crate::json::read(&value.to_string()).map_err(|error| {
        LiveSessionError::backend(format!("unable to decode live-session value: {error}"))
    })
}

fn instrumentation_error(error: InstrumentationError) -> LiveSessionError {
    LiveSessionError::new("live-session/instrumentation", error.to_string())
}

fn event_json(event: &DeliveredEvent) -> JsonValue {
    let envelope = &event.envelope;
    json!({
        "schema": envelope.schema,
        "protocol": envelope.protocol,
        "instrument/id": envelope.instrument_id,
        "runtime": envelope.runtime.as_str(),
        "session-id": envelope.session_id,
        "target-id": envelope.target_id,
        "target-kind": envelope.target_kind.as_str(),
        "generation": envelope.generation,
        "sequence": envelope.sequence,
        "phase": phase_name(envelope.phase),
        "event": event_name(envelope.event),
        "location": envelope.location.as_ref().map(|location| json!({
            "source-id": location.source_id,
            "form/path": location.form_path,
            "function": location.function,
            "ip": location.instruction_pointer,
        })),
        "data": envelope.data,
        "projection": {
            "value-preview": event.projection.value_preview.as_ref().map(|value| &value.fields),
            "machine-snapshot": event.projection.machine_snapshot.as_ref().map(|value| &value.fields),
        },
        "dropped-before": event.dropped_before,
    })
}

fn phase_name(phase: EventPhase) -> &'static str {
    match phase {
        EventPhase::Live => "live",
        EventPhase::Replay => "replay",
    }
}

fn event_name(event: EventKind) -> &'static str {
    match event {
        EventKind::SemanticBoundary => "semantic/boundary",
        EventKind::InstructionExecute => "instruction/execute",
        EventKind::CallEnter => "call/enter",
        EventKind::CallReturn => "call/return",
        EventKind::ExceptionRaise => "exception/raise",
        EventKind::ExceptionUnwind => "exception/unwind",
        EventKind::VarSet => "effect/var-set",
        EventKind::FieldSet => "effect/field-set",
        EventKind::PromiseSuspend => "promise/suspend",
        EventKind::PromiseResume => "promise/resume",
        EventKind::MachineSuspend => "machine/suspend",
        EventKind::MachineResume => "machine/resume",
        EventKind::ProtocolCall => "semantic/protocol-call",
        EventKind::ExecutionTerminal => "execution/terminal",
    }
}
