package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertThrows;
import static org.junit.Assert.assertTrue;

import hara.truffle.InstrumentationException.Code;
import hara.truffle.InstrumentationModel.Capability;
import hara.truffle.InstrumentationModel.EventDelivery;
import hara.truffle.InstrumentationModel.DeliveredEvent;
import hara.truffle.InstrumentationModel.EventKind;
import hara.truffle.InstrumentationModel.EventPhase;
import hara.truffle.InstrumentationModel.InstrumentFilter;
import hara.truffle.InstrumentationModel.InstrumentMode;
import hara.truffle.InstrumentationModel.InstrumentRegistration;
import hara.truffle.InstrumentationModel.ProjectionRequest;
import hara.truffle.InstrumentationModel.RuntimeBackend;
import hara.truffle.InstrumentationModel.TargetDescriptor;
import hara.truffle.InstrumentationModel.TargetHandle;
import hara.truffle.InstrumentationModel.TargetKind;
import hara.truffle.NativeInstrumentation.NativeInstrumentHandle;
import hara.truffle.NativeInstrumentation.NativeControlLease;
import hara.truffle.NativeInstrumentation.NativeTargetHandle;
import hara.truffle.bytecode.HbcProgram;
import hara.truffle.bytecode.HbcProgram.Function;
import hara.truffle.bytecode.HbcProgram.Instruction;
import java.util.Arrays;
import java.util.List;
import java.util.Map;
import java.util.Set;
import org.junit.Test;

public class SessionInstrumentationTest {
  @Test
  public void sessionOwnsScopedHostServiceAndDirectStopCleansEverything() {
    SessionModel.SessionId sessionId = SessionModel.SessionId.parse("instrumented");
    try (SessionKernel kernel = new SessionKernel(false, false)) {
      SessionKernel.Session session = kernel.create(sessionId);
      InstrumentationHub hub = kernel.instrumentationHub();
      TargetHandle target =
          hub.registerTarget(interpreterTarget("execution", sessionId.value()));
      NativeInstrumentation service = kernel.instrumentation(sessionId);
      NativeTargetHandle nativeTarget =
          service.bindTargetIdentity(target.targetId(), target.generation());
      NativeInstrumentHandle trace =
          service.register(passive("trace", sessionId.value()));
      service.attach(trace, nativeTarget);

      assertEquals(
          1,
          hub.publish(
              target,
              EventKind.EXECUTION_TERMINAL,
              EventPhase.LIVE,
              null,
              Map.of("status", "returned")));
      assertEquals(1, service.drainEvents(trace).events().size());
      session.stop();
      assertEquals(0, hub.instrumentCount());
      assertEquals(0, hub.targetCount());
      assertEquals(0, hub.attachmentCount());
      InstrumentationException closed =
          assertThrows(
              InstrumentationException.class,
              () -> service.drainEvents(trace));
      assertEquals(Code.SESSION_CLOSED, closed.code());
    }
  }

  @Test
  public void sessionIdReuseDoesNotReviveOldServiceOrHandles() {
    SessionModel.SessionId sessionId = SessionModel.SessionId.parse("reused");
    try (SessionKernel kernel = new SessionKernel(false, false)) {
      kernel.create(sessionId);
      InstrumentationHub hub = kernel.instrumentationHub();
      TargetHandle firstTarget =
          hub.registerTarget(interpreterTarget("execution", sessionId.value()));
      NativeInstrumentation oldService = kernel.instrumentation(sessionId);
      NativeInstrumentHandle oldTrace =
          oldService.register(passive("trace", sessionId.value()));
      oldService.attach(
          oldTrace,
          oldService.bindTargetIdentity("execution", firstTarget.generation()));
      kernel.closeSession(sessionId);

      kernel.create(sessionId);
      TargetHandle secondTarget =
          hub.registerTarget(interpreterTarget("execution", sessionId.value()));
      NativeInstrumentation newService = kernel.instrumentation(sessionId);
      NativeTargetHandle newTarget =
          newService.bindTargetIdentity("execution", secondTarget.generation());
      assertEquals(1L, secondTarget.generation());
      InstrumentationException closed =
          assertThrows(
              InstrumentationException.class,
              () ->
                  oldService.bindTargetIdentity(
                      "execution", secondTarget.generation()));
      assertEquals(Code.SESSION_CLOSED, closed.code());
      InstrumentationException stale =
          assertThrows(
              InstrumentationException.class,
              () -> newService.attach(oldTrace, newTarget));
      assertEquals(Code.STALE_INSTRUMENT, stale.code());
    }
  }

