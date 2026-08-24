package hara.truffle;

import hara.lang.data.Keyword;
import hara.lang.data.Map;
import hara.lang.data.Symbol;
import hara.lang.protocol.IMapType;
import hara.lang.protocol.IFn;
import hara.lang.protocol.IMetadata;
import hara.lang.protocol.IPromise;
import hara.lang.protocol.IStream;
import hara.lang.protocol.IWorkHost;
import hara.lang.protocol.IWorkRef;
import hara.lang.protocol.IWorkRun;
import java.util.ArrayList;
import java.util.List;
import java.util.UUID;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.CompletionException;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.ConcurrentLinkedDeque;
import java.util.concurrent.ConcurrentMap;
import java.util.concurrent.ScheduledFuture;
import java.util.concurrent.ScheduledThreadPoolExecutor;
import java.util.concurrent.ThreadFactory;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.concurrent.atomic.AtomicReference;

/** Process-owned native work host and live run registry. */
public final class HaraWorkHost implements IWorkHost {
  static final HaraWorkHost INSTANCE = new HaraWorkHost();

  private static final Keyword ID = Keyword.create("id");
  private static final Keyword WORK_ID = Keyword.create("work", "id");
  private static final Keyword STATUS_WORK_ID = Keyword.create("work-id");
  private static final Keyword RUN_ID = Keyword.create("run", "id");
  private static final Keyword EXECUTE = Keyword.create("work", "execute");
  private static final Keyword TYPE = Keyword.create("type");
  private static final Keyword SCOPE = Keyword.create("scope");
  private static final Keyword STATE = Keyword.create("state");
  private static final Keyword RUN_COUNT = Keyword.create("run-count");
  private static final Keyword WORK_HOST = Keyword.create("work-host");
  private static final Keyword PROCESS = Keyword.create("process");
  private static final Keyword STARTED = Keyword.create("started");
  private static final Keyword STOPPED = Keyword.create("stopped");
  private static final Keyword CREATED = Keyword.create("created");
  private static final Keyword QUEUED = Keyword.create("queued");
  private static final Keyword RUNNING = Keyword.create("running");
  private static final Keyword WAITING = Keyword.create("waiting");
  private static final Keyword CANCELLING = Keyword.create("cancelling");
  private static final Keyword COMPLETED = Keyword.create("completed");
  private static final Keyword FAILED = Keyword.create("failed");
  private static final Keyword CANCELLED = Keyword.create("cancelled");
  private static final Keyword STARTED_AT = Keyword.create("started-at");
  private static final Keyword FINISHED_AT = Keyword.create("finished-at");
  private static final Keyword ERROR = Keyword.create("error");
  private static final Keyword CANCEL_REASON = Keyword.create("cancel-reason");
  private static final Keyword DEADLINE_NANOS = Keyword.create("deadline-nanos");
  private static final Keyword TIMEOUT_MS = Keyword.create("timeout-ms");
  private static final Keyword DETACHED = Keyword.create("detached");
  private static final Keyword PARENT_ID = Keyword.create("parent-id");
  private static final Keyword CHILD_COUNT = Keyword.create("child-count");
  private static final Keyword DEADLINE_EXCEEDED = Keyword.create("deadline-exceeded");
  private static final Keyword HOST_STOPPED = Keyword.create("host-stopped");
  private static final Keyword EVENT_TYPE = Keyword.create("event", "type");
  private static final Keyword EVENT_RUN = Keyword.create("event", "run");
  private static final Keyword EVENT_SEQUENCE = Keyword.create("event", "sequence");
  private static final Keyword EVENT_DATA = Keyword.create("event", "data");
  private static final Keyword RUN_QUEUED = Keyword.create("work", "run-queued");
  private static final Keyword RUN_RUNNING = Keyword.create("work", "run-running");
  private static final Keyword RUN_WAITING = Keyword.create("work", "run-waiting");
  private static final Keyword RUN_CANCELLING = Keyword.create("work", "run-cancelling");
  private static final Keyword RUN_COMPLETED = Keyword.create("work", "run-completed");
  private static final Keyword RUN_FAILED = Keyword.create("work", "run-failed");
  private static final Keyword RUN_CANCELLED = Keyword.create("work", "run-cancelled");

