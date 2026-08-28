package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertTrue;

import hara.truffle.InstrumentationModel.Capability;
import hara.truffle.InstrumentationModel.EventDelivery;
import hara.truffle.InstrumentationModel.EventKind;
import hara.truffle.InstrumentationModel.InstrumentFilter;
import hara.truffle.InstrumentationModel.InstrumentMode;
import hara.truffle.InstrumentationModel.InstrumentRegistration;
import hara.truffle.InstrumentationModel.ProjectionLimits;
import hara.truffle.InstrumentationModel.ProjectionRequest;
import hara.truffle.bytecode.HbcProgram;
import hara.truffle.bytecode.HbcProgram.Function;
import hara.truffle.bytecode.HbcProgram.Instruction;
import hara.truffle.bytecode.HbcProgram.Primitive;
import org.graalvm.polyglot.Context;
import java.math.BigInteger;
import java.util.Arrays;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.CountDownLatch;
import org.junit.Test;

public class HbcNativeExecutionTest {
  @Test
  public void passiveInstructionAndTerminalTracingUsesTheGeneratedTier() {
    SessionModel.SessionId sessionId = SessionModel.SessionId.parse("hbc-native");
    try (SessionKernel kernel = new SessionKernel(false, false)) {
      SessionKernel.Session session = kernel.create(sessionId);
      NativeInstrumentation service = kernel.instrumentation(sessionId);
      NativeInstrumentation.NativeTargetHandle target =
          service.bindTargetIdentity(sessionId.value() + "/hbc", 0);
      NativeInstrumentation.NativeInstrumentHandle trace =
          service.register(
              new InstrumentRegistration(
                  "native-trace",
                  sessionId.value(),
                  InstrumentMode.PASSIVE,
                  Set.of(Capability.EVENT_INSTRUCTION, Capability.EVENT_LIFECYCLE),
                  Set.of(EventKind.INSTRUCTION_EXECUTE, EventKind.EXECUTION_TERMINAL),
                  new InstrumentFilter(sessionId.value(), Set.of(), Set.of(), Set.of()),
                  ProjectionRequest.none(),
                  EventDelivery.queue(16)));
      service.attach(trace, target);

      assertEquals(42L, session.executeHbc(arithmeticProgram()));
      var events = service.drainEvents(trace).events();
      assertEquals(5, events.size());
      assertEquals(
          4,
          events.stream().filter(event -> event.event() == EventKind.INSTRUCTION_EXECUTE).count());
      assertEquals(
          1,
          events.stream().filter(event -> event.event() == EventKind.EXECUTION_TERMINAL).count());
      assertEquals("returned", events.get(events.size() - 1).data().get("status"));
    }
  }

  @Test
  public void reducibleConditionalAndLoopControlExecuteInTheGeneratedTier() {
    try (SessionKernel kernel = new SessionKernel(false, false)) {
      SessionKernel.Session session = kernel.create(SessionModel.SessionId.parse("hbc-control"));
      assertEquals(11L, session.executeHbc(conditionalProgram(true)));
      assertEquals(22L, session.executeHbc(conditionalProgram(false)));
      assertEquals(5L, session.executeHbc(loopProgram()));
    }
  }

  @Test
  public void generatedTierAppliesHaraTruthinessToConditionalValues() {
    try (SessionKernel kernel = new SessionKernel(false, false)) {
      SessionKernel.Session session = kernel.create(SessionModel.SessionId.parse("hbc-truthy"));
      assertEquals(
          11L,
          session.executeHbc(
              truthinessProgram(
                  new Instruction(HbcProgram.Opcode.CONSTANT, 2, 0, 0),
                  List.of(11L, 22L, 0L))));
      assertEquals(
          11L,
          session.executeHbc(
              truthinessProgram(
                  new Instruction(HbcProgram.Opcode.CONSTANT, 2, 0, 0),
                  List.of(11L, 22L, ""))));
      assertEquals(
          22L,
          session.executeHbc(truthinessProgram(Instruction.of(HbcProgram.Opcode.NIL), List.of(11L, 22L))));
    }
  }