  @Test
  public void crossSessionAndCrossRuntimeHandlesFailClosed() {
    SessionModel.SessionId alpha = SessionModel.SessionId.parse("alpha");
    SessionModel.SessionId beta = SessionModel.SessionId.parse("beta");
    try (SessionKernel first = new SessionKernel(false, false);
        SessionKernel second = new SessionKernel(false, false)) {
      first.create(alpha);
      first.create(beta);
      second.create(alpha);
      TargetHandle alphaTarget =
          first
              .instrumentationHub()
              .registerTarget(interpreterTarget("alpha-target", alpha.value()));
      TargetHandle betaTarget =
          first
              .instrumentationHub()
              .registerTarget(interpreterTarget("beta-target", beta.value()));
      TargetHandle foreignTarget =
          second
              .instrumentationHub()
              .registerTarget(interpreterTarget("foreign-target", alpha.value()));
      NativeInstrumentation alphaService = first.instrumentation(alpha);
      NativeInstrumentation betaService = first.instrumentation(beta);
      NativeInstrumentation foreignService = second.instrumentation(alpha);
      NativeInstrumentHandle alphaTrace =
          alphaService.register(passive("trace", alpha.value()));
      NativeTargetHandle betaNative =
          betaService.bindTargetIdentity(
              betaTarget.targetId(), betaTarget.generation());
      NativeTargetHandle foreignNative =
          foreignService.bindTargetIdentity(
              foreignTarget.targetId(), foreignTarget.generation());

      InstrumentationException crossSession =
          assertThrows(
              InstrumentationException.class,
              () -> betaService.attach(alphaTrace, betaNative));
      assertEquals(Code.CROSS_SESSION, crossSession.code());
      InstrumentationException crossRuntime =
          assertThrows(
              InstrumentationException.class,
              () -> foreignService.attach(alphaTrace, foreignNative));
      assertEquals(Code.CROSS_RUNTIME, crossRuntime.code());
      assertEquals(
          alphaTarget, first.instrumentationHub().bindTarget("alpha-target", 0));
    }
  }

  @Test
  public void kernelShutdownInvalidatesRootServiceAndHub() {
    SessionKernel kernel = new SessionKernel(false, false);
    NativeInstrumentation service = kernel.instrumentation(kernel.root().id());
    NativeInstrumentHandle trace = service.register(passive("root-trace", "ROOT"));
    InstrumentationHub hub = kernel.instrumentationHub();
    kernel.close();
    assertTrue(hub.isClosed());
    InstrumentationException closed =
        assertThrows(
            InstrumentationException.class,
            () -> service.drainEvents(trace));
    assertEquals(Code.RUNTIME_CLOSED, closed.code());
  }

  @Test
  public void truffleProducerIsLazyAndEmitsPassiveTopLevelEvents() {
    SessionModel.SessionId sessionId = SessionModel.SessionId.parse("truffle");
    try (SessionKernel kernel = new SessionKernel(false, false)) {
      SessionKernel.Session session = kernel.create(sessionId);
      NativeInstrumentation service = kernel.instrumentation(sessionId);
      NativeTargetHandle target =
          service.bindTargetIdentity(sessionId.value() + "/interpreter", 0);
      TargetDescriptor descriptor = service.targetDescriptor(target);
      assertEquals(new RuntimeBackend("java-truffle"), descriptor.backend());
      assertEquals(
          Set.of(
              Capability.EVENT_SEMANTIC_BOUNDARY,
              Capability.EVENT_EXCEPTION,
              Capability.EVENT_LIFECYCLE,
              Capability.INSPECT_SOURCE_LOCATION),
          descriptor.capabilities());
      NativeInstrumentHandle trace =
          service.register(
              passive(
                  "trace",
                  sessionId.value(),
                  Set.of(EventKind.SEMANTIC_BOUNDARY, EventKind.EXECUTION_TERMINAL),
                  Set.of(Capability.EVENT_SEMANTIC_BOUNDARY, Capability.EVENT_LIFECYCLE),
                  ProjectionRequest.none()));

      session.eval("42");
      assertFalse(session.truffleInstrumentationActive());
      assertTrue(service.drainEvents(trace).events().isEmpty());

      NativeInstrumentation.NativeAttachment attachment = service.attach(trace, target);
      assertTrue(session.truffleInstrumentationActive());
      session.eval("42");
      var events = service.drainEvents(trace).events();
      assertTrue(events.stream().anyMatch(event -> event.event() == EventKind.SEMANTIC_BOUNDARY));
      assertEquals(
          1,
          events.stream()
              .filter(event -> event.event() == EventKind.EXECUTION_TERMINAL)
              .count());

      service.detach(attachment);
      assertFalse(session.truffleInstrumentationActive());
    }
  }

