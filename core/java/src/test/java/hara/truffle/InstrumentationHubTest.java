package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertThrows;
import static org.junit.Assert.assertTrue;

import hara.truffle.InstrumentationException.Code;
import hara.truffle.InstrumentationModel.Capability;
import hara.truffle.InstrumentationModel.ControlLease;
import hara.truffle.InstrumentationModel.EventBatch;
import hara.truffle.InstrumentationModel.EventDelivery;
import hara.truffle.InstrumentationModel.EventKind;
import hara.truffle.InstrumentationModel.EventPhase;
import hara.truffle.InstrumentationModel.InstrumentDirective;
import hara.truffle.InstrumentationModel.InstrumentFilter;
import hara.truffle.InstrumentationModel.InstrumentHandle;
import hara.truffle.InstrumentationModel.InstrumentMode;
import hara.truffle.InstrumentationModel.InstrumentRegistration;
import hara.truffle.InstrumentationModel.ProjectionLimits;
import hara.truffle.InstrumentationModel.ProjectionRequest;
import hara.truffle.InstrumentationModel.RuntimeBackend;
import hara.truffle.InstrumentationModel.TargetDescriptor;
import hara.truffle.InstrumentationModel.TargetHandle;
import hara.truffle.InstrumentationModel.TargetKind;
import java.util.List;
import java.util.Map;
import java.util.Set;
import org.junit.Test;

public class InstrumentationHubTest {
  @Test
  public void passiveFanoutFollowsRegistrationOrderAndSharesTargetSequence() {
    try (InstrumentationHub hub = new InstrumentationHub()) {
      TargetHandle target =
          hub.registerTarget(interpreterTarget("execution", "session"));
      InstrumentHandle first =
          hub.registerInstrument(
              passive(
                  "first",
                  "session",
                  8,
                  Set.of(Capability.EVENT_LIFECYCLE)));
      InstrumentHandle second =
          hub.registerInstrument(
              passive(
                  "second",
                  "session",
                  8,
                  Set.of(Capability.EVENT_LIFECYCLE)));
      hub.attach(first, target);
      hub.attach(second, target);

      assertEquals(
          2,
          hub.publish(
              target,
              EventKind.EXECUTION_TERMINAL,
              EventPhase.LIVE,
              null,
              Map.of("status", "returned")));
      assertEquals(
          List.of("first", "second"),
          hub.registrations().stream()
              .map(InstrumentRegistration::instrumentId)
              .toList());
      EventBatch firstBatch = hub.drain(first);
      EventBatch secondBatch = hub.drain(second);
      assertEquals(1, firstBatch.events().size());
      assertEquals(1, secondBatch.events().size());
      assertEquals(1L, firstBatch.events().get(0).sequence());
      assertEquals(1L, secondBatch.events().get(0).sequence());
      assertEquals("first", firstBatch.events().get(0).instrumentId());
      assertEquals("second", secondBatch.events().get(0).instrumentId());
    }
  }

  @Test
  public void boundedQueuesReportDeterministicOverflowEvidence() {
    try (InstrumentationHub hub = new InstrumentationHub()) {
      TargetHandle target =
          hub.registerTarget(interpreterTarget("execution", "session"));
      InstrumentHandle trace =
          hub.registerInstrument(
              passive(
                  "trace",
                  "session",
                  2,
                  Set.of(Capability.EVENT_LIFECYCLE)));
      hub.attach(trace, target);
      for (int index = 0; index < 4; index++) {
        hub.publish(
            target,
            EventKind.EXECUTION_TERMINAL,
            EventPhase.LIVE,
            null,
            Map.of("index", Integer.toString(index)));
      }

      EventBatch batch = hub.drain(trace);
      assertEquals(
          List.of(3L, 4L),
          batch.events().stream().map(event -> event.sequence()).toList());
      assertEquals(2L, batch.droppedSinceDrain());
      assertEquals(2L, batch.droppedTotal());
      EventBatch empty = hub.drain(trace);
      assertTrue(empty.events().isEmpty());
      assertEquals(0L, empty.droppedSinceDrain());
      assertEquals(2L, empty.droppedTotal());
    }
  }

