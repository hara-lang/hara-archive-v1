use crate::core::{Promise, PromiseRejection, PromiseState, Value};
use crate::lang::protocol::IComponent;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub(crate) mod guest;
mod scope;
mod types;

pub use scope::{
    current_work_context, monotonic_nanos, process_work_host, WorkCancellationToken, WorkContext,
};
pub use types::{WorkHostStatus, WorkId, WorkOptions, WorkRunState, WorkRunStatus};

use scope::{
    cancellation_rejection, deadline_remaining_millis, install_progress_hooks, next_work_id,
    now_millis, resolve_deadline, work_failure,
};

pub(crate) use scope::with_current_work_context;

type WorkTask = Box<dyn FnOnce(WorkContext) -> Result<Value, PromiseRejection>>;
type WorkFinalizer = Box<dyn FnOnce(WorkContext) -> Result<(), PromiseRejection>>;

struct PendingWork {
    id: WorkId,
    task: WorkTask,
}

struct WorkHostInner {
    started: bool,
    next_id: u64,
    runs: HashMap<WorkId, WorkRun>,
    queue: VecDeque<PendingWork>,
}

/// Cloneable process-owned host for live work handles.
///
/// Rust Hara values are currently evaluator-thread values (`Rc`, not `Send`).
/// The host therefore schedules work cooperatively on that evaluator thread.
/// Submission only enqueues; polling or waiting on the result Promise, or an
/// explicit [`WorkHost::run`], advances the run.
#[derive(Clone)]
pub struct WorkHost {
    inner: Rc<RefCell<WorkHostInner>>,
}

impl fmt::Debug for WorkHost {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WorkHost")
            .field("status", &self.status())
            .finish()
    }
}

impl Default for WorkHost {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkHost {
    pub fn new() -> Self {
        Self {
            inner: Rc::new(RefCell::new(WorkHostInner {
                started: true,
                next_id: 1,
                runs: HashMap::new(),
                queue: VecDeque::new(),
            })),
        }
    }

    /// Submit work without executing it inline.
    pub fn submit<F>(&self, id: Option<&str>, task: F) -> Result<WorkRun, String>
    where
        F: FnOnce() -> Result<Value, String> + 'static,
    {
        let options = WorkOptions {
            id: id.map(WorkId::new).transpose()?,
            ..WorkOptions::default()
        };
        self.submit_scoped(options, move |_| task())
    }

    /// Submit work whose executor already returns a native structured rejection.
    pub fn submit_rejection<F>(&self, id: Option<&str>, task: F) -> Result<WorkRun, String>
    where
        F: FnOnce() -> Result<Value, PromiseRejection> + 'static,
    {
        let options = WorkOptions {
            id: id.map(WorkId::new).transpose()?,
            ..WorkOptions::default()
        };
        self.submit_scoped_rejection(options, move |_| task())
    }

    pub fn submit_scoped<F>(&self, options: WorkOptions, task: F) -> Result<WorkRun, String>
    where
        F: FnOnce(WorkContext) -> Result<Value, String> + 'static,
    {
        self.submit_scoped_rejection(options, move |context| task(context).map_err(work_failure))
    }

    pub fn submit_scoped_rejection<F>(
        &self,
        options: WorkOptions,
        task: F,
    ) -> Result<WorkRun, String>
    where
        F: FnOnce(WorkContext) -> Result<Value, PromiseRejection> + 'static,
    {
        let parent = if options.detached {
            None
        } else {
            current_work_context()
                .filter(|context| context.host.same_identity(self))
                .map(|context| context.run)
        };
        self.submit_with_parent(parent, options, Box::new(task))
    }