  private static final ThreadLocal<WorkContext> CURRENT_CONTEXT = new ThreadLocal<>();
  private static final ScheduledThreadPoolExecutor DEADLINES = deadlineScheduler();

  private final ConcurrentMap<Object, HaraWorkRun> runs = new ConcurrentHashMap<>();
  private final AtomicBoolean started = new AtomicBoolean(true);

  private HaraWorkHost() {}

  static HaraWorkHost instance() {
    return INSTANCE;
  }

  static WorkContext currentWorkContext() {
    return CURRENT_CONTEXT.get();
  }

  static Object workStatusSnapshot(IWorkRun value) {
    if (value instanceof HaraWorkRun run) return run.statusSnapshot();
    throw new HaraException("Work status snapshots require a native work run");
  }

  @Override
  public IWorkRun workSubmit(Object work, Object input, Object options) {
    return workSubmit(HaraLanguage.currentContext(), work, input, options);
  }

  IWorkRun workSubmit(HaraContext context, Object work, Object input, Object options) {
    WorkContext current = CURRENT_CONTEXT.get();
    HaraWorkRun parent =
        current != null && current.host == this && !truthy(option(options, DETACHED))
            ? current.run
            : null;
    return workSubmit(context, parent, work, input, options);
  }

  private IWorkRun workSubmit(
      HaraContext context, HaraWorkRun parent, Object work, Object input, Object options) {
    if (!started.get()) {
      throw new HaraException("Native work host is stopped");
    }
    Object executor = option(options, EXECUTE);
    if (executor == null && work instanceof IFn<?, ?, ?>) {
      executor = work;
    }
    if (executor == null) {
      throw new HaraException(
          "work-submit requires callable work or a :work/execute adapter");
    }

    Object requestedId =
        firstNonNull(
            option(options, ID),
            firstNonNull(option(options, RUN_ID), option(options, WORK_ID)));
    Object runId = requestedId == null ? nextRunId() : validateRunId(requestedId);
    long deadlineNanos = resolveDeadlineNanos(options, parent);
    boolean detached = parent == null && truthy(option(options, DETACHED));
    HaraWorkRun run =
        new HaraWorkRun(
            context,
            runId,
            work,
            input,
            options,
            executor,
            parent,
            deadlineNanos,
            detached);
    HaraWorkRun previous = runs.putIfAbsent(runId, run);
    if (previous != null) {
      throw new HaraException("Work run ID is already active: " + runId);
    }
    if (parent != null && !parent.attachChild(run)) {
      runs.remove(runId, run);
      throw new HaraException("Parent work scope is closed: " + parent.runId);
    }
    run.start();
    return run;
  }

  @Override
  public IWorkRun workResolve(Object reference) {
    Object runId = referenceId(reference);
    HaraWorkRun run = runs.get(runId);
    if (run == null) {
      throw new HaraException("Unknown work run: " + runId);
    }
    return run;
  }

  @Override
  public IMetadata getProps() {
    return Map.Standard.from(null, TYPE, WORK_HOST, SCOPE, PROCESS);
  }

  @Override
  public IMetadata getStatus() {
    return Map.Standard.from(
        null, STATE, started.get() ? STARTED : STOPPED, RUN_COUNT, (long) runs.size());
  }

  @Override
  public boolean isStarted() {
    return started.get();
  }

  @Override
  public boolean isStopped() {
    return !started.get();
  }

  @Override
  public IWorkHost start() {
    started.set(true);
    return this;
  }

  @Override
  public IWorkHost stop() {
    started.set(false);
    return this;
  }

  @Override
  public IWorkHost kill() {
    started.set(false);
    for (HaraWorkRun run : runs.values()) run.requestCancellation(HOST_STOPPED);
    return this;
  }

  private Object nextRunId() {
    Object candidate;
    do {
      candidate = UUID.randomUUID().toString();
    } while (runs.containsKey(candidate));
    return candidate;
  }

  private static Object validateRunId(Object value) {
    if (value == null) {
      throw new HaraException("Work run ID cannot be nil");
    }
    if (value instanceof String string) {
      if (string.isBlank()) {
        throw new HaraException("Work run ID cannot be blank");
      }
      return string;
    }
    if (value instanceof Keyword || value instanceof Symbol || value instanceof Number) {
      return value;
    }
    throw new HaraException(
        "Work run ID must be a string, keyword, symbol, or number: "
            + value.getClass().getName());
  }