  @Test
  public void eligibleStaticCallsStayInTheGeneratedTier() {
    try (SessionKernel kernel = new SessionKernel(false, false)) {
      SessionKernel.Session session = kernel.create(SessionModel.SessionId.parse("hbc-static"));
      assertEquals(42L, session.executeHbc(staticCallProgram()));
    }
  }

  @Test
  public void generatedTierFallbackPreservesArbitraryPrecisionResults() {
    try (SessionKernel kernel = new SessionKernel(false, false)) {
      SessionKernel.Session session = kernel.create(SessionModel.SessionId.parse("hbc-bigint"));
      Function entry =
          new Function(
              null,
              false,
              0,
              false,
              0,
              0,
              2,
              List.of(
                  new Instruction(HbcProgram.Opcode.CONSTANT, 0, 0, 0),
                  new Instruction(HbcProgram.Opcode.CONSTANT, 1, 0, 0),
                  new Instruction(HbcProgram.Opcode.PRIMITIVE, Primitive.ADD.id(), 2, 0),
                  Instruction.of(HbcProgram.Opcode.RETURN)),
              Arrays.asList(null, null, null, null),
              List.of());
      HbcProgram program =
          new HbcProgram(
              "hbc-bigint",
              List.of(BigInteger.valueOf(Long.MAX_VALUE), BigInteger.ONE),
              List.of(),
              Map.of(),
              Map.of(),
              Map.of(),
              List.of(entry),
              0);

      assertEquals(BigInteger.ONE.shiftLeft(63), HaraBox.unwrap(session.executeHbc(program)));
    }
  }

  @Test
  public void passiveTracingIncludesStructuredControlInstructions() {
    SessionModel.SessionId sessionId = SessionModel.SessionId.parse("hbc-native-control-trace");
    try (SessionKernel kernel = new SessionKernel(false, false)) {
      SessionKernel.Session session = kernel.create(sessionId);
      NativeInstrumentation service = kernel.instrumentation(sessionId);
      NativeInstrumentation.NativeTargetHandle target =
          service.bindTargetIdentity(sessionId.value() + "/hbc", 0);
      NativeInstrumentation.NativeInstrumentHandle trace =
          service.register(
              new InstrumentRegistration(
                  "native-control-trace",
                  sessionId.value(),
                  InstrumentMode.PASSIVE,
                  Set.of(Capability.EVENT_INSTRUCTION, Capability.EVENT_LIFECYCLE),
                  Set.of(EventKind.INSTRUCTION_EXECUTE, EventKind.EXECUTION_TERMINAL),
                  new InstrumentFilter(sessionId.value(), Set.of(), Set.of(), Set.of()),
                  ProjectionRequest.none(),
                  EventDelivery.queue(16)));
      service.attach(trace, target);

      assertEquals(11L, session.executeHbc(conditionalProgram(true)));
      var events = service.drainEvents(trace).events();
      assertEquals(6, events.size());
      assertEquals(
          List.of("TRUE", "JUMP_IF_FALSE", "CONSTANT", "JUMP", "RETURN"),
          events.stream()
              .filter(event -> event.event() == EventKind.INSTRUCTION_EXECUTE)
              .map(event -> event.data().get("opcode"))
              .toList());
    }
  }

  @Test
  public void richHbcProjectionUsesThePortableMachineAndIsBounded() {
    SessionModel.SessionId sessionId = SessionModel.SessionId.parse("hbc-projection");
    try (SessionKernel kernel = new SessionKernel(false, false)) {
      SessionKernel.Session session = kernel.create(sessionId);
      NativeInstrumentation service = kernel.instrumentation(sessionId);
      NativeInstrumentation.NativeTargetHandle target =
          service.bindTargetIdentity(sessionId.value() + "/hbc", 0);
      NativeInstrumentation.NativeInstrumentHandle inspect =
          service.register(
              new InstrumentRegistration(
                  "hbc-stack-inspector",
                  sessionId.value(),
                  InstrumentMode.PASSIVE,
                  Set.of(Capability.EVENT_INSTRUCTION, Capability.INSPECT_STACK),
                  Set.of(EventKind.INSTRUCTION_EXECUTE),
                  new InstrumentFilter(sessionId.value(), Set.of(), Set.of(), Set.of()),
                  new ProjectionRequest(
                      false,
                      null,
                      null,
                      null,
                      new ProjectionLimits(4, 4, 128),
                      null,
                      null),
                  EventDelivery.queue(16)));
      service.attach(inspect, target);

      assertEquals(42L, session.executeHbc(arithmeticProgram()));
      var events = service.drainEvents(inspect).events();
      var returnInstruction =
          events.stream()
              .filter(
                  event ->
                      event.event() == EventKind.INSTRUCTION_EXECUTE
                          && "RETURN".equals(event.data().get("opcode")))
              .findFirst()
              .orElseThrow();
      assertEquals("42", returnInstruction.projection().stack().fields().get("stack/0"));
      assertEquals(0L, returnInstruction.droppedBefore());
    }
  }

