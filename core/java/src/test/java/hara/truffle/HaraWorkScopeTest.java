package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertNotNull;
import static org.junit.Assert.assertThrows;
import static org.junit.Assert.assertTrue;

import hara.lang.data.Keyword;
import hara.lang.data.Symbol;
import hara.lang.protocol.IMapType;
import hara.lang.protocol.IWorkRun;
import java.util.UUID;
import java.util.List;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.concurrent.atomic.AtomicReference;
import org.graalvm.polyglot.Context;
import org.junit.Test;

public class HaraWorkScopeTest {
  private static final Keyword ID = Keyword.create("id");
  private static final Keyword EXECUTE = Keyword.create("work", "execute");
  private static final Keyword TIMEOUT_MS = Keyword.create("timeout-ms");
  private static final Keyword STATE = Keyword.create("state");
  private static final Keyword CANCEL_REASON = Keyword.create("cancel-reason");
  private static final Keyword DEADLINE_NANOS = Keyword.create("deadline-nanos");
  private static final Keyword WAITING = Keyword.create("waiting");
  private static final Keyword CANCELLING = Keyword.create("cancelling");
  private static final Keyword CANCELLED = Keyword.create("cancelled");
  private static final Keyword COMPLETED = Keyword.create("completed");
  private static final Keyword DEADLINE_EXCEEDED = Keyword.create("deadline-exceeded");

  @Test
  public void domainEventsAcceptOnlyNamedEventTypes() {
    try (Context polyglot = Context.newBuilder(HaraLanguage.ID).build()) {
      HaraContext context = initialize(polyglot);
      AtomicReference<List<Boolean>> accepted = new AtomicReference<>();
      Object executor =
          context.libraryFunction(
              "work.scope/events",
              arguments -> {
                HaraWorkHost.WorkContext workContext = HaraWorkHost.currentWorkContext();
                accepted.set(
                    List.of(
                        workContext.emit(Keyword.create("task", "keyword"), 1L),
                        workContext.emit(Symbol.create("task", "symbol"), 2L),
                        workContext.emit("task/string", 3L),
                        workContext.emit(42L, 4L)));
                return Keyword.create("done");
              });

      IWorkRun run = submit(context, "events", executor, java.util.Map.of());
      assertEquals(Keyword.create("done"), run.workResult().deref());
      assertEquals(List.of(true, true, true, false), accepted.get());
    }
  }

  @Test
  public void repeatedCancellationIsCooperativeAndFinalizesExactlyOnce() throws Exception {
    try (Context polyglot = Context.newBuilder(HaraLanguage.ID).build()) {
      HaraContext context = initialize(polyglot);
      CompletableFuture<Object> body = new CompletableFuture<>();
      CompletableFuture<Void> started = new CompletableFuture<>();
      AtomicInteger cleanups = new AtomicInteger();
      AtomicInteger interrupted = new AtomicInteger();
      Object finalizer =
          context.libraryFunction(
              "work.scope/finalizer",
              arguments -> {
                cleanups.incrementAndGet();
                if (Thread.currentThread().isInterrupted()) interrupted.incrementAndGet();
                return null;
              });
      Object executor =
          context.libraryFunction(
              "work.scope/cancellable",
              arguments -> {
                HaraWorkHost.WorkContext workContext = HaraWorkHost.currentWorkContext();
                assertNotNull(workContext);
                assertTrue(workContext.onClose(finalizer));
                started.complete(null);
                return context.promiseValue(body);
              });
      IWorkRun run = submit(context, "cancel", executor, java.util.Map.of());
      started.get(2, TimeUnit.SECONDS);

      assertEquals(Boolean.TRUE, run.workCancel(Keyword.create("test")).deref());
      assertThrows(RuntimeException.class, () -> run.workResult().deref());
      assertEquals(Boolean.FALSE, run.workCancel(Keyword.create("again")).deref());
      assertEquals(1, cleanups.get());
      assertEquals(0, interrupted.get());
      assertEquals(CANCELLED, status(run, STATE));
    }
  }

  @Test
  public void parentCompletionWaitsForAttachedChildren() throws Exception {
    try (Context polyglot = Context.newBuilder(HaraLanguage.ID).build()) {
      HaraContext context = initialize(polyglot);
      CompletableFuture<Object> childBody = new CompletableFuture<>();
      CompletableFuture<Void> childStarted = new CompletableFuture<>();
      String childId = "child-" + UUID.randomUUID();
      Object childExecutor =
          context.libraryFunction(
              "work.scope/child",
              arguments -> {
                childStarted.complete(null);
                return context.promiseValue(childBody);
              });
      Object parentExecutor =
          context.libraryFunction(
              "work.scope/parent",
              arguments -> {
                HaraWorkHost.WorkContext workContext = HaraWorkHost.currentWorkContext();
                workContext.submitChild(
                    Keyword.create("child"),
                    null,
                    java.util.Map.of(ID, childId, EXECUTE, childExecutor));
                return Keyword.create("parent");
              });
      IWorkRun parent = submit(context, "parent", parentExecutor, java.util.Map.of());
      childStarted.get(2, TimeUnit.SECONDS);
      awaitState(parent, WAITING);
      assertFalse(parent.closed());

      childBody.complete(Keyword.create("child-complete"));
      assertEquals(Keyword.create("parent"), parent.workResult().deref());
      assertEquals(COMPLETED, status(parent, STATE));
      IWorkRun child = HaraWorkHost.instance().workResolve(childId);
      assertEquals(COMPLETED, status(child, STATE));
    }
  }

