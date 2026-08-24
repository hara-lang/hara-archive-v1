package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertThrows;
import static org.junit.Assert.assertTrue;

import hara.lang.data.Keyword;
import hara.lang.protocol.IWorkRun;
import java.util.UUID;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.TimeUnit;
import org.graalvm.polyglot.Context;
import org.graalvm.polyglot.PolyglotException;
import org.junit.Test;

public class HaraWorkHostTest {
  @Test
  public void cancellationPreventsDelayedNativeEffectsFromRunning() throws Exception {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      enterNamespace(context, "work.host.cancel.delay");
      context.eval(
          HaraLanguage.ID,
          "(do "
              + "(def delay-fired (atom false)) "
              + "(def delay-run "
              + "  (IWorkHost/work-submit (Work/default-host) :timer nil "
              + "    {:work/execute (fn [work input options id] "
              + "      (promise/then "
              + "       (promise/delay 150 (fn [] (reset! delay-fired true))) identity))})) nil)");
      awaitValue(context, "(IWorkRun/work-status delay-run)", ":waiting");

      context.eval(HaraLanguage.ID, "(deref (IWorkRun/work-cancel delay-run :test/timer))");
      Thread.sleep(250L);

      assertEquals("false", context.eval(HaraLanguage.ID, "(deref delay-fired)").toString());
      assertEquals(
          ":cancelled",
          context.eval(HaraLanguage.ID, "(IWorkRun/work-status delay-run)").toString());
    }
  }

  @Test
  public void cancellationTerminatesAnAwaitedNativeProcess() throws Exception {
    try (Context context =
        Context.newBuilder(HaraLanguage.ID).allowCreateProcess(true).build()) {
      enterNamespace(context, "work.host.cancel.process");
      context.eval(
          HaraLanguage.ID,
          "(do "
              + "(def active-process (atom nil)) "
              + "(def process-run "
              + "  (IWorkHost/work-submit (Work/default-host) :process nil "
              + "    {:work/execute (fn [work input options id] "
              + "      (let [process (Process/spawn [\"/bin/sh\" \"-c\" \"sleep 30\"])] "
              + "        (reset! active-process process) "
              + "        (promise/then (Process/wait process) identity)))})) nil)");
      awaitValue(context, "(IWorkRun/work-status process-run)", ":waiting");
      assertEquals(
          "true",
          context.eval(HaraLanguage.ID, "(Process/alive? (deref active-process))").toString());

      context.eval(HaraLanguage.ID, "(deref (IWorkRun/work-cancel process-run :test/process))");
      awaitValue(context, "(Process/alive? (deref active-process))", "false");

      assertEquals(
          ":cancelled",
          context.eval(HaraLanguage.ID, "(IWorkRun/work-status process-run)").toString());
    }
  }

  @Test
  public void scopeHelpersAreOrdinaryWorkNativeFunctions() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      enterNamespace(context, "work.host.scope.functions");
      assertEquals(
          "[\"jvm-scope-functions\" false true 42]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(let [run (IWorkHost/work-submit "
                      + "(Work/default-host) :payload 42 "
                      + "{:id \"jvm-scope-functions\" "
                      + ":work/execute (fn [work input options id] "
                      + "[(IWorkRef/work-id (Work/current-run)) "
                      + " (Work/cancelled?) "
                      + " (Work/on-close (fn [context] nil)) input])})] "
                      + "(deref (IWorkRun/work-result run)))")
              .toString());
    }
  }

  @Test
  public void stopDrainsWhileKillCancelsAdmittedRuns() {
    try (Context polyglot = Context.newBuilder(HaraLanguage.ID).build()) {
      polyglot.eval(HaraLanguage.ID, "nil");
      polyglot.enter();
      try {
        HaraContext context = HaraLanguage.currentContext();
        HaraWorkHost host = HaraWorkHost.instance();
        host.start();
        CompletableFuture<Object> body = new CompletableFuture<>();
        Object executor =
            context.libraryFunction("work.host/drain", arguments -> context.promiseValue(body));
        IWorkRun draining =
            host.workSubmit(
                context,
                Keyword.create("payload"),
                null,
                java.util.Map.of(
                    Keyword.create("id"), "stop-drain-" + UUID.randomUUID(),
                    Keyword.create("work", "execute"), executor));
        host.stop();
        assertThrows(
            RuntimeException.class,
            () ->
                host.workSubmit(
                    context,
                    Keyword.create("payload"),
                    null,
                    java.util.Map.of(Keyword.create("work", "execute"), executor)));
        body.complete(7L);
        assertEquals(7L, draining.workResult().deref());

        host.start();
        CompletableFuture<Object> never = new CompletableFuture<>();
        Object blocking =
            context.libraryFunction("work.host/kill", arguments -> context.promiseValue(never));
        IWorkRun cancelled =
            host.workSubmit(
                context,
                Keyword.create("payload"),
                null,
                java.util.Map.of(
                    Keyword.create("id"), "kill-cancel-" + UUID.randomUUID(),
                    Keyword.create("work", "execute"), blocking));
        host.kill();
        assertThrows(RuntimeException.class, () -> cancelled.workResult().deref());
        host.start();
      } finally {
        polyglot.leave();
      }
    }
  }

  @Test
  public void lifecycleEventsAreOrderedReplayableStreams() {
    String id = "events-" + UUID.randomUUID();
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      enterNamespace(context, "work.host.events");
      assertEquals(
          "[[:work/run-queued 1] [:work/run-running 2] [:work/run-completed 3] nil]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(let [run (IWorkHost/work-submit (Work/default-host) "
                      + "              :payload 42 {:id \""
                      + id
                      + "\" :work/execute (fn [work input options id] input)}) "
                      + "      events (IWorkRun/work-events run {}) "
                      + "      _ (deref (IWorkRun/work-result run)) "
                      + "      first (deref (IStream/next events)) "
                      + "      second (deref (IStream/next events)) "
                      + "      third (deref (IStream/next events))] "
                      + "  [[(:event/type first) (:event/sequence first)] "
                      + "   [(:event/type second) (:event/sequence second)] "
                      + "   [(:event/type third) (:event/sequence third)] "
                      + "   (deref (IStream/next events))])")
              .toString());
    }
  }

  @Test
  public void submissionReturnsBeforeTheNativeResultSettles() {
    String id = "immediate-" + UUID.randomUUID();
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      enterNamespace(context, "work.host.immediate");
      String state =
          context
              .eval(
                  HaraLanguage.ID,
                  "(do "
                      + "(def live-run "
                      + "  (IWorkHost/work-submit (Work/default-host) "
                      + "    :payload 7 "
                      + "    {:id \""
                      + id
                      + "\" "
                      + "     :work/execute "
                      + "     (fn [work input options run-id] "
                      + "       (promise/delay 1000 (fn [] [work input run-id])))})) "
                      + "(IWorkRun/work-status live-run))")
              .toString();

      assertTrue(state.equals(":queued") || state.equals(":running"));
      assertEquals(
          "[:payload 7 \"" + id + "\"]",
          context
              .eval(HaraLanguage.ID, "(deref (IWorkRun/work-result live-run))")
              .toString());
      assertEquals(
          ":completed",
          context
              .eval(HaraLanguage.ID, "(IWorkRun/work-status live-run)")
              .toString());
    }
  }

  @Test
  public void resolvesTheSameCompletedRunFromAnIndependentSession() {
    String id = "cross-session-" + UUID.randomUUID();
    try (Context first = Context.newBuilder(HaraLanguage.ID).build();
        Context second = Context.newBuilder(HaraLanguage.ID).build()) {
      enterNamespace(first, "work.host.first");
      assertEquals(
          "42",
          first
              .eval(
                  HaraLanguage.ID,
                  "(do "
                      + "(def cross-run "
                      + "  (IWorkHost/work-submit (Work/default-host) "
                      + "    :payload nil "
                      + "    {:id \""
                      + id
                      + "\" "
                      + "     :work/execute (fn [work input options run-id] 42)})) "
                      + "(deref (IWorkRun/work-result cross-run)))")
              .toString());

      enterNamespace(second, "work.host.second");
      assertEquals(
          "[\"" + id + "\" 42 :completed]",
          second
              .eval(
                  HaraLanguage.ID,
                  "(let [run (IWorkHost/work-resolve (Work/default-host) \""
                      + id
                      + "\")] "
                      + "  [(IWorkRef/work-id run) "
                      + "   (deref (IWorkRun/work-result run)) "
                      + "   (IWorkRun/work-status run)])")
              .toString());
    }
  }

  @Test
  public void retainsStructuredFailureAndRejectsTheResultPromise() {
    String id = "failed-" + UUID.randomUUID();
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      enterNamespace(context, "work.host.failed");
      context.eval(
          HaraLanguage.ID,
          "(def failed-run "
              + "  (IWorkHost/work-submit (Work/default-host) "
              + "    :payload nil "
              + "    {:id \""
              + id
              + "\" "
              + "     :work/execute "
              + "     (fn [work input options run-id] "
              + "       (throw (ex-info \"work failed\" {:code :work/test})))}))");

      PolyglotException failure =
          assertThrows(
              PolyglotException.class,
              () -> context.eval(HaraLanguage.ID, "(deref (IWorkRun/work-result failed-run))"));
      assertTrue(failure.getMessage().contains("work failed"));
      assertEquals(
          ":failed",
          context.eval(HaraLanguage.ID, "(IWorkRun/work-status failed-run)").toString());
    }
  }

  @Test
  public void terminalStateAndResultCannotBeOverwrittenByLateCancellation() {
    String id = "terminal-" + UUID.randomUUID();
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      enterNamespace(context, "work.host.terminal");
      assertEquals(
          "[42 false :completed 42]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(do "
                      + "(def terminal-run "
                      + "  (IWorkHost/work-submit (Work/default-host) "
                      + "    :payload nil "
                      + "    {:id \""
                      + id
                      + "\" "
                      + "     :work/execute (fn [work input options run-id] 42)})) "
                      + "(let [value (deref (IWorkRun/work-result terminal-run)) "
                      + "      cancelled (deref (IWorkRun/work-cancel terminal-run :late))] "
                      + "  [value cancelled "
                      + "   (IWorkRun/work-status terminal-run) "
                      + "   (deref (IWorkRun/work-result terminal-run))]))")
              .toString());
    }
  }

  private static void enterNamespace(Context context, String name) {
    context.eval(
        HaraLanguage.ID,
        "(ns " + name + ")");
  }

  private static void awaitValue(Context context, String source, String expected) throws Exception {
    long deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(5L);
    String actual = null;
    while (System.nanoTime() < deadline) {
      actual = context.eval(HaraLanguage.ID, source).toString();
      if (expected.equals(actual)) return;
      Thread.sleep(5L);
    }
    assertEquals(expected, actual);
  }
}