  private static Object referenceId(Object reference) {
    if (reference instanceof IWorkRef workRef) {
      return validateRunId(workRef.workId());
    }
    Object workId = option(reference, WORK_ID);
    if (workId != null) return validateRunId(workId);
    Object runId = option(reference, RUN_ID);
    if (runId != null) return validateRunId(runId);
    Object id = option(reference, ID);
    return validateRunId(id == null ? reference : id);
  }

  private static long resolveDeadlineNanos(Object options, HaraWorkRun parent) {
    long inherited = parent == null ? 0L : parent.deadlineNanos;
    long explicit = positiveLong(option(options, DEADLINE_NANOS), DEADLINE_NANOS);
    Object timeoutValue = option(options, TIMEOUT_MS);
    long relative = 0L;
    if (timeoutValue != null) {
      if (!(timeoutValue instanceof Number number)) {
        throw new HaraException(":timeout-ms must be a non-negative number");
      }
      long timeoutMillis = number.longValue();
      if (timeoutMillis < 0L) {
        throw new HaraException(":timeout-ms must be non-negative");
      }
      relative = saturatingAdd(System.nanoTime(), saturatingMultiply(timeoutMillis, 1_000_000L));
    }
    return minimumPositive(inherited, minimumPositive(explicit, relative));
  }

  private static long positiveLong(Object value, Keyword key) {
    if (value == null) return 0L;
    if (!(value instanceof Number number)) {
      throw new HaraException(key + " must be a positive number");
    }
    long result = number.longValue();
    if (result <= 0L) {
      throw new HaraException(key + " must be positive");
    }
    return result;
  }

  private static long minimumPositive(long first, long second) {
    if (first <= 0L) return second;
    if (second <= 0L) return first;
    return Math.min(first, second);
  }

  private static long saturatingAdd(long first, long second) {
    try {
      return Math.addExact(first, second);
    } catch (ArithmeticException ignored) {
      return Long.MAX_VALUE;
    }
  }

  private static long saturatingMultiply(long first, long second) {
    try {
      return Math.multiplyExact(first, second);
    } catch (ArithmeticException ignored) {
      return Long.MAX_VALUE;
    }
  }

  @SuppressWarnings({"rawtypes", "unchecked"})
  private static Object option(Object options, Keyword key) {
    if (options instanceof IMapType map) {
      return map.lookup(key);
    }
    if (options instanceof java.util.Map map) {
      return map.get(key);
    }
    return null;
  }

  private static boolean truthy(Object value) {
    return value != null && !Boolean.FALSE.equals(value);
  }

  private static Object firstNonNull(Object first, Object second) {
    return first == null ? second : first;
  }

  private static ScheduledThreadPoolExecutor deadlineScheduler() {
    ThreadFactory factory =
        task -> Thread.ofPlatform().daemon().name("hara-work-deadline").unstarted(task);
    ScheduledThreadPoolExecutor scheduler = new ScheduledThreadPoolExecutor(1, factory);
    scheduler.setRemoveOnCancelPolicy(true);
    scheduler.setExecuteExistingDelayedTasksAfterShutdownPolicy(false);
    return scheduler;
  }

  /** Opaque native context for the currently executing work scope. */
  public static final class WorkContext {
    private final HaraWorkHost host;
    private final HaraWorkRun run;

    private WorkContext(HaraWorkHost host, HaraWorkRun run) {
      this.host = host;
      this.run = run;
    }

    public Object workId() {
      return run.runId;
    }

    IWorkRun currentRun() {
      return run;
    }

    public boolean cancelled() {
      return run.snapshot.get().cancellation != null;
    }

    public Object cancelReason() {
      CancellationRequest request = run.snapshot.get().cancellation;
      return request == null ? null : request.reason;
    }

    public Long deadlineNanos() {
      return run.deadlineNanos <= 0L ? null : run.deadlineNanos;
    }