  @Test
  public void parentCancellationPropagatesToAttachedChildren() throws Exception {
    try (Context polyglot = Context.newBuilder(HaraLanguage.ID).build()) {
      HaraContext context = initialize(polyglot);
      CompletableFuture<Object> childBody = new CompletableFuture<>();
      CompletableFuture<Void> childStarted = new CompletableFuture<>();
      String childId = "cancel-child-" + UUID.randomUUID();
      Object childExecutor =
          context.libraryFunction(
              "work.scope/cancel-child",
              arguments -> {
                childStarted.complete(null);
                return context.promiseValue(childBody);
              });
      Object parentExecutor =
          context.libraryFunction(
              "work.scope/cancel-parent",
              arguments -> {
                HaraWorkHost.currentWorkContext()
                    .submitChild(
                        Keyword.create("child"),
                        null,
                        java.util.Map.of(ID, childId, EXECUTE, childExecutor));
                return Keyword.create("parent");
              });
      IWorkRun parent = submit(context, "cancel-parent", parentExecutor, java.util.Map.of());
      childStarted.get(2, TimeUnit.SECONDS);
      awaitState(parent, WAITING);

      assertEquals(Boolean.TRUE, parent.workCancel(Keyword.create("parent-stop")).deref());
      assertThrows(RuntimeException.class, () -> parent.workResult().deref());
      IWorkRun child = HaraWorkHost.instance().workResolve(childId);
      assertThrows(RuntimeException.class, () -> child.workResult().deref());
      assertEquals(CANCELLED, status(parent, STATE));
      assertEquals(CANCELLED, status(child, STATE));
    }
  }

  @Test
  public void deadlinesCancelAtPromiseBoundariesAndAreInheritedByChildren() throws Exception {
    try (Context polyglot = Context.newBuilder(HaraLanguage.ID).build()) {
      HaraContext context = initialize(polyglot);
      CompletableFuture<Object> childBody = new CompletableFuture<>();
      CompletableFuture<Void> childStarted = new CompletableFuture<>();
      String childId = "deadline-child-" + UUID.randomUUID();
      Object childExecutor =
          context.libraryFunction(
              "work.scope/deadline-child",
              arguments -> {
                childStarted.complete(null);
                return context.promiseValue(childBody);
              });
      Object parentExecutor =
          context.libraryFunction(
              "work.scope/deadline-parent",
              arguments -> {
                HaraWorkHost.currentWorkContext()
                    .submitChild(
                        Keyword.create("child"),
                        null,
                        java.util.Map.of(
                            ID, childId,
                            EXECUTE, childExecutor,
                            TIMEOUT_MS, 5_000L));
                return context.promiseValue(new CompletableFuture<>());
              });
      IWorkRun parent =
          submit(context, "deadline-parent", parentExecutor, java.util.Map.of(TIMEOUT_MS, 40L));
      childStarted.get(2, TimeUnit.SECONDS);
      IWorkRun child = HaraWorkHost.instance().workResolve(childId);
      assertEquals(status(parent, DEADLINE_NANOS), status(child, DEADLINE_NANOS));

      assertThrows(RuntimeException.class, () -> parent.workResult().deref());
      assertEquals(CANCELLED, status(parent, STATE));
      assertEquals(DEADLINE_EXCEEDED, status(parent, CANCEL_REASON));
      assertEquals(CANCELLED, status(child, STATE));
    }
  }

  private static HaraContext initialize(Context polyglot) {
    polyglot.eval(HaraLanguage.ID, "nil");
    polyglot.enter();
    try {
      return HaraLanguage.currentContext();
    } finally {
      polyglot.leave();
    }
  }

  private static IWorkRun submit(
      HaraContext context, String prefix, Object executor, java.util.Map<Object, Object> extra) {
    java.util.Map<Object, Object> options = new java.util.HashMap<>(extra);
    options.put(ID, prefix + "-" + UUID.randomUUID());
    options.put(EXECUTE, executor);
    return HaraWorkHost.instance().workSubmit(context, Keyword.create("work"), null, options);
  }

  private static Object status(IWorkRun run, Keyword key) {
    return ((IMapType) HaraWorkHost.workStatusSnapshot(run)).lookup(key);
  }

  private static void awaitState(IWorkRun run, Keyword expected) throws Exception {
    long deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(2);
    while (System.nanoTime() < deadline) {
      Object state = status(run, STATE);
      if (expected.equals(state)) return;
      if (CANCELLING.equals(state) || CANCELLED.equals(state)) {
        throw new AssertionError("run closed before reaching " + expected + ": " + state);
      }
      Thread.sleep(2L);
    }
    throw new AssertionError("run did not reach state " + expected + ": " + status(run, STATE));
  }
}