  @Test
  public void topLevelFailureIsReportedOnceAndNestedRootsAreNotTerminal() {
    SessionModel.SessionId sessionId = SessionModel.SessionId.parse("failure");
    try (SessionKernel kernel = new SessionKernel(false, false)) {
      SessionKernel.Session session = kernel.create(sessionId);
      NativeInstrumentation service = kernel.instrumentation(sessionId);
      NativeTargetHandle target =
          service.bindTargetIdentity(sessionId.value() + "/interpreter", 0);
      NativeInstrumentHandle trace =
          service.register(
              passive(
                  "trace",
                  sessionId.value(),
                  Set.of(EventKind.EXCEPTION_RAISE, EventKind.EXECUTION_TERMINAL),
                  Set.of(Capability.EVENT_EXCEPTION, Capability.EVENT_LIFECYCLE),
                  ProjectionRequest.none()));
      service.attach(trace, target);

      session.eval("(do (defn inner [] 1) (inner))");
      var success = service.drainEvents(trace).events();
      assertEquals(
          1,
          success.stream()
              .filter(event -> event.event() == EventKind.EXECUTION_TERMINAL)
              .count());

      assertThrows(IllegalArgumentException.class, () -> session.eval("(throw \"boom\")"));
      var failure = service.drainEvents(trace).events();
      assertEquals(
          1,
          failure.stream().filter(event -> event.event() == EventKind.EXCEPTION_RAISE).count());
      assertEquals(
          1,
          failure.stream()
              .filter(event -> event.event() == EventKind.EXECUTION_TERMINAL)
              .count());
      DeliveredEvent terminal =
          failure.stream()
              .filter(event -> event.event() == EventKind.EXECUTION_TERMINAL)
              .findFirst()
              .orElseThrow();
      assertEquals("failure", terminal.data().get("status"));
    }
  }

  @Test
  public void sourceLocationIsProjectedOnlyToRequestingInstruments() {
    SessionModel.SessionId sessionId = SessionModel.SessionId.parse("locations");
    try (SessionKernel kernel = new SessionKernel(false, false)) {
      SessionKernel.Session session = kernel.create(sessionId);
      NativeInstrumentation service = kernel.instrumentation(sessionId);
      NativeTargetHandle target =
          service.bindTargetIdentity(sessionId.value() + "/interpreter", 0);
      NativeInstrumentHandle withoutLocation =
          service.register(
              passive(
                  "without-location",
                  sessionId.value(),
                  Set.of(EventKind.SEMANTIC_BOUNDARY),
                  Set.of(Capability.EVENT_SEMANTIC_BOUNDARY),
                  ProjectionRequest.none()));
      NativeInstrumentHandle withLocation =
          service.register(
              passive(
                  "with-location",
                  sessionId.value(),
                  Set.of(EventKind.SEMANTIC_BOUNDARY),
                  Set.of(Capability.EVENT_SEMANTIC_BOUNDARY, Capability.INSPECT_SOURCE_LOCATION),
                  new ProjectionRequest(true, null, null, null, null, null, null)));
      service.attach(withoutLocation, target);
      service.attach(withLocation, target);

      session.eval("42", "location.hal", 1, 1);
      var without = service.drainEvents(withoutLocation).events();
      var with = service.drainEvents(withLocation).events();
      assertFalse(without.isEmpty());
      assertTrue(with.stream().allMatch(event -> event.location() != null));
      assertTrue(without.stream().allMatch(event -> event.location() == null));
    }
  }