    public void checkCancelled() {
      CancellationRequest request = run.snapshot.get().cancellation;
      if (request != null) {
        throw request.failure;
      }
      if (run.deadlineNanos > 0L && System.nanoTime() >= run.deadlineNanos) {
        run.requestCancellation(DEADLINE_EXCEEDED);
        request = run.snapshot.get().cancellation;
        if (request != null) throw request.failure;
      }
    }

    public boolean emit(Object type, Object data) {
      if (run.closed()) return false;
      Keyword eventType;
      if (type instanceof Keyword keyword) {
        eventType = keyword;
      } else if (type instanceof Symbol symbol) {
        eventType = Keyword.create(symbol.getNamespace(), symbol.getName());
      } else if (type instanceof String name) {
        eventType = Keyword.create(name.replaceFirst("^:", ""));
      } else {
        return false;
      }
      run.publishEvent(eventType, data, false);
      return true;
    }

    public IWorkRun submitChild(Object work, Object input, Object options) {
      checkCancelled();
      boolean detached = truthy(option(options, DETACHED));
      return host.workSubmit(run.context, detached ? null : run, work, input, options);
    }

    public boolean onClose(Object function) {
      if (function == null) {
        throw new HaraException("Work finalizer cannot be nil");
      }
      return run.registerFinalizer(function);
    }
  }

  private final class HaraWorkRun implements IWorkRun {
    private final HaraContext context;
    private final Object runId;
    private final Object work;
    private final Object input;
    private final Object options;
    private final Object executor;
    private final HaraWorkRun parent;
    private final boolean detached;
    private final long deadlineNanos;
    private final CompletableFuture<Object> resultFuture = new CompletableFuture<>();
    private final IPromise result;
    private final AtomicReference<RunSnapshot> snapshot;
    private final AtomicReference<Thread> worker = new AtomicReference<>();
    private final AtomicReference<IPromise> activePromise = new AtomicReference<>();
    private final AtomicReference<BodyOutcome> bodyOutcome = new AtomicReference<>();
    private final AtomicBoolean bodyDone = new AtomicBoolean(false);
    private final AtomicBoolean finalizersStarted = new AtomicBoolean(false);
    private final AtomicBoolean parentNotified = new AtomicBoolean(false);
    private final ConcurrentMap<Object, HaraWorkRun> children = new ConcurrentHashMap<>();
    private final ConcurrentLinkedDeque<Object> finalizers = new ConcurrentLinkedDeque<>();
    private final Object eventLock = new Object();
    private final List<Object> events = new ArrayList<>();
    private final List<WorkEventStream> eventStreams = new ArrayList<>();
    private long eventSequence;
    private boolean eventsClosed;
    private final WorkContext workContext;
    private volatile ScheduledFuture<?> deadlineTask;

    HaraWorkRun(
        HaraContext context,
        Object runId,
        Object work,
        Object input,
        Object options,
        Object executor,
        HaraWorkRun parent,
        long deadlineNanos,
        boolean detached) {
      this.context = context;
      this.runId = runId;
      this.work = work;
      this.input = input;
      this.options = options;
      this.executor = executor;
      this.parent = parent;
      this.deadlineNanos = deadlineNanos;
      this.detached = detached;
      this.result = (IPromise) context.promiseValue(resultFuture);
      this.snapshot = new AtomicReference<>(new RunSnapshot(CREATED, 0L, 0L, null, null));
      this.workContext = new WorkContext(HaraWorkHost.this, this);
      publishEvent(RUN_QUEUED, Map.Standard.from(null, STATE, QUEUED), false);
    }

    void start() {
      long startedAt = System.currentTimeMillis();
      RunSnapshot current = snapshot.get();
      if (!CREATED.equals(current.state)
          || !snapshot.compareAndSet(
              current, new RunSnapshot(QUEUED, startedAt, 0L, null, null))) {
        return;
      }
      scheduleDeadline();
      if (snapshot.get().cancellation != null) {
        bodyDone.set(true);
        finishIfReady();
        return;
      }
      Thread thread = Thread.ofVirtual().name("hara-work-" + runId).unstarted(this::execute);
      worker.set(thread);
      thread.start();
    }