  @Test
  public void generatedHbcLocationsPreserveSourceMapOffsets() {
    SessionModel.SessionId sessionId = SessionModel.SessionId.parse("hbc-source-map");
    try (SessionKernel kernel = new SessionKernel(false, false)) {
      SessionKernel.Session session = kernel.create(sessionId);
      NativeInstrumentation service = kernel.instrumentation(sessionId);
      NativeInstrumentation.NativeTargetHandle target =
          service.bindTargetIdentity(sessionId.value() + "/hbc", 0);
      NativeInstrumentation.NativeInstrumentHandle trace =
          service.register(
              new InstrumentRegistration(
                  "hbc-source-map-trace",
                  sessionId.value(),
                  InstrumentMode.PASSIVE,
                  Set.of(Capability.EVENT_INSTRUCTION, Capability.INSPECT_SOURCE_LOCATION),
                  Set.of(EventKind.INSTRUCTION_EXECUTE),
                  new InstrumentFilter(sessionId.value(), Set.of(), Set.of(), Set.of()),
                  new ProjectionRequest(true, null, null, null, null, null, null),
                  EventDelivery.queue(8)));
      service.attach(trace, target);
      Function entry =
          new Function(
              "source-entry",
              false,
              0,
              false,
              0,
              0,
              1,
              List.of(
                  new Instruction(HbcProgram.Opcode.CONSTANT, 0, 0, 0),
                  Instruction.of(HbcProgram.Opcode.RETURN)),
              List.of(new HbcProgram.Position(41, 3, 2), new HbcProgram.Position(43, 3, 4)),
              List.of());
      HbcProgram program =
          new HbcProgram(
              "hbc-source-map",
              List.of(9L),
              List.of(),
              Map.of(),
              Map.of(),
              Map.of(),
              List.of(entry),
              0);

      assertEquals(9L, session.executeHbc(program));
      var first = service.drainEvents(trace).events().get(0);
      assertEquals(41, first.location().span().start());
      assertEquals(41, first.location().span().end());
    }
  }

  @Test
  public void portableMachineRetainsExactAwaitContinuationUntilSettlement() throws Exception {
    try (Context polyglot = Context.newBuilder(HaraLanguage.ID).build()) {
      polyglot.eval(HaraLanguage.ID, "nil");
      polyglot.enter();
      try {
        HaraContext context = HaraLanguage.currentContext();
        CountDownLatch release = new CountDownLatch(1);
        Object promise =
            context.hbcAsync(
                () -> {
                  try {
                    release.await();
                    return 7L;
                  } catch (InterruptedException error) {
                    Thread.currentThread().interrupt();
                    throw new HaraException("await test interrupted");
                  }
                });
        HbcProgram program = awaitingProgram(promise);

        Object suspended = HbcMachine.execute(program, context);
        assertTrue(suspended instanceof HbcMachine.HbcSuspension);
        assertEquals(
            HbcMachine.SuspensionKind.AWAIT,
            ((HbcMachine.HbcSuspension) suspended).kind());

        release.countDown();
        Object result = suspended;
        for (int attempt = 0;
            attempt < 100 && result instanceof HbcMachine.HbcSuspension;
            attempt++) {
          Thread.sleep(5);
          result = HbcMachine.execute(program, context);
        }
        assertEquals(7L, result);
      } finally {
        polyglot.leave();
      }
    }
  }