    fn submit_with_parent(
        &self,
        parent: Option<WorkRun>,
        options: WorkOptions,
        task: WorkTask,
    ) -> Result<WorkRun, String> {
        let mut host = self.inner.borrow_mut();
        if !host.started {
            return Err("native work host is stopped".into());
        }
        let deadline = resolve_deadline(&options, parent.as_ref());
        let id = match options.id.clone() {
            Some(id) => id,
            None => next_work_id(&mut host)?,
        };
        if host.runs.contains_key(&id) {
            return Err(format!("work run ID already exists: {id}"));
        }
        if parent
            .as_ref()
            .is_some_and(|parent| !parent.accepts_children())
        {
            return Err("parent work scope is closed".into());
        }

        let result = Promise::new();
        let run = WorkRun {
            inner: Rc::new(WorkRunInner {
                id: id.clone(),
                result,
                status: RefCell::new(WorkRunStatus {
                    id: id.clone(),
                    state: WorkRunState::Queued,
                    started_at_millis: now_millis(),
                    finished_at_millis: None,
                    error: None,
                    cancel_reason: None,
                    parent_id: parent.as_ref().map(WorkRun::work_id),
                    child_count: 0,
                    deadline_remaining_millis: deadline.map(deadline_remaining_millis),
                    detached: options.detached,
                }),
                host: Rc::downgrade(&self.inner),
                parent: parent.as_ref().map(|parent| Rc::downgrade(&parent.inner)),
                children: RefCell::new(HashMap::new()),
                deadline,
                cancellation: RefCell::new(None),
                body_done: Cell::new(false),
                body_outcome: RefCell::new(None),
                finalizers: RefCell::new(Vec::new()),
                finalizers_started: Cell::new(false),
                active_promise: RefCell::new(None),
                parent_notified: Cell::new(false),
                events: Rc::new(RefCell::new(WorkEventLog {
                    values: vec![work_event(
                        &id,
                        1,
                        "work/run-queued",
                        WorkRunState::Queued,
                        None,
                    )],
                    ..WorkEventLog::default()
                })),
            }),
        };
        if let Some(parent) = &parent {
            if !parent.attach_child(run.clone()) {
                return Err("parent work scope is closed".into());
            }
        }
        install_progress_hooks(self, &run);
        host.runs.insert(id.clone(), run.clone());
        host.queue.push_back(PendingWork { id, task });
        drop(host);
        run.check_deadline();
        Ok(run)
    }

    /// Resolve a live handle from a portable raw identifier.
    pub fn resolve(&self, reference: &str) -> Result<WorkRun, String> {
        let id = WorkId::new(reference)?;
        self.resolve_id(&id)
    }

    pub fn resolve_id(&self, id: &WorkId) -> Result<WorkRun, String> {
        let run = self
            .inner
            .borrow()
            .runs
            .get(id)
            .cloned()
            .ok_or_else(|| format!("unknown work run: {id}"))?;
        run.check_deadline();
        Ok(run)
    }

    /// Run one queued item by ID. Returns false when no runnable item remains.
    pub fn run(&self, id: &WorkId) -> bool {
        let (run, task) = {
            let mut host = self.inner.borrow_mut();
            let Some(index) = host.queue.iter().position(|pending| &pending.id == id) else {
                return false;
            };
            let pending = host.queue.remove(index).expect("queued work disappeared");
            let run = host
                .runs
                .get(id)
                .cloned()
                .expect("queued work has no live run");
            (run, pending.task)
        };
        run.check_deadline();
        if !run.mark_running() {
            return false;
        }
        let context = WorkContext {
            host: self.clone(),
            run: run.clone(),
        };
        let result = with_current_work_context(context.clone(), || task(context));
        match result {
            Ok(value) => run.settle_body(value),
            Err(error) => run.fail_body(error),
        }
        true
    }

    pub fn run_next(&self) -> bool {
        let id = self
            .inner
            .borrow()
            .queue
            .front()
            .map(|pending| pending.id.clone());
        id.is_some_and(|id| self.run(&id))
    }

    pub fn drain(&self) {
        while self.run_next() {}
    }

    fn progress(&self, id: &WorkId) {
        let _ = self.run(id);
        let run = self.resolve_id(id).ok();
        if let Some(run) = &run {
            run.progress_active_promise();
        }
        while run.as_ref().is_some_and(|run| !run.closed()) && self.run_next() {}
        if let Some(run) = run {
            run.progress_active_promise();
            run.check_deadline();
        }
    }

    fn wait_for(&self, id: &WorkId) {
        self.progress(id);
        if let Ok(run) = self.resolve_id(id) {
            let active = run.inner.active_promise.borrow().clone();
            if let Some(active) = active {
                active.wait_state();
            }
            self.progress(id);
        }
    }

    pub fn status(&self) -> WorkHostStatus {
        let host = self.inner.borrow();
        WorkHostStatus {
            state: if host.started { "started" } else { "stopped" },
            run_count: host.runs.len(),
            queued_count: host.queue.len(),
        }
    }

    pub fn started(&self) -> bool {
        self.inner.borrow().started
    }

    pub fn start(&self) {
        self.inner.borrow_mut().started = true;
    }

    pub fn stop(&self) {
        self.inner.borrow_mut().started = false;
    }