    private void scheduleDeadline() {
      if (deadlineNanos <= 0L) return;
      long delay = Math.max(0L, deadlineNanos - System.nanoTime());
      deadlineTask =
          DEADLINES.schedule(
              () -> requestCancellation(DEADLINE_EXCEEDED), delay, TimeUnit.NANOSECONDS);
    }

    private void execute() {
      if (!transition(QUEUED, RUNNING, null, null)) {
        bodyDone.set(true);
        finishIfReady();
        return;
      }
      publishEvent(RUN_RUNNING, Map.Standard.from(null, STATE, RUNNING), false);
      CURRENT_CONTEXT.set(workContext);
      try {
        workContext.checkCancelled();
        Object value =
            context.invokeInContext(
                () -> context.invokeCallable(executor, new Object[] {work, input, options, runId}));
        if (value instanceof IPromise promise) {
          activePromise.set(promise);
          transitionAnyNonTerminal(WAITING, null);
          if (snapshot.get().cancellation != null) promise.cancel();
          try {
            value = promise.deref();
          } finally {
            activePromise.compareAndSet(promise, null);
          }
        }
        bodyOutcome.set(BodyOutcome.success(value));
      } catch (Throwable error) {
        bodyOutcome.set(BodyOutcome.failure(unwrap(error)));
      } finally {
        CURRENT_CONTEXT.remove();
        bodyDone.set(true);
        finishIfReady();
      }
    }

    boolean attachChild(HaraWorkRun child) {
      if (snapshot.get().cancellation != null || snapshot.get().terminal() || bodyDone.get()) {
        return false;
      }
      children.put(child.runId, child);
      if (snapshot.get().cancellation != null || snapshot.get().terminal()) {
        children.remove(child.runId, child);
        return false;
      }
      return true;
    }

    void childClosed(HaraWorkRun child) {
      children.remove(child.runId, child);
      finishIfReady();
    }

    boolean registerFinalizer(Object function) {
      if (finalizersStarted.get() || snapshot.get().terminal()) return false;
      finalizers.push(function);
      if (finalizersStarted.get() || snapshot.get().terminal()) {
        finalizers.removeFirstOccurrence(function);
        return false;
      }
      return true;
    }

    boolean requestCancellation(Object reason) {
      HaraException failure =
          new HaraException("Work run cancelled: " + String.valueOf(reason));
      CancellationRequest request = new CancellationRequest(reason, failure);
      Keyword previousState = transitionToCancelling(request);
      if (previousState == null) return false;
      publishEvent(
          RUN_CANCELLING,
          Map.Standard.from(null, STATE, CANCELLING, CANCEL_REASON, reason),
          false);

      cancelDeadlineTask();
      IPromise promise = activePromise.get();
      if (promise != null) promise.cancel();
      for (HaraWorkRun child : children.values()) {
        child.requestCancellation(reason);
      }
      if (CREATED.equals(previousState) || QUEUED.equals(previousState)) {
        bodyDone.set(true);
      }
      finishIfReady();
      return true;
    }

    private Keyword transitionToCancelling(CancellationRequest request) {
      while (true) {
        RunSnapshot current = snapshot.get();
        if (current.terminal() || current.cancellation != null) return null;
        RunSnapshot update =
            new RunSnapshot(
                CANCELLING,
                current.startedAt,
                current.finishedAt,
                current.error,
                request);
        if (snapshot.compareAndSet(current, update)) return current.state;
      }
    }

    private void finishIfReady() {
      if (!bodyDone.get()) return;
      CancellationRequest request = snapshot.get().cancellation;
      if (!children.isEmpty()) {
        transitionAnyNonTerminal(request == null ? WAITING : CANCELLING, request);
        return;
      }
      if (!finalizersStarted.compareAndSet(false, true)) return;

      Throwable finalizerError = runFinalizers();
      request = snapshot.get().cancellation;
      if (request != null) {
        settleTerminal(CANCELLED, request.failure, null);
        return;
      }
      BodyOutcome outcome = bodyOutcome.get();
      if (finalizerError != null) {
        settleTerminal(FAILED, finalizerError, null);
      } else if (outcome == null) {
        settleTerminal(FAILED, new HaraException("Work body produced no outcome"), null);
      } else if (outcome.error != null) {
        settleTerminal(FAILED, outcome.error, null);
      } else {
        settleTerminal(COMPLETED, null, outcome.value);
      }
    }