  @Test
  public void nativeYieldIsExposedThroughTheExistingCoroutineContract() {
    try (Context polyglot = Context.newBuilder(HaraLanguage.ID).build()) {
      polyglot.eval(HaraLanguage.ID, "nil");
      polyglot.enter();
      try {
        HaraContext context = HaraLanguage.currentContext();
        HbcProgram program = yieldingProgram();
        HbcMachine.HbcClosure closure =
            new HbcMachine.HbcClosure(program, context, 0, new Object[0]);
        Object coroutine = StdFoundationCoroutine.create(context, new Object[] {closure});

        assertEquals(
            7L,
            StdFoundationCoroutine.resume(context, new Object[] {coroutine}));
        assertEquals(
            42L,
            StdFoundationCoroutine.resume(context, new Object[] {coroutine, 99L}));
        assertEquals(
            StdFoundationCoroutine.STATUS_DEAD,
            StdFoundationCoroutine.status(context, new Object[] {coroutine}));
      } finally {
        polyglot.leave();
      }
    }
  }

  private static HbcProgram conditionalProgram(boolean condition) {
    Function entry =
        new Function(
            null,
            false,
            0,
            false,
            0,
            0,
            1,
            List.of(
                condition
                    ? Instruction.of(HbcProgram.Opcode.TRUE)
                    : Instruction.of(HbcProgram.Opcode.FALSE),
                new Instruction(HbcProgram.Opcode.JUMP_IF_FALSE, 4, 0, 0),
                new Instruction(HbcProgram.Opcode.CONSTANT, 0, 0, 0),
                new Instruction(HbcProgram.Opcode.JUMP, 5, 0, 0),
                new Instruction(HbcProgram.Opcode.CONSTANT, 1, 0, 0),
                Instruction.of(HbcProgram.Opcode.RETURN)),
            Arrays.asList(null, null, null, null, null, null),
            List.of());
    return new HbcProgram(
        "native-conditional",
        List.of(11L, 22L),
        List.of(),
        Map.of(),
        Map.of(),
        Map.of(),
        List.of(entry),
        0);
  }

  private static HbcProgram truthinessProgram(Instruction condition, List<Object> constants) {
    Function entry =
        new Function(
            null,
            false,
            0,
            false,
            0,
            0,
            1,
            List.of(
                condition,
                new Instruction(HbcProgram.Opcode.JUMP_IF_FALSE, 4, 0, 0),
                new Instruction(HbcProgram.Opcode.CONSTANT, 0, 0, 0),
                new Instruction(HbcProgram.Opcode.JUMP, 5, 0, 0),
                new Instruction(HbcProgram.Opcode.CONSTANT, 1, 0, 0),
                Instruction.of(HbcProgram.Opcode.RETURN)),
            Arrays.asList(null, null, null, null, null, null),
            List.of());
    return new HbcProgram(
        "native-truthiness", constants, List.of(), Map.of(), Map.of(), Map.of(), List.of(entry), 0);
  }

  private static HbcProgram loopProgram() {
    Function entry =
        new Function(
            null,
            false,
            0,
            false,
            0,
            1,
            2,
            List.of(
                new Instruction(HbcProgram.Opcode.CONSTANT, 0, 0, 0),
                Instruction.of(HbcProgram.Opcode.STORE_LOCAL),
                new Instruction(HbcProgram.Opcode.LOAD_LOCAL, 0, 0, 0),
                new Instruction(HbcProgram.Opcode.CONSTANT, 1, 0, 0),
                new Instruction(HbcProgram.Opcode.PRIMITIVE, Primitive.LESS.id(), 2, 0),
                new Instruction(HbcProgram.Opcode.JUMP_IF_FALSE, 11, 0, 0),
                new Instruction(HbcProgram.Opcode.LOAD_LOCAL, 0, 0, 0),
                new Instruction(HbcProgram.Opcode.CONSTANT, 2, 0, 0),
                new Instruction(HbcProgram.Opcode.PRIMITIVE, Primitive.ADD.id(), 2, 0),
                Instruction.of(HbcProgram.Opcode.STORE_LOCAL),
                new Instruction(HbcProgram.Opcode.JUMP, 2, 0, 0),
                new Instruction(HbcProgram.Opcode.LOAD_LOCAL, 0, 0, 0),
                Instruction.of(HbcProgram.Opcode.RETURN)),
            Arrays.asList(
                null, null, null, null, null, null, null, null, null, null, null, null, null),
            List.of());
    return new HbcProgram(
        "native-loop",
        List.of(0L, 5L, 1L),
        List.of(),
        Map.of(),
        Map.of(),
        Map.of(),
        List.of(entry),
        0);
  }