    pub fn kill(&self) {
        let runs = {
            let mut host = self.inner.borrow_mut();
            host.started = false;
            host.runs.values().cloned().collect::<Vec<_>>()
        };
        for run in runs {
            run.cancel(Value::Keyword("host-stopped".into()));
        }
    }

    pub fn same_identity(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.inner, &other.inner)
    }
}

impl IComponent for WorkHost {
    type Metadata = WorkHostStatus;

    fn props(&self) -> Self::Metadata {
        self.status()
    }

    fn status(&self) -> Self::Metadata {
        WorkHost::status(self)
    }

    fn started(&self) -> bool {
        WorkHost::started(self)
    }

    fn stopped(&self) -> bool {
        !WorkHost::started(self)
    }

    fn start(&mut self) {
        WorkHost::start(self);
    }

    fn stop(&mut self) {
        WorkHost::stop(self);
    }

    fn kill(&mut self) {
        WorkHost::kill(self);
    }
}

#[derive(Clone)]
struct CancellationRequest {
    reason: Value,
    rejection: PromiseRejection,
}

struct WorkRunInner {
    id: WorkId,
    result: Promise,
    status: RefCell<WorkRunStatus>,
    host: Weak<RefCell<WorkHostInner>>,
    parent: Option<Weak<WorkRunInner>>,
    children: RefCell<HashMap<WorkId, WorkRun>>,
    deadline: Option<Instant>,
    cancellation: RefCell<Option<CancellationRequest>>,
    body_done: Cell<bool>,
    body_outcome: RefCell<Option<Result<Value, PromiseRejection>>>,
    finalizers: RefCell<Vec<WorkFinalizer>>,
    finalizers_started: Cell<bool>,
    active_promise: RefCell<Option<Promise>>,
    parent_notified: Cell<bool>,
    events: Rc<RefCell<WorkEventLog>>,
}

#[derive(Default)]
struct WorkEventLog {
    values: Vec<Value>,
    closed: bool,
    cursors: Vec<Weak<RefCell<WorkEventCursor>>>,
}

struct WorkEventCursor {
    next: usize,
    pending: Option<Promise>,
    closed: bool,
}

/// Live process-owned handle returned immediately from submission.
#[derive(Clone)]
pub struct WorkRun {
    inner: Rc<WorkRunInner>,
}

impl fmt::Debug for WorkRun {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WorkRun")
            .field("status", &self.work_status())
            .finish()
    }
}

impl WorkRun {
    pub fn work_id(&self) -> WorkId {
        self.inner.id.clone()
    }

    pub fn work_status(&self) -> WorkRunStatus {
        self.check_deadline();
        let mut status = self.inner.status.borrow().clone();
        status.child_count = self.inner.children.borrow().len();
        status.deadline_remaining_millis = self.inner.deadline.map(deadline_remaining_millis);
        status
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.inner.deadline
    }

    pub fn cancellation_token(&self) -> WorkCancellationToken {
        WorkCancellationToken {
            run: Rc::downgrade(&self.inner),
        }
    }

    /// Return the same native result Promise on every call.
    pub fn work_result(&self) -> Promise {
        self.inner.result.clone()
    }

    pub fn work_cancel(&self, reason: Value) -> Promise {
        let result = Promise::new();
        result.resolve(Value::Bool(self.cancel(reason)));
        result
    }

    pub fn work_events(&self, after: usize) -> Value {
        let cursor = Rc::new(RefCell::new(WorkEventCursor {
            next: after,
            pending: None,
            closed: false,
        }));
        self.inner
            .events
            .borrow_mut()
            .cursors
            .push(Rc::downgrade(&cursor));
        let events = self.inner.events.clone();
        let next_cursor = cursor.clone();
        let next = Rc::new(move || event_next(&events, &next_cursor));
        let events = self.inner.events.clone();
        let close_cursor = cursor;
        let close = Rc::new(move || {
            let pending = {
                let mut cursor = close_cursor.borrow_mut();
                cursor.closed = true;
                cursor.pending.take()
            };
            if let Some(pending) = pending {
                pending.resolve(Value::Nil);
            }
            events.borrow_mut().cursors.retain(|candidate| {
                candidate
                    .upgrade()
                    .is_some_and(|candidate| !Rc::ptr_eq(&candidate, &close_cursor))
            });
            Ok(())
        });
        crate::core::host_stream(next, close)
    }

