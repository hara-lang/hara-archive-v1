use crate::core::{Promise, PromiseState, Value};
use crate::kernel::{NamespaceRegistry, VarOrigin};
use crate::lang::data::{OrderedMap, Vector};
use crate::lang::protocol::INamespaced;

use super::{
    evidence, fresh_registry, trace_id, BytecodeObservationSession, BytecodeSessionError,
    BytecodeSessionStatus, Machine, SessionMetrics, VmError,
};
use crate::vm::machine::observation::MachineSnapshot;

const GLOBAL_SNAPSHOT_LIMIT: usize = 64;

impl BytecodeObservationSession {
    pub fn snapshot(&self) -> Result<MachineSnapshot, BytecodeSessionError> {
        let machine = self
            .machine
            .as_ref()
            .ok_or_else(|| BytecodeSessionError::new("bytecode session is disposed"))?;
        Ok(machine.snapshot_with_limits(self.observation_limits))
    }

    pub fn snapshot_value(&self) -> Result<Value, BytecodeSessionError> {
        let snapshot = self.snapshot()?;
        let value = evidence::snapshot_value(&snapshot, &self.source_id);
        Ok(with_global_state(
            value,
            global_state_value(&self.registry, self.observation_limits.display_chars),
        ))
    }

    pub fn step(&mut self) -> Result<Value, BytecodeSessionError> {
        self.ensure_runnable()?;
        let record = self.execute_step()?;
        Ok(self.delta_trace(std::slice::from_ref(&record)))
    }

    pub fn run(&mut self, step_limit: usize) -> Result<Value, BytecodeSessionError> {
        self.ensure_runnable()?;
        let mut records = Vec::new();
        for _ in 0..step_limit {
            let record = self.execute_step()?;
            records.push(record);
            if !matches!(
                self.status,
                BytecodeSessionStatus::Ready | BytecodeSessionStatus::Running
            ) {
                break;
            }
        }
        Ok(self.delta_trace(&records))
    }

    pub fn pause(&mut self) -> bool {
        if !matches!(
            self.status,
            BytecodeSessionStatus::Ready | BytecodeSessionStatus::Running
        ) {
            return false;
        }
        self.paused_from = Some(self.status);
        self.status = BytecodeSessionStatus::Paused;
        true
    }

    pub fn resume(
        &mut self,
        settlement: Option<PromiseState>,
    ) -> Result<Value, BytecodeSessionError> {
        if self.status == BytecodeSessionStatus::Paused {
            if settlement.is_some() {
                return Err(BytecodeSessionError::new(
                    "paused bytecode session does not accept a promise settlement",
                ));
            }
            self.status = self
                .paused_from
                .take()
                .unwrap_or(BytecodeSessionStatus::Running);
            return Ok(self.delta_trace(&[]));
        }
        if self.status != BytecodeSessionStatus::Suspended {
            return Err(BytecodeSessionError::new(format!(
                "bytecode session cannot resume from {}",
                self.status.as_keyword()
            )));
        }
        let settlement = match settlement {
            Some(settlement) => settlement,
            None => self
                .suspension
                .as_ref()
                .ok_or_else(|| BytecodeSessionError::new("suspended promise is unavailable"))?
                .state(),
        };
        let step = self.execute_resume(settlement)?;
        Ok(self.delta_trace(std::slice::from_ref(&step)))
    }

    pub fn resolve_suspension(&self, value: Value) -> Result<bool, BytecodeSessionError> {
        self.suspension
            .as_ref()
            .map(|promise| promise.resolve(value))
            .ok_or_else(|| BytecodeSessionError::new("bytecode session is not suspended"))
    }

    pub fn reject_suspension(&self, error: Value) -> Result<bool, BytecodeSessionError> {
        self.suspension
            .as_ref()
            .map(|promise| promise.reject_value(error))
            .ok_or_else(|| BytecodeSessionError::new("bytecode session is not suspended"))
    }

    pub fn suspended_promise(&self) -> Option<Promise> {
        self.suspension.clone()
    }

    pub fn result(&self) -> Option<&Value> {
        self.result.as_ref()
    }

    pub fn take_result(&mut self) -> Option<Value> {
        self.result.take()
    }

    pub fn error(&self) -> Option<&VmError> {
        self.error.as_ref()
    }

    pub fn reset(&mut self) -> Result<Value, BytecodeSessionError> {
        let program =
            self.program.as_ref().cloned().ok_or_else(|| {
                BytecodeSessionError::new("disposed bytecode session cannot reset")
            })?;
        cancel_pending(self.suspension.take());
        self.machine = Some(Machine::entry(program));
        self.registry = fresh_registry();
        self.status = BytecodeSessionStatus::Ready;
        self.paused_from = None;
        self.trace_generation = self.trace_generation.saturating_add(1);
        self.trace_id = trace_id(&self.session_id, self.trace_generation);
        self.metrics = SessionMetrics::default();
        self.events.clear();
        self.trace_steps.clear();
        self.dropped_events = 0;
        self.omitted_trace_steps = 0;
        self.result = None;
        self.error = None;
        self.snapshot_value()
    }

    pub fn metrics(&self) -> Value {
        evidence::metrics_document(
            &self.session_id,
            &self.trace_id,
            self.next_sequence,
            self.status.as_keyword(),
            &self.metrics,
        )
    }

