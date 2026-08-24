package hara.truffle;

import hara.truffle.InstrumentationModel.EventKind;
import hara.truffle.bytecode.HbcNativeInstruction;
import hara.truffle.bytecode.HbcProgram;
import java.util.List;
import java.util.Map;

/** Small boundary shared by generated HBC operations and the existing instrumentation model. */
public final class HbcInstrumentationBridge {
  private HbcInstrumentationBridge() {}

  public static void instruction(HaraContext context, HbcNativeInstruction location) {
    if (!context.hbcInstrumentationEnabled(EventKind.INSTRUCTION_EXECUTE)) return;
    HbcProgram.Function function = location.program().functions().get(location.functionIndex());
    context.publishHbcEvent(
        EventKind.INSTRUCTION_EXECUTE,
        location.instructionPointer(),
        function.name(),
        location.program().namespace(),
        function.sourceMap().get(location.instructionPointer()),
        Map.of("opcode", function.code().get(location.instructionPointer()).opcode().name()),
        InstrumentationEventAccess.none());
  }

  public static Object terminal(
      HaraContext context, HbcNativeInstruction location, Object value) {
    if (context.hbcInstrumentationEnabled(EventKind.EXECUTION_TERMINAL)) {
      HbcProgram.Function function = location.program().functions().get(location.functionIndex());
      context.publishHbcEvent(
          EventKind.EXECUTION_TERMINAL,
          location.instructionPointer(),
          function.name(),
          location.program().namespace(),
          function.sourceMap().get(location.instructionPointer()),
          Map.of("status", "returned"),
          InstrumentationEventAccess.none());
    }
    return value;
  }

  static void machineEvent(
      HaraContext context,
      EventKind event,
      HbcProgram program,
      int functionIndex,
      HbcProgram.Function function,
      int instructionPointer,
      Map<String, String> data,
      Object[] locals,
      List<Object> stack,
      List<HbcMachine.CallFrame> calls) {
    machineEventAt(
        context,
        event,
        program,
        functionIndex,
        function,
        instructionPointer,
        function,
        instructionPointer,
        data,
        locals,
        stack,
        calls,
        "running",
        null,
        null);
  }

  static void machineEventAt(
      HaraContext context,
      EventKind event,
      HbcProgram program,
      int functionIndex,
      HbcProgram.Function function,
      int instructionPointer,
      HbcProgram.Function locationFunction,
      int locationInstructionPointer,
      Map<String, String> data,
      Object[] locals,
      List<Object> stack,
      List<HbcMachine.CallFrame> calls) {
    machineEventAt(
        context,
        event,
        program,
        functionIndex,
        function,
        instructionPointer,
        locationFunction,
        locationInstructionPointer,
        data,
        locals,
        stack,
        calls,
        "running",
        null,
        null);
  }

  static void machineTerminal(
      HaraContext context,
      HbcProgram program,
      int functionIndex,
      HbcProgram.Function function,
      int instructionPointer,
      Map<String, String> data,
      Object[] locals,
      List<Object> stack,
      List<HbcMachine.CallFrame> calls,
      String status,
      Object result,
      String error) {
    machineEventAt(
        context,
        EventKind.EXECUTION_TERMINAL,
        program,
        functionIndex,
        function,
        instructionPointer,
        function,
        instructionPointer,
        data,
        locals,
        stack,
        calls,
        status,
        result,
        error);
  }

  static Map<String, String> instructionData(
      HbcProgram.Opcode opcode, int stackDepth, int callDepth) {
    return Map.of(
        "opcode", opcode.name(),
        "stack/depth", Integer.toString(stackDepth),
        "call/depth", Integer.toString(callDepth));
  }

  static Map<String, String> transitionData(HbcBoundary.Transition transition) {
    return Map.of(
        "from/function", Integer.toString(transition.fromFunction()),
        "from/ip", Integer.toString(transition.fromInstructionPointer()),
        "to/function", Integer.toString(transition.toFunction()),
        "to/ip", Integer.toString(transition.toInstructionPointer()));
  }

  static Map<String, String> callData(HbcBoundary.Transition transition) {
    return transitionData(transition);
  }

  static Map<String, String> terminalData(
      HbcBoundary.Terminal terminal, String status) {
    return Map.of(
        "status", status,
        "stack/depth", Integer.toString(terminal.stackDepth()),
        "call/depth", Integer.toString(terminal.callDepth()));
  }

  private static void machineEventAt(
      HaraContext context,
      EventKind event,
      HbcProgram program,
      int functionIndex,
      HbcProgram.Function function,
      int instructionPointer,
      HbcProgram.Function locationFunction,
      int locationInstructionPointer,
      Map<String, String> data,
      Object[] locals,
      List<Object> stack,
      List<HbcMachine.CallFrame> calls,
      String status,
      Object result,
      String error) {
    if (!context.hbcInstrumentationEnabled(event)) return;
    InstrumentationEventAccess access =
        "running".equals(status)
            ? HbcInstrumentationAccess.live(
                program, functionIndex, function, instructionPointer, locals, stack, calls)
            : HbcInstrumentationAccess.terminal(
                program,
                functionIndex,
                function,
                instructionPointer,
                locals,
                stack,
                calls,
                status,
                result,
                error);
    context.publishHbcEvent(
        event,
        locationInstructionPointer,
        locationFunction.name(),
        program.namespace(),
        locationFunction.sourceMap().get(locationInstructionPointer),
        data,
        access);
  }
}