    /// Append one ordered domain event to this live run.
    pub fn emit(&self, kind: Value, data: Value) -> bool {
        if self.closed() {
            return false;
        }
        let kind = match kind {
            Value::Keyword(value) => value.to_string(),
            Value::Symbol(value) => value.to_string(),
            Value::String(value) => value,
            _ => return false,
        };
        self.publish_domain_event(&kind, data);
        true
    }

    pub fn cancel(&self, reason: Value) -> bool {
        let rejection = cancellation_rejection(reason.clone());
        {
            let mut cancellation = self.inner.cancellation.borrow_mut();
            if cancellation.is_some() || self.closed() {
                return false;
            }
            *cancellation = Some(CancellationRequest {
                reason: reason.clone(),
                rejection: rejection.clone(),
            });
        }

        let previous = {
            let mut status = self.inner.status.borrow_mut();
            let previous = status.state;
            if !previous.terminal() {
                status.state = WorkRunState::Cancelling;
                status.cancel_reason = Some(reason.clone());
            }
            previous
        };
        self.publish_event(
            "work/run-cancelling",
            WorkRunState::Cancelling,
            Some(reason.clone()),
            false,
        );
        if previous == WorkRunState::Queued {
            if let Some(host) = self.inner.host.upgrade() {
                host.borrow_mut()
                    .queue
                    .retain(|pending| pending.id != self.inner.id);
            }
            self.inner.body_done.set(true);
        }
        let active = self.inner.active_promise.borrow().clone();
        if let Some(active) = active {
            active.cancel();
        }
        let children = self
            .inner
            .children
            .borrow()
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for child in children {
            child.cancel(reason.clone());
        }
        self.finish_if_ready();
        true
    }

    pub fn closed(&self) -> bool {
        self.inner.status.borrow().state.terminal()
    }