  @Test
  public void hbcProducerUsesProductionLoopBoundariesAndNativeLocations() {
    SessionModel.SessionId sessionId = SessionModel.SessionId.parse("hbc");
    try (SessionKernel kernel = new SessionKernel(false, false)) {
      SessionKernel.Session session = kernel.create(sessionId);
      NativeInstrumentation service = kernel.instrumentation(sessionId);
      NativeTargetHandle target =
          service.bindTargetIdentity(sessionId.value() + "/hbc", 0);
      TargetDescriptor descriptor = service.targetDescriptor(target);
      assertEquals(new RuntimeBackend("java-hbc"), descriptor.backend());
      assertTrue(descriptor.capabilities().contains(Capability.CONTROL_PAUSE));
      NativeInstrumentHandle trace =
          service.register(
              passive(
                  "hbc-trace",
                  sessionId.value(),
                  Set.of(
                      EventKind.INSTRUCTION_EXECUTE,
                      EventKind.CALL_ENTER,
                      EventKind.CALL_RETURN,
                      EventKind.EXECUTION_TERMINAL),
                  Set.of(
                      Capability.EVENT_INSTRUCTION,
                      Capability.EVENT_CALL,
                      Capability.EVENT_LIFECYCLE,
                      Capability.INSPECT_SOURCE_LOCATION),
                  new ProjectionRequest(true, null, null, null, null, null, null)));
      NativeInstrumentHandle withoutLocation =
          service.register(
              passive(
                  "hbc-without-location",
                  sessionId.value(),
                  Set.of(
                      EventKind.INSTRUCTION_EXECUTE,
                      EventKind.CALL_ENTER,
                      EventKind.CALL_RETURN,
                      EventKind.EXECUTION_TERMINAL),
                  Set.of(
                      Capability.EVENT_INSTRUCTION,
                      Capability.EVENT_CALL,
                      Capability.EVENT_LIFECYCLE),
                  ProjectionRequest.none()));
      var attachment = service.attach(trace, target);
      var withoutLocationAttachment = service.attach(withoutLocation, target);

      HbcProgram program =
          new HbcProgram(
              "demo.main",
              List.of(42L),
              List.of(),
              Map.of(),
              Map.of(),
              Map.of(),
              List.of(
                  function(
                      "entry",
                      new Instruction(HbcProgram.Opcode.CLOSURE, 1, 0, 0),
                      new Instruction(HbcProgram.Opcode.CALL, 0, 0, 0),
                      Instruction.of(HbcProgram.Opcode.RETURN)),
                  function(
                      "answer",
                      new Instruction(HbcProgram.Opcode.CONSTANT, 0, 0, 0),
                      Instruction.of(HbcProgram.Opcode.RETURN))
              ),
              0);
      assertEquals(42L, session.executeHbc(program));

      var events = service.drainEvents(trace).events();
      assertTrue(events.stream().anyMatch(event -> event.event() == EventKind.INSTRUCTION_EXECUTE));
      assertTrue(events.stream().anyMatch(event -> event.event() == EventKind.CALL_ENTER));
      assertTrue(events.stream().anyMatch(event -> event.event() == EventKind.CALL_RETURN));
      DeliveredEvent callEnter =
          events.stream()
              .filter(event -> event.event() == EventKind.CALL_ENTER)
              .findFirst()
              .orElseThrow();
      assertEquals("0", callEnter.data().get("from/function"));
      assertEquals("1", callEnter.data().get("to/function"));
      DeliveredEvent callReturn =
          events.stream()
              .filter(event -> event.event() == EventKind.CALL_RETURN)
              .findFirst()
              .orElseThrow();
      assertEquals("1", callReturn.data().get("from/function"));
      assertEquals("0", callReturn.data().get("to/function"));
      assertEquals(
          1,
          events.stream()
              .filter(event -> event.event() == EventKind.EXECUTION_TERMINAL)
              .count());
      assertTrue(
          events.stream()
              .filter(event -> event.event() == EventKind.INSTRUCTION_EXECUTE)
              .allMatch(
                  event ->
                      event.location() != null
                          && event.location().instructionPointer() != null
                          && event.location().formPath().isEmpty()));
      var without = service.drainEvents(withoutLocation).events();
      assertFalse(without.isEmpty());
      assertTrue(without.stream().allMatch(event -> event.location() == null));

      service.detach(attachment);
      service.detach(withoutLocationAttachment);
      assertEquals(42L, session.executeHbc(program));
      assertTrue(service.drainEvents(trace).events().isEmpty());
    }
  }