  @Test
  public void unsupportedCapabilitiesIncludeExactProviderEvidence() {
    try (InstrumentationHub hub = new InstrumentationHub()) {
      TargetHandle target =
          hub.registerTarget(interpreterTarget("execution", "session"));
      InstrumentHandle locals =
          hub.registerInstrument(
              new InstrumentRegistration(
                  "locals",
                  "session",
                  InstrumentMode.PASSIVE,
                  Set.of(
                      Capability.EVENT_LIFECYCLE,
                      Capability.INSPECT_LOCALS),
                  Set.of(EventKind.EXECUTION_TERMINAL),
                  InstrumentFilter.all(),
                  new ProjectionRequest(
                      false,
                      null,
                      null,
                      ProjectionLimits.defaults(),
                      null,
                      null,
                      null),
                  EventDelivery.queue(8)));

      InstrumentationException error =
          assertThrows(
              InstrumentationException.class,
              () -> hub.attach(locals, target));
      assertEquals(Code.UNSUPPORTED_CAPABILITIES, error.code());
      assertEquals("execution", error.evidence().get("target"));
      assertEquals(
          new RuntimeBackend("java"), error.evidence().get("backend"));
      assertEquals(
          Set.of(Capability.INSPECT_LOCALS), error.evidence().get("missing"));
      assertEquals(
          Set.of(Capability.EVENT_LIFECYCLE),
          error.evidence().get("potential"));
    }
  }

  @Test
  public void reusedIdsAdvanceGenerationAndRejectStaleHandles() {
    try (InstrumentationHub hub = new InstrumentationHub()) {
      TargetHandle original =
          hub.registerTarget(interpreterTarget("execution", "session"));
      hub.removeTarget(original);
      TargetHandle replacement =
          hub.registerTarget(interpreterTarget("execution", "session"));
      assertEquals(0L, original.generation());
      assertEquals(1L, replacement.generation());
      InstrumentationException stale =
          assertThrows(
              InstrumentationException.class,
              () ->
                  hub.bindTarget(
                      original.targetId(), original.generation()));
      assertEquals(Code.STALE_TARGET, stale.code());
      assertEquals(
          replacement,
          hub.bindTarget(
              replacement.targetId(), replacement.generation()));
    }
  }

  @Test
  public void oneControllerOwnsTheTargetLeaseAndIssuesFencedDirectives() {
    try (InstrumentationHub hub = new InstrumentationHub()) {
      TargetHandle target =
          hub.registerTarget(
              new TargetDescriptor(
                  "execution",
                  "session",
                  TargetKind.INTERPRETER,
                  new RuntimeBackend("java"),
                  Set.of(
                      Capability.EVENT_LIFECYCLE,
                      Capability.CONTROL_PAUSE,
                      Capability.CONTROL_SINGLE_STEP,
                      Capability.CONTROL_RESUME,
                      Capability.CONTROL_TERMINATE)));
      Set<Capability> controls =
          Set.of(
              Capability.EVENT_LIFECYCLE,
              Capability.CONTROL_PAUSE,
              Capability.CONTROL_SINGLE_STEP,
              Capability.CONTROL_RESUME,
              Capability.CONTROL_TERMINATE);
      InstrumentHandle first =
          hub.registerInstrument(control("first", controls));
      InstrumentHandle second =
          hub.registerInstrument(control("second", controls));
      hub.attach(first, target);
      hub.attach(second, target);

      ControlLease lease = hub.acquireControlLease(first, target);
      InstrumentationException conflict =
          assertThrows(
              InstrumentationException.class,
              () -> hub.acquireControlLease(second, target));
      assertEquals(Code.CONTROL_LEASE_CONFLICT, conflict.code());
      assertEquals("first", conflict.evidence().get("holder"));
      hub.issueDirective(lease, InstrumentDirective.SUSPEND);
      hub.issueDirective(lease, InstrumentDirective.STEP_NEXT);
      hub.issueDirective(lease, InstrumentDirective.CONTINUE);
      hub.issueDirective(lease, InstrumentDirective.TERMINATE);
      assertEquals(InstrumentDirective.SUSPEND, hub.pollDirective(target));
      assertEquals(
          InstrumentDirective.STEP_NEXT, hub.pollDirective(target));
      assertEquals(InstrumentDirective.CONTINUE, hub.pollDirective(target));
      assertEquals(InstrumentDirective.TERMINATE, hub.pollDirective(target));
      hub.releaseControlLease(lease);
      ControlLease replacement = hub.acquireControlLease(second, target);
      assertEquals(1L, replacement.generation());
    }
  }