    pub fn same_identity(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.inner, &other.inner)
    }

    fn accepts_children(&self) -> bool {
        !self.inner.body_done.get()
            && !self.inner.finalizers_started.get()
            && self.inner.cancellation.borrow().is_none()
            && !self.closed()
    }

    fn attach_child(&self, child: WorkRun) -> bool {
        if !self.accepts_children() {
            return false;
        }
        self.inner
            .children
            .borrow_mut()
            .insert(child.work_id(), child);
        true
    }

    fn child_closed(&self, child: &WorkRun) {
        self.inner.children.borrow_mut().remove(&child.work_id());
        self.finish_if_ready();
    }

    fn mark_running(&self) -> bool {
        self.check_deadline();
        let mut status = self.inner.status.borrow_mut();
        if status.state != WorkRunState::Queued {
            return false;
        }
        status.state = WorkRunState::Running;
        drop(status);
        self.publish_event("work/run-running", WorkRunState::Running, None, false);
        true
    }

    fn settle_body(&self, value: Value) {
        if let Value::Promise(source) = value {
            if source.same_identity(&self.inner.result) {
                self.fail_body(work_failure("work result promise adoption cycle".into()));
                return;
            }
            *self.inner.active_promise.borrow_mut() = Some(source.clone());
            if self.inner.cancellation.borrow().is_some() {
                source.cancel();
            }
            let run = Rc::downgrade(&self.inner);
            source.on_settle(Rc::new(move |state| {
                let Some(inner) = run.upgrade() else {
                    return;
                };
                let run = WorkRun { inner };
                run.inner.active_promise.borrow_mut().take();
                match state {
                    PromiseState::Pending => return,
                    PromiseState::Fulfilled(value) => {
                        *run.inner.body_outcome.borrow_mut() = Some(Ok(value));
                    }
                    PromiseState::Rejected(error) => {
                        *run.inner.body_outcome.borrow_mut() = Some(Err(error));
                    }
                }
                run.inner.body_done.set(true);
                run.finish_if_ready();
            }));
            self.set_nonterminal_state(if self.inner.cancellation.borrow().is_some() {
                WorkRunState::Cancelling
            } else {
                WorkRunState::Waiting
            });
            source.state();
            return;
        }
        *self.inner.body_outcome.borrow_mut() = Some(Ok(value));
        self.inner.body_done.set(true);
        self.finish_if_ready();
    }

    fn fail_body(&self, error: PromiseRejection) {
        *self.inner.body_outcome.borrow_mut() = Some(Err(error));
        self.inner.body_done.set(true);
        self.finish_if_ready();
    }

    fn progress_active_promise(&self) {
        let active = self.inner.active_promise.borrow().clone();
        if let Some(active) = active {
            active.state();
        }
    }

    fn check_deadline(&self) {
        if self
            .inner
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.cancel(Value::Keyword("deadline-exceeded".into()));
        }
    }

    fn register_finalizer(&self, finalizer: WorkFinalizer) -> bool {
        if self.inner.finalizers_started.get() || self.closed() {
            return false;
        }
        self.inner.finalizers.borrow_mut().push(finalizer);
        true
    }

    fn finish_if_ready(&self) {
        if !self.inner.body_done.get() {
            return;
        }
        if !self.inner.children.borrow().is_empty() {
            self.set_nonterminal_state(if self.inner.cancellation.borrow().is_some() {
                WorkRunState::Cancelling
            } else {
                WorkRunState::Waiting
            });
            return;
        }
        if self.inner.finalizers_started.replace(true) {
            return;
        }

        let context = self.context();
        let mut finalizer_error = None;
        let mut finalizers = std::mem::take(&mut *self.inner.finalizers.borrow_mut());
        while let Some(finalizer) = finalizers.pop() {
            let result = with_current_work_context(context.clone(), || finalizer(context.clone()));
            if finalizer_error.is_none() {
                if let Err(error) = result {
                    finalizer_error = Some(error);
                }
            }
        }

        if let Some(cancellation) = self.inner.cancellation.borrow().clone() {
            self.settle_terminal(
                WorkRunState::Cancelled,
                Some(cancellation.rejection.clone()),
                Some(cancellation.reason),
                None,
            );
            return;
        }
        if let Some(error) = finalizer_error {
            self.settle_terminal(WorkRunState::Failed, Some(error), None, None);
            return;
        }
        match self.inner.body_outcome.borrow_mut().take() {
            Some(Ok(value)) => {
                self.settle_terminal(WorkRunState::Completed, None, None, Some(value));
            }
            Some(Err(error)) => {
                self.settle_terminal(WorkRunState::Failed, Some(error), None, None);
            }
            None => {
                self.settle_terminal(
                    WorkRunState::Failed,
                    Some(work_failure("work body produced no outcome".into())),
                    None,
                    None,
                );
            }
        }
    }

    fn settle_terminal(
        &self,
        state: WorkRunState,
        error: Option<PromiseRejection>,
        cancel_reason: Option<Value>,
        value: Option<Value>,
    ) -> bool {
        {
            let mut status = self.inner.status.borrow_mut();
            if status.state.terminal() {
                return false;
            }
            status.state = state;
            status.finished_at_millis = Some(now_millis());
            status.error = error.clone();
            status.cancel_reason = cancel_reason.clone();
        }
        let event_detail = if state == WorkRunState::Failed {
            error.as_ref().map(PromiseRejection::value)
        } else {
            cancel_reason
        };
        match state {
            WorkRunState::Completed => {
                self.inner.result.resolve(value.unwrap_or(Value::Nil));
            }
            WorkRunState::Failed | WorkRunState::Cancelled => {
                self.inner
                    .result
                    .reject_rejection(error.expect("terminal failure requires rejection"));
            }
            _ => unreachable!("non-terminal state passed to settle_terminal"),
        }
        self.publish_event(work_event_type(state), state, event_detail, true);
        self.notify_parent();
        true
    }

    fn set_nonterminal_state(&self, state: WorkRunState) {
        let mut status = self.inner.status.borrow_mut();
        let changed = !status.state.terminal() && status.state != state;
        if changed {
            status.state = state;
        }
        drop(status);
        if changed && state == WorkRunState::Waiting {
            self.publish_event("work/run-waiting", state, None, false);
        }
    }

    fn notify_parent(&self) {
        if self.inner.parent_notified.replace(true) {
            return;
        }
        let Some(parent) = self.inner.parent.as_ref().and_then(Weak::upgrade) else {
            return;
        };
        WorkRun { inner: parent }.child_closed(self);
    }

    fn context(&self) -> WorkContext {
        let host = self
            .inner
            .host
            .upgrade()
            .map(|inner| WorkHost { inner })
            .expect("work host was dropped while run remained live");
        WorkContext {
            host,
            run: self.clone(),
        }
    }

    fn publish_event(
        &self,
        kind: &str,
        state: WorkRunState,
        detail: Option<Value>,
        terminal: bool,
    ) {
        let pending = {
            let mut events = self.inner.events.borrow_mut();
            let sequence = events.values.len() + 1;
            events
                .values
                .push(work_event(&self.inner.id, sequence, kind, state, detail));
            if terminal {
                events.closed = true;
            }
            let mut pending = Vec::new();
            let mut retained = Vec::new();
            for weak in std::mem::take(&mut events.cursors) {
                let Some(cursor) = weak.upgrade() else {
                    continue;
                };
                if let Some(settlement) = event_take(&events, &mut cursor.borrow_mut()) {
                    pending.push(settlement);
                }
                retained.push(Rc::downgrade(&cursor));
            }
            events.cursors = retained;
            pending
        };
        for (promise, value) in pending {
            promise.resolve(value);
        }
    }

    fn publish_domain_event(&self, kind: &str, data: Value) {
        let pending = {
            let mut events = self.inner.events.borrow_mut();
            let sequence = events.values.len() + 1;
            events.values.push(Value::Map(
                [
                    (
                        Value::Keyword("event/type".into()),
                        Value::Keyword(kind.into()),
                    ),
                    (
                        Value::Keyword("event/run".into()),
                        Value::String(self.inner.id.to_string()),
                    ),
                    (
                        Value::Keyword("event/sequence".into()),
                        Value::Number(sequence as i64),
                    ),
                    (Value::Keyword("event/data".into()), data),
                ]
                .into_iter()
                .collect(),
            ));
            let mut pending = Vec::new();
            let mut retained = Vec::new();
            for weak in std::mem::take(&mut events.cursors) {
                let Some(cursor) = weak.upgrade() else {
                    continue;
                };
                if let Some(settlement) = event_take(&events, &mut cursor.borrow_mut()) {
                    pending.push(settlement);
                }
                retained.push(Rc::downgrade(&cursor));
            }
            events.cursors = retained;
            pending
        };
        for (promise, value) in pending {
            promise.resolve(value);
        }
    }
}