  @Test
  public void hbcControlRetainsStateAcrossPauseStepResumeAndTerminate() {
    SessionModel.SessionId sessionId = SessionModel.SessionId.parse("hbc-control");
    try (SessionKernel kernel = new SessionKernel(false, false)) {
      SessionKernel.Session session = kernel.create(sessionId);
      session.eval("nil");
      NativeInstrumentation service = kernel.instrumentation(sessionId);
      NativeTargetHandle target =
          service.bindTargetIdentity(sessionId.value() + "/hbc", 0);
      NativeInstrumentHandle controller =
          service.register(control("hbc-controller", sessionId.value()));
      var controllerAttachment = service.attach(controller, target);
      NativeControlLease lease = service.acquireControlLease(controller, target);
      HbcProgram program =
          new HbcProgram(
              "hbc-control",
              List.of(42L),
              List.of(),
              Map.of(),
              Map.of(),
              Map.of(),
              List.of(function(
                  "entry",
                  new Instruction(HbcProgram.Opcode.CONSTANT, 0, 0, 0),
                  Instruction.of(HbcProgram.Opcode.RETURN))),
              0);

      service.issueDirective(lease, InstrumentationModel.InstrumentDirective.SUSPEND);
      Object suspended = session.executeHbc(program);
      assertTrue(suspended instanceof HbcMachine.HbcSuspension);
      assertEquals(0, ((HbcMachine.HbcSuspension) suspended).instructionPointer());
      assertEquals(
          HbcMachine.SuspensionKind.CONTROL_PAUSE,
          ((HbcMachine.HbcSuspension) suspended).kind());
      assertTrue(
          service.drainEvents(controller).events().stream()
              .noneMatch(event -> event.event() == EventKind.MACHINE_SUSPEND));
      Object retained = session.executeHbc(program);
      assertEquals(suspended, retained);

      service.issueDirective(lease, InstrumentationModel.InstrumentDirective.STEP_NEXT);
      suspended = session.executeHbc(program);
      assertTrue(suspended instanceof HbcMachine.HbcSuspension);
      assertEquals(1, ((HbcMachine.HbcSuspension) suspended).instructionPointer());
      assertTrue(
          service.drainEvents(controller).events().stream()
              .noneMatch(event -> event.event() == EventKind.MACHINE_SUSPEND));

      service.issueDirective(lease, InstrumentationModel.InstrumentDirective.CONTINUE);
      assertEquals(42L, session.executeHbc(program));
      assertEquals(
          1,
          service
              .drainEvents(controller)
              .events()
              .stream()
              .filter(event -> event.event() == EventKind.EXECUTION_TERMINAL)
              .count());

      service.issueDirective(lease, InstrumentationModel.InstrumentDirective.SUSPEND);
      assertTrue(session.executeHbc(program) instanceof HbcMachine.HbcSuspension);
      service.issueDirective(lease, InstrumentationModel.InstrumentDirective.TERMINATE);
      assertThrows(HaraException.class, () -> session.executeHbc(program));
      List<DeliveredEvent> terminalEvents = service.drainEvents(controller).events();
      assertEquals(
          1,
          terminalEvents.stream()
              .filter(event -> event.event() == EventKind.EXECUTION_TERMINAL)
              .count());
      assertEquals(
          "cancelled",
          terminalEvents.stream()
              .filter(event -> event.event() == EventKind.EXECUTION_TERMINAL)
              .findFirst()
              .orElseThrow()
              .data()
              .get("status"));
      assertEquals(42L, session.executeHbc(program));
      service.issueDirective(lease, InstrumentationModel.InstrumentDirective.SUSPEND);
      assertTrue(session.executeHbc(program) instanceof HbcMachine.HbcSuspension);
      service.detach(controllerAttachment);
      assertEquals(42L, session.executeHbc(program));
    }
  }