  private static HbcProgram yieldingProgram() {
    Function entry =
        new Function(
            "yielding",
            false,
            0,
            false,
            0,
            0,
            1,
            List.of(
                new Instruction(HbcProgram.Opcode.CONSTANT, 0, 0, 0),
                Instruction.of(HbcProgram.Opcode.YIELD),
                new Instruction(HbcProgram.Opcode.CONSTANT, 1, 0, 0),
                Instruction.of(HbcProgram.Opcode.RETURN)),
            Arrays.asList(null, null, null, null),
            List.of());
    return new HbcProgram(
        "native-yield",
        List.of(7L, 42L),
        List.of(),
        Map.of(),
        Map.of(),
        Map.of(),
        List.of(entry),
        0);
  }

  private static HbcProgram staticCallProgram() {
    Function entry =
        new Function(
            null,
            false,
            0,
            false,
            0,
            0,
            2,
            List.of(
                new Instruction(HbcProgram.Opcode.CONSTANT, 0, 0, 0),
                new Instruction(HbcProgram.Opcode.CALL_STATIC, 1, 1, 0),
                Instruction.of(HbcProgram.Opcode.RETURN)),
            Arrays.asList(null, null, null),
            List.of());
    Function callee =
        new Function(
            "add-two",
            false,
            1,
            false,
            0,
            1,
            2,
            List.of(
                new Instruction(HbcProgram.Opcode.LOAD_LOCAL, 0, 0, 0),
                new Instruction(HbcProgram.Opcode.CONSTANT, 1, 0, 0),
                new Instruction(HbcProgram.Opcode.PRIMITIVE, Primitive.ADD.id(), 2, 0),
                Instruction.of(HbcProgram.Opcode.RETURN)),
            Arrays.asList(null, null, null, null),
            List.of());
    return new HbcProgram(
        "native-static", List.of(40L, 2L), List.of(), Map.of(), Map.of(), Map.of(),
        List.of(entry, callee), 0);
  }

  private static HbcProgram arithmeticProgram() {
    Function entry =
        new Function(
            null,
            false,
            0,
            false,
            0,
            0,
            2,
            List.of(
                new Instruction(HbcProgram.Opcode.CONSTANT, 0, 0, 0),
                new Instruction(HbcProgram.Opcode.CONSTANT, 1, 0, 0),
                new Instruction(HbcProgram.Opcode.PRIMITIVE, Primitive.ADD.id(), 2, 0),
                Instruction.of(HbcProgram.Opcode.RETURN)),
            Arrays.asList(null, null, null, null),
            List.of());
    return new HbcProgram(
        "native-arithmetic",
        List.of(19L, 23L),
        List.of(),
        Map.of(),
        Map.of(),
        Map.of(),
        List.of(entry),
        0);
  }

  private static HbcProgram awaitingProgram(Object promise) {
    Function entry =
        new Function(
            "awaiting",
            false,
            0,
            false,
            0,
            0,
            1,
            List.of(
                new Instruction(HbcProgram.Opcode.CONSTANT, 0, 0, 0),
                Instruction.of(HbcProgram.Opcode.AWAIT),
                Instruction.of(HbcProgram.Opcode.RETURN)),
            Arrays.asList(null, null, null),
            List.of());
    return new HbcProgram(
        "awaiting",
        List.of(promise),
        List.of(),
        Map.of(),
        Map.of(),
        Map.of(),
        List.of(entry),
        0);
  }
}