    pub fn events(&self) -> Value {
        evidence::events_document(
            &self.session_id,
            &self.trace_id,
            self.next_sequence,
            self.status.as_keyword(),
            self.events.iter(),
            self.dropped_events,
        )
    }

    pub fn trace(&self) -> Value {
        evidence::trace_document(
            &self.session_id,
            &self.trace_id,
            &self.source_id,
            self.next_sequence,
            self.status.as_keyword(),
            self.trace_steps.iter(),
            self.omitted_trace_steps,
        )
    }

    pub fn metrics_json(&self) -> Result<String, BytecodeSessionError> {
        write_json(&self.metrics())
    }

    pub fn events_json(&self) -> Result<String, BytecodeSessionError> {
        write_json(&self.events())
    }

    pub fn trace_json(&self) -> Result<String, BytecodeSessionError> {
        write_json(&self.trace())
    }

    pub fn dispose(&mut self) -> bool {
        if self.status == BytecodeSessionStatus::Disposed {
            return false;
        }
        self.dispose_inner();
        true
    }
}

#[path = "control/runtime.rs"]
mod runtime;

fn cancel_pending(promise: Option<Promise>) {
    if let Some(promise) = promise {
        if matches!(promise.state(), PromiseState::Pending) {
            promise.cancel();
        }
    }
}

fn write_json(value: &Value) -> Result<String, BytecodeSessionError> {
    crate::json::write(value).map_err(BytecodeSessionError::new)
}

fn with_global_state(snapshot: Value, globals: Value) -> Value {
    match snapshot {
        Value::OrderedMap(fields) => Value::OrderedMap(Box::new(
            (*fields).assoc_value_owned(Value::String("globals".into()), globals),
        )),
        value => value,
    }
}

fn global_state_value(registry: &NamespaceRegistry<Value>, display_chars: usize) -> Value {
    let current = registry.current();
    let namespace = current.name().as_str().to_owned();
    let mut bindings = current
        .mappings()
        .into_iter()
        .filter_map(|(_, var)| {
            (var.symbol().get_namespace() == Some(namespace.as_str())).then(|| {
                let metadata = var.metadata();
                let symbol = var.symbol().as_str().to_owned();
                let value = object([
                    ("symbol", string(&symbol)),
                    ("dynamic", Value::Bool(var.is_dynamic())),
                    ("macro", Value::Bool(var.is_macro())),
                    ("origin", string(origin_keyword(metadata.origin))),
                    (
                        "value",
                        global_value_snapshot(&var.deref_value(), display_chars),
                    ),
                ]);
                (symbol, value)
            })
        })
        .collect::<Vec<_>>();
    bindings.sort_by(|left, right| left.0.cmp(&right.0));
    let omitted = bindings.len().saturating_sub(GLOBAL_SNAPSHOT_LIMIT);
    let retained = bindings
        .into_iter()
        .take(GLOBAL_SNAPSHOT_LIMIT)
        .map(|(_, value)| value);

    object([
        ("namespace", string(namespace)),
        ("scope", string("current-namespace-owned")),
        ("bindings", vector(retained)),
        ("omitted", integer(omitted)),
        ("limit", integer(GLOBAL_SNAPSHOT_LIMIT)),
    ])
}

fn global_value_snapshot(value: &Value, display_chars: usize) -> Value {
    let kind = match value {
        Value::Number(_) => "long",
        Value::BigInteger(_) if crate::numeric::is_long_value(value) => "long",
        Value::BigInteger(_) => "bigint",
        Value::Float(_) => "float",
        Value::Character(_) => "character",
        Value::Bool(_) => "boolean",
        Value::String(_) => "string",
        Value::Keyword(_) => "keyword",
        Value::Symbol(_) => "symbol",
        Value::Promise(_) => "promise",
        Value::Function(_) => "function",
        Value::Var(_) => "var",
        Value::Nil => "nil",
        _ => "value",
    };
    let (display, truncated) = bounded_display(value.display(), display_chars);
    object([
        ("kind", string(kind)),
        ("display", string(display)),
        ("truncated", Value::Bool(truncated)),
    ])
}

fn bounded_display(display: String, limit: usize) -> (String, bool) {
    let mut chars = display.chars();
    let mut bounded = chars.by_ref().take(limit).collect::<String>();
    let truncated = chars.next().is_some();
    if truncated {
        bounded.push('…');
    }
    (bounded, truncated)
}

fn origin_keyword(origin: VarOrigin) -> &'static str {
    match origin {
        VarOrigin::Source => "source",
        VarOrigin::HalFallback => "hal-fallback",
        VarOrigin::RustLibrary => "rust-library",
        VarOrigin::RuntimePrimitive => "runtime-primitive",
    }
}

fn object<const N: usize>(fields: [(&str, Value); N]) -> Value {
    Value::OrderedMap(Box::new(OrderedMap::from_iter(
        fields
            .into_iter()
            .map(|(key, value)| (Value::String(key.into()), value)),
    )))
}

fn vector(values: impl IntoIterator<Item = Value>) -> Value {
    Value::Vector(Vector::from_iter(values))
}

fn string(value: impl Into<String>) -> Value {
    Value::String(value.into())
}

fn integer(value: usize) -> Value {
    Value::Number(i64::try_from(value).unwrap_or(i64::MAX))
}