  @Test
  public void hbcProducerPublishesHandledUnwindAndTerminalOrdering() {
    SessionModel.SessionId sessionId = SessionModel.SessionId.parse("hbc-unwind");
    try (SessionKernel kernel = new SessionKernel(false, false)) {
      SessionKernel.Session session = kernel.create(sessionId);
      NativeInstrumentation service = kernel.instrumentation(sessionId);
      NativeTargetHandle target =
          service.bindTargetIdentity(sessionId.value() + "/hbc", 0);
      NativeInstrumentHandle trace =
          service.register(
              passive(
                  "hbc-unwind-trace",
                  sessionId.value(),
                  Set.of(EventKind.EXCEPTION_UNWIND, EventKind.EXECUTION_TERMINAL),
                  Set.of(
                      Capability.EVENT_EXCEPTION,
                      Capability.EVENT_LIFECYCLE,
                      Capability.INSPECT_SOURCE_LOCATION),
                  new ProjectionRequest(true, null, null, null, null, null, null)));
      service.attach(trace, target);

      HbcProgram program =
          new HbcProgram(
              "demo.unwind",
              List.of(
                  new hara.lang.base.Ex.Info(
                      "caught", hara.lang.data.Map.Standard.from(null))),
              List.of(),
              Map.of(),
              Map.of(),
              Map.of(),
              List.of(
                  new Function(
                      "entry",
                      false,
                      0,
                      false,
                      0,
                      1,
                      1,
                      List.of(
                          new Instruction(HbcProgram.Opcode.CONSTANT, 0, 0, 0),
                          Instruction.of(HbcProgram.Opcode.THROW),
                          Instruction.of(HbcProgram.Opcode.RETURN),
                          new Instruction(HbcProgram.Opcode.LOAD_LOCAL, 0, 0, 0),
                          Instruction.of(HbcProgram.Opcode.RETURN)),
                      List.of(
                          new HbcProgram.Position(0, 1, 1),
                          new HbcProgram.Position(1, 1, 2),
                          new HbcProgram.Position(2, 1, 3),
                          new HbcProgram.Position(3, 1, 4),
                          new HbcProgram.Position(4, 1, 5)),
                      List.of(
                          new HbcProgram.TryEntry(
                              0,
                              2,
                              0,
                              List.of(new HbcProgram.CatchEntry("Exception", 0, 3)),
                              null,
                              null,
                              null)))),
                  0);
      assertTrue(HaraBox.unwrap(session.executeHbc(program)) instanceof hara.lang.protocol.IExInfo);

      List<DeliveredEvent> events = service.drainEvents(trace).events();
      assertEquals(2, events.size());
      assertEquals(EventKind.EXCEPTION_UNWIND, events.get(0).event());
      assertEquals(EventKind.EXECUTION_TERMINAL, events.get(1).event());
      assertEquals("1", Integer.toString(events.get(0).location().instructionPointer()));
      assertEquals("returned", events.get(1).data().get("status"));
    }
  }

  private static Function function(String name, Instruction... instructions) {
    return new Function(
        name,
        false,
        0,
        false,
        0,
        0,
        2,
        List.of(instructions),
        Arrays.asList(new HbcProgram.Position[instructions.length]),
        List.of());
  }

  private static TargetDescriptor interpreterTarget(String id, String session) {
    return new TargetDescriptor(
        id,
        session,
        TargetKind.INTERPRETER,
        new RuntimeBackend("java"),
        Set.of(Capability.EVENT_LIFECYCLE));
  }

  private static InstrumentRegistration passive(String id, String session) {
    return passive(
        id,
        session,
        Set.of(EventKind.EXECUTION_TERMINAL),
        Set.of(Capability.EVENT_LIFECYCLE),
        ProjectionRequest.none());
  }

  private static InstrumentRegistration passive(
      String id,
      String session,
      Set<EventKind> events,
      Set<Capability> capabilities,
      ProjectionRequest projection) {
    return new InstrumentRegistration(
        id,
        session,
        InstrumentMode.PASSIVE,
        capabilities,
        events,
        new InstrumentFilter(session, Set.of(), Set.of(), Set.of()),
        projection,
        EventDelivery.queue(8));
  }

  private static InstrumentRegistration control(String id, String session) {
    return new InstrumentRegistration(
        id,
        session,
        InstrumentMode.CONTROL,
        Set.of(
            Capability.EVENT_SUSPENSION,
            Capability.EVENT_LIFECYCLE,
            Capability.CONTROL_PAUSE,
            Capability.CONTROL_SINGLE_STEP,
            Capability.CONTROL_RESUME,
            Capability.CONTROL_TERMINATE),
        Set.of(
            EventKind.MACHINE_SUSPEND,
            EventKind.EXECUTION_TERMINAL),
        new InstrumentFilter(session, Set.of(), Set.of(), Set.of()),
        ProjectionRequest.none(),
        EventDelivery.queue(8));
  }
}