    private Throwable runFinalizers() {
      Throwable first = null;
      WorkContext previous = CURRENT_CONTEXT.get();
      CURRENT_CONTEXT.set(workContext);
      try {
        Object function;
        while ((function = finalizers.poll()) != null) {
          try {
            Object finalizer = function;
            context.invokeInContext(
                () -> context.invokeCallable(finalizer, new Object[] {workContext}));
          } catch (Throwable error) {
            if (first == null) first = unwrap(error);
          }
        }
      } finally {
        if (previous == null) CURRENT_CONTEXT.remove();
        else CURRENT_CONTEXT.set(previous);
      }
      return first;
    }

    private void settleTerminal(Keyword state, Throwable error, Object value) {
      while (true) {
        RunSnapshot current = snapshot.get();
        if (current.terminal()) return;
        RunSnapshot update =
            new RunSnapshot(
                state,
                current.startedAt,
                System.currentTimeMillis(),
                error,
                current.cancellation);
        if (!snapshot.compareAndSet(current, update)) continue;
        publishEvent(
            eventType(state),
            Map.Standard.from(
                null,
                STATE,
                state,
                ERROR,
                error,
                CANCEL_REASON,
                current.cancellation == null ? null : current.cancellation.reason),
            true);
        cancelDeadlineTask();
        if (COMPLETED.equals(state)) {
          resultFuture.complete(value);
        } else {
          resultFuture.completeExceptionally(error);
        }
        notifyParent();
        return;
      }
    }

    private void notifyParent() {
      if (parent != null && parentNotified.compareAndSet(false, true)) {
        parent.childClosed(this);
      }
    }

    private void cancelDeadlineTask() {
      ScheduledFuture<?> scheduled = deadlineTask;
      if (scheduled != null) scheduled.cancel(false);
    }

    private boolean transition(
        Keyword expected, Keyword next, Throwable error, CancellationRequest cancellation) {
      while (true) {
        RunSnapshot current = snapshot.get();
        if (!expected.equals(current.state) || current.terminal()) return false;
        RunSnapshot update =
            new RunSnapshot(next, current.startedAt, 0L, error, cancellation);
        if (snapshot.compareAndSet(current, update)) return true;
      }
    }

    private void transitionAnyNonTerminal(Keyword state, CancellationRequest request) {
      while (true) {
        RunSnapshot current = snapshot.get();
        if (current.terminal() || state.equals(current.state)) return;
        RunSnapshot update =
            new RunSnapshot(
                state,
                current.startedAt,
                current.finishedAt,
                current.error,
                request == null ? current.cancellation : request);
        if (snapshot.compareAndSet(current, update)) {
          if (WAITING.equals(state)) {
            publishEvent(RUN_WAITING, Map.Standard.from(null, STATE, WAITING), false);
          }
          return;
        }
      }
    }

    @Override
    public Object workId() {
      return runId;
    }

    @Override
    public Object workStatus() {
      if (deadlineNanos > 0L && System.nanoTime() >= deadlineNanos) {
        requestCancellation(DEADLINE_EXCEEDED);
      }
      return snapshot.get().state;
    }

    private Object statusSnapshot() {
      if (deadlineNanos > 0L && System.nanoTime() >= deadlineNanos) {
        requestCancellation(DEADLINE_EXCEEDED);
      }
      RunSnapshot current = snapshot.get();
      List<Object> entries = new ArrayList<>();
      entries.add(STATE);
      entries.add(current.state);
      entries.add(STATUS_WORK_ID);
      entries.add(runId);
      entries.add(CHILD_COUNT);
      entries.add((long) children.size());
      if (parent != null) {
        entries.add(PARENT_ID);
        entries.add(parent.runId);
      }
      if (detached) {
        entries.add(DETACHED);
        entries.add(true);
      }
      if (deadlineNanos > 0L) {
        entries.add(DEADLINE_NANOS);
        entries.add(deadlineNanos);
      }
      if (current.startedAt != 0L) {
        entries.add(STARTED_AT);
        entries.add(current.startedAt);
      }
      if (current.finishedAt != 0L) {
        entries.add(FINISHED_AT);
        entries.add(current.finishedAt);
      }
      if (current.error != null) {
        entries.add(ERROR);
        entries.add(current.error);
      }
      if (current.cancellation != null) {
        entries.add(CANCEL_REASON);
        entries.add(current.cancellation.reason);
      }
      return Map.Standard.from(null, entries.toArray());
    }

