use super::*;

/// Cooperative cancellation token exposed through the active work context.
#[derive(Clone)]
pub struct WorkCancellationToken {
    pub(super) run: Weak<WorkRunInner>,
}

impl WorkCancellationToken {
    pub fn cancelled(&self) -> bool {
        self.run
            .upgrade()
            .is_none_or(|run| run.cancellation.borrow().is_some())
    }

    pub fn reason(&self) -> Option<Value> {
        self.run.upgrade().and_then(|run| {
            run.cancellation
                .borrow()
                .as_ref()
                .map(|request| request.reason.clone())
        })
    }

    pub fn check(&self) -> Result<(), PromiseRejection> {
        let Some(run) = self.run.upgrade() else {
            return Err(cancellation_rejection(Value::Keyword(
                "scope-closed".into(),
            )));
        };
        let run = WorkRun { inner: run };
        run.check_deadline();
        let result = match run.inner.cancellation.borrow().as_ref() {
            Some(request) => Err(request.rejection.clone()),
            None => Ok(()),
        };
        result
    }
}

/// Opaque evaluator-thread context for one native work scope.
#[derive(Clone)]
pub struct WorkContext {
    pub(super) host: WorkHost,
    pub(super) run: WorkRun,
}

impl WorkContext {
    pub fn work_id(&self) -> WorkId {
        self.run.work_id()
    }

    pub fn token(&self) -> WorkCancellationToken {
        self.run.cancellation_token()
    }

    pub fn cancelled(&self) -> bool {
        self.token().cancelled()
    }

    pub fn cancel_reason(&self) -> Option<Value> {
        self.token().reason()
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.run.deadline()
    }

    pub fn deadline_nanos(&self) -> Option<u64> {
        let deadline = self.deadline()?;
        let now = Instant::now();
        let remaining =
            u64::try_from(deadline.saturating_duration_since(now).as_nanos()).unwrap_or(u64::MAX);
        Some(monotonic_nanos().saturating_add(remaining))
    }

    pub fn check_cancelled(&self) -> Result<(), PromiseRejection> {
        self.token().check()
    }

    pub fn emit(&self, kind: Value, data: Value) -> bool {
        self.run.emit(kind, data)
    }

    pub fn submit_child<F>(&self, options: WorkOptions, task: F) -> Result<WorkRun, String>
    where
        F: FnOnce(WorkContext) -> Result<Value, String> + 'static,
    {
        self.check_cancelled().map_err(|error| error.message())?;
        let parent = if options.detached {
            None
        } else {
            Some(self.run.clone())
        };
        self.host.submit_with_parent(
            parent,
            options,
            Box::new(move |context| task(context).map_err(work_failure)),
        )
    }

    pub fn submit_child_rejection<F>(
        &self,
        options: WorkOptions,
        task: F,
    ) -> Result<WorkRun, String>
    where
        F: FnOnce(WorkContext) -> Result<Value, PromiseRejection> + 'static,
    {
        self.check_cancelled().map_err(|error| error.message())?;
        let parent = if options.detached {
            None
        } else {
            Some(self.run.clone())
        };
        self.host
            .submit_with_parent(parent, options, Box::new(task))
    }

    pub fn on_close<F>(&self, finalizer: F) -> bool
    where
        F: FnOnce(WorkContext) -> Result<(), PromiseRejection> + 'static,
    {
        self.run.register_finalizer(Box::new(finalizer))
    }
}

thread_local! {
    static PROCESS_WORK_HOST: WorkHost = WorkHost::new();
    static CURRENT_WORK_CONTEXT: RefCell<Option<WorkContext>> = const { RefCell::new(None) };
    static MONOTONIC_ORIGIN: Instant = Instant::now();
}

pub fn monotonic_nanos() -> u64 {
    MONOTONIC_ORIGIN.with(|origin| origin.elapsed().as_nanos() as u64)
}

/// Return the process/evaluator-thread host shared by independent sessions.
pub fn process_work_host() -> WorkHost {
    PROCESS_WORK_HOST.with(Clone::clone)
}

/// Return the currently executing cooperative work context, if any.
pub fn current_work_context() -> Option<WorkContext> {
    CURRENT_WORK_CONTEXT.with(|current| current.borrow().clone())
}

pub(crate) fn with_current_work_context<T>(
    context: WorkContext,
    function: impl FnOnce() -> T,
) -> T {
    let previous = CURRENT_WORK_CONTEXT.with(|current| current.replace(Some(context)));
    let result = function();
    CURRENT_WORK_CONTEXT.with(|current| {
        current.replace(previous);
    });
    result
}

pub(super) fn install_progress_hooks(host: &WorkHost, run: &WorkRun) {
    let weak_host = Rc::downgrade(&host.inner);
    let id = run.work_id();
    run.inner.result.set_poller(Rc::new(move || {
        if let Some(inner) = weak_host.upgrade() {
            WorkHost { inner }.progress(&id);
        }
    }));

    let weak_host = Rc::downgrade(&host.inner);
    let id = run.work_id();
    run.inner.result.set_waiter(Rc::new(move || {
        if let Some(inner) = weak_host.upgrade() {
            WorkHost { inner }.wait_for(&id);
        }
    }));

    let weak_run = Rc::downgrade(&run.inner);
    run.inner.result.set_cancel_hook(Rc::new(move || {
        if let Some(inner) = weak_run.upgrade() {
            WorkRun { inner }.cancel(Value::Keyword("result-cancelled".into()));
        }
    }));
}

pub(super) fn resolve_deadline(options: &WorkOptions, parent: Option<&WorkRun>) -> Option<Instant> {
    let inherited = parent.and_then(WorkRun::deadline);
    let relative = options
        .timeout
        .and_then(|timeout| Instant::now().checked_add(timeout));
    [inherited, options.deadline, relative]
        .into_iter()
        .flatten()
        .min()
}

pub(super) fn deadline_remaining_millis(deadline: Instant) -> u64 {
    u64::try_from(
        deadline
            .saturating_duration_since(Instant::now())
            .as_millis(),
    )
    .unwrap_or(u64::MAX)
}

pub(super) fn next_work_id(host: &mut WorkHostInner) -> Result<WorkId, String> {
    loop {
        let id = WorkId::new(format!("run-{}", host.next_id))?;
        host.next_id = host
            .next_id
            .checked_add(1)
            .ok_or_else(|| "work run identifiers exhausted".to_string())?;
        if !host.runs.contains_key(&id) {
            return Ok(id);
        }
    }
}

pub(super) fn work_failure(message: String) -> PromiseRejection {
    PromiseRejection::Value(Value::Map(
        [
            (
                Value::Keyword("code".into()),
                Value::Keyword("work/failed".into()),
            ),
            (Value::Keyword("message".into()), Value::String(message)),
            (Value::Keyword("retryable".into()), Value::Bool(false)),
        ]
        .into_iter()
        .collect(),
    ))
}

pub(super) fn cancellation_rejection(reason: Value) -> PromiseRejection {
    PromiseRejection::Cancelled(Value::Map(
        [
            (
                Value::Keyword("code".into()),
                Value::Keyword("work/cancelled".into()),
            ),
            (Value::Keyword("reason".into()), reason),
            (Value::Keyword("retryable".into()), Value::Bool(false)),
        ]
        .into_iter()
        .collect(),
    ))
}

pub(super) fn now_millis() -> u64 {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    u64::try_from(millis).unwrap_or(u64::MAX)
}