  @Test
  public void cleanupRemovesOnlyOneSessionAndInvalidatesItsHandles() {
    try (InstrumentationHub hub = new InstrumentationHub()) {
      TargetHandle alphaTarget =
          hub.registerTarget(interpreterTarget("alpha-target", "alpha"));
      TargetHandle betaTarget =
          hub.registerTarget(interpreterTarget("beta-target", "beta"));
      InstrumentHandle alpha =
          hub.registerInstrument(
              passive(
                  "alpha-trace",
                  "alpha",
                  8,
                  Set.of(Capability.EVENT_LIFECYCLE)));
      InstrumentHandle beta =
          hub.registerInstrument(
              passive(
                  "beta-trace",
                  "beta",
                  8,
                  Set.of(Capability.EVENT_LIFECYCLE)));
      hub.attach(alpha, alphaTarget);
      hub.attach(beta, betaTarget);

      hub.cleanupSession("alpha");
      assertEquals(1, hub.instrumentCount());
      assertEquals(1, hub.targetCount());
      assertEquals(1, hub.attachmentCount());
      assertEquals("beta-trace", hub.registrations().get(0).instrumentId());
      assertThrows(InstrumentationException.class, () -> hub.drain(alpha));
      assertEquals(betaTarget, hub.bindTarget("beta-target", 0));
    }
  }

  @Test
  public void closedHubInvalidatesEveryOperation() {
    InstrumentationHub hub = new InstrumentationHub();
    hub.close();
    InstrumentationException error =
        assertThrows(InstrumentationException.class, hub::instrumentCount);
    assertEquals(Code.RUNTIME_CLOSED, error.code());
  }

  @Test
  public void nativeHbcInstrumentationOnlyRetainsPassiveInstructionAndTerminalEvents() {
    try (InstrumentationHub hub = new InstrumentationHub()) {
      TargetHandle target =
          hub.registerTarget(
              new TargetDescriptor(
                  "hbc",
                  "session",
                  TargetKind.HBC,
                  new RuntimeBackend("java-hbc"),
                  Set.of(
                      Capability.EVENT_INSTRUCTION,
                      Capability.EVENT_CALL,
                      Capability.EVENT_LIFECYCLE)));
      InstrumentHandle nativeTrace =
          hub.registerInstrument(
              new InstrumentRegistration(
                  "native-trace",
                  "session",
                  InstrumentMode.PASSIVE,
                  Set.of(Capability.EVENT_INSTRUCTION, Capability.EVENT_LIFECYCLE),
                  Set.of(EventKind.INSTRUCTION_EXECUTE, EventKind.EXECUTION_TERMINAL),
                  InstrumentFilter.all(),
                  ProjectionRequest.none(),
                  EventDelivery.queue(8)));
      hub.attach(nativeTrace, target);
      assertTrue(hub.hbcNativeExecutionAllowed(target));

      InstrumentHandle callTrace =
          hub.registerInstrument(
              new InstrumentRegistration(
                  "call-trace",
                  "session",
                  InstrumentMode.PASSIVE,
                  Set.of(Capability.EVENT_CALL),
                  Set.of(EventKind.CALL_ENTER),
                  InstrumentFilter.all(),
                  ProjectionRequest.none(),
                  EventDelivery.queue(8)));
      hub.attach(callTrace, target);
      assertTrue(!hub.hbcNativeExecutionAllowed(target));
    }
  }

  private static TargetDescriptor interpreterTarget(
      String id, String session) {
    return new TargetDescriptor(
        id,
        session,
        TargetKind.INTERPRETER,
        new RuntimeBackend("java"),
        Set.of(Capability.EVENT_LIFECYCLE));
  }

  private static InstrumentRegistration passive(
      String id,
      String session,
      int capacity,
      Set<Capability> capabilities) {
    return new InstrumentRegistration(
        id,
        session,
        InstrumentMode.PASSIVE,
        capabilities,
        Set.of(EventKind.EXECUTION_TERMINAL),
        new InstrumentFilter(session, Set.of(), Set.of(), Set.of()),
        ProjectionRequest.none(),
        EventDelivery.queue(capacity));
  }

  private static InstrumentRegistration control(
      String id, Set<Capability> capabilities) {
    return new InstrumentRegistration(
        id,
        "session",
        InstrumentMode.CONTROL,
        capabilities,
        Set.of(EventKind.EXECUTION_TERMINAL),
        new InstrumentFilter("session", Set.of(), Set.of(), Set.of()),
        ProjectionRequest.none(),
        EventDelivery.queue(8));
  }
}
