package hara.truffle;

import static org.junit.Assert.assertEquals;

import hara.truffle.InstrumentationModel.Capability;
import hara.truffle.InstrumentationModel.EventDelivery;
import hara.truffle.InstrumentationModel.EventKind;
import hara.truffle.InstrumentationModel.InstrumentFilter;
import hara.truffle.InstrumentationModel.InstrumentMode;
import hara.truffle.InstrumentationModel.InstrumentRegistration;
import hara.truffle.InstrumentationModel.ProjectionRequest;
import hara.truffle.bytecode.HbcProgram;
import hara.truffle.bytecode.HbcProgram.Function;
import hara.truffle.bytecode.HbcProgram.Instruction;
import hara.truffle.bytecode.HbcProgram.Primitive;
import org.graalvm.polyglot.Context;
import java.util.Arrays;
import java.util.List;
import java.util.Map;
import java.util.Set;
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
      assertEquals("return", events.get(events.size() - 1).data().get("status"));
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
  public void eligibleStaticCallsStayInTheGeneratedTier() {
    try (SessionKernel kernel = new SessionKernel(false, false)) {
      SessionKernel.Session session = kernel.create(SessionModel.SessionId.parse("hbc-static"));
      assertEquals(42L, session.executeHbc(staticCallProgram()));
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
}