    @Override
    public IPromise workResult() {
      return result;
    }

    @Override
    public IStream workEvents(Object options) {
      long after = 0L;
      Object value = option(options, Keyword.create("after"));
      if (value != null) {
        if (!(value instanceof Number number) || number.longValue() < 0L) {
          throw new HaraException(":after must be a non-negative integer");
        }
        after = number.longValue();
      }
      WorkEventStream stream = new WorkEventStream(after);
      synchronized (eventLock) {
        eventStreams.add(stream);
      }
      return stream;
    }

    @Override
    public IPromise workCancel(Object reason) {
      return (IPromise)
          context.promiseValue(
              CompletableFuture.completedFuture(requestCancellation(reason)));
    }

    @Override
    public boolean closed() {
      return snapshot.get().terminal();
    }

    private Keyword eventType(Keyword state) {
      if (COMPLETED.equals(state)) return RUN_COMPLETED;
      if (FAILED.equals(state)) return RUN_FAILED;
      if (CANCELLED.equals(state)) return RUN_CANCELLED;
      throw new HaraException("No terminal work event for state: " + state);
    }

    private void publishEvent(Keyword type, Object data, boolean terminal) {
      List<Runnable> completions = new ArrayList<>();
      synchronized (eventLock) {
        long sequence = ++eventSequence;
        events.add(
            Map.Standard.from(
                null,
                EVENT_TYPE,
                type,
                EVENT_RUN,
                runId,
                EVENT_SEQUENCE,
                sequence,
                EVENT_DATA,
                data));
        if (terminal) eventsClosed = true;
        for (WorkEventStream stream : eventStreams) {
          Runnable completion = stream.takeCompletion();
          if (completion != null) completions.add(completion);
        }
      }
      for (Runnable completion : completions) completion.run();
    }

    private final class WorkEventStream implements IStream {
      private long after;
      private boolean closed;
      private CompletableFuture<Object> pending;

      WorkEventStream(long after) {
        this.after = after;
      }

      @Override
      public Object next() {
        CompletableFuture<Object> future;
        Runnable completion;
        synchronized (eventLock) {
          if (closed) return context.completedPromise(null);
          if (pending != null) {
            return context.rejectedPromise(
                "stream/pending-pull: only one Stream/next may be pending");
          }
          future = new CompletableFuture<>();
          pending = future;
          completion = takeCompletion();
        }
        if (completion != null) completion.run();
        return context.promiseValue(future);
      }

      Runnable takeCompletion() {
        if (pending == null) return null;
        if (after < events.size()) {
          Object event = events.get((int) after);
          after += 1L;
          CompletableFuture<Object> future = pending;
          pending = null;
          return () -> future.complete(event);
        }
        if (eventsClosed || closed) {
          closed = true;
          CompletableFuture<Object> future = pending;
          pending = null;
          return () -> future.complete(null);
        }
        return null;
      }

      @Override
      public void close() {
        Runnable completion;
        synchronized (eventLock) {
          if (closed) return;
          closed = true;
          completion = takeCompletion();
          eventStreams.remove(this);
        }
        if (completion != null) completion.run();
      }
    }
  }

  private record CancellationRequest(Object reason, HaraException failure) {}

  private record BodyOutcome(Object value, Throwable error) {
    static BodyOutcome success(Object value) {
      return new BodyOutcome(value, null);
    }

    static BodyOutcome failure(Throwable error) {
      return new BodyOutcome(null, error);
    }
  }

  private record RunSnapshot(
      Keyword state,
      long startedAt,
      long finishedAt,
      Throwable error,
      CancellationRequest cancellation) {
    boolean terminal() {
      return COMPLETED.equals(state) || FAILED.equals(state) || CANCELLED.equals(state);
    }
  }

  private static Throwable unwrap(Throwable error) {
    Throwable current = error;
    while ((current instanceof CompletionException) && current.getCause() != null) {
      current = current.getCause();
    }
    return current;
  }
}