fn work_event(
    id: &WorkId,
    sequence: usize,
    kind: &str,
    state: WorkRunState,
    detail: Option<Value>,
) -> Value {
    let data = Value::Map(
        [
            (
                Value::Keyword("state".into()),
                Value::Keyword(work_state_name(state).into()),
            ),
            (
                Value::Keyword("detail".into()),
                detail.unwrap_or(Value::Nil),
            ),
        ]
        .into_iter()
        .collect(),
    );
    Value::Map(
        [
            (
                Value::Keyword("event/type".into()),
                Value::Keyword(kind.into()),
            ),
            (
                Value::Keyword("event/run".into()),
                Value::String(id.to_string()),
            ),
            (
                Value::Keyword("event/sequence".into()),
                Value::Number(sequence as i64),
            ),
            (Value::Keyword("event/data".into()), data),
        ]
        .into_iter()
        .collect(),
    )
}

fn work_state_name(state: WorkRunState) -> &'static str {
    match state {
        WorkRunState::Queued => "queued",
        WorkRunState::Running => "running",
        WorkRunState::Waiting => "waiting",
        WorkRunState::Cancelling => "cancelling",
        WorkRunState::Completed => "completed",
        WorkRunState::Failed => "failed",
        WorkRunState::Cancelled => "cancelled",
    }
}

fn work_event_type(state: WorkRunState) -> &'static str {
    match state {
        WorkRunState::Completed => "work/run-completed",
        WorkRunState::Failed => "work/run-failed",
        WorkRunState::Cancelled => "work/run-cancelled",
        _ => unreachable!("terminal work event requires a terminal state"),
    }
}

fn event_take(events: &WorkEventLog, cursor: &mut WorkEventCursor) -> Option<(Promise, Value)> {
    let promise = cursor.pending.take()?;
    if cursor.next < events.values.len() {
        let value = events.values[cursor.next].clone();
        cursor.next += 1;
        return Some((promise, value));
    }
    if events.closed || cursor.closed {
        cursor.closed = true;
        return Some((promise, Value::Nil));
    }
    cursor.pending = Some(promise);
    None
}

fn event_next(
    events: &Rc<RefCell<WorkEventLog>>,
    cursor: &Rc<RefCell<WorkEventCursor>>,
) -> Result<Promise, String> {
    let promise = Promise::new();
    let settlement = {
        let events = events.borrow();
        let mut cursor = cursor.borrow_mut();
        if cursor.closed {
            Some((promise.clone(), Value::Nil))
        } else if cursor.pending.is_some() {
            return Err("stream/pending-pull: only one Stream/next may be pending".into());
        } else {
            cursor.pending = Some(promise.clone());
            event_take(&events, &mut cursor)
        }
    };
    if let Some((target, value)) = settlement {
        target.resolve(value);
    }
    Ok(promise)
}

#[cfg(test)]
mod tests;
