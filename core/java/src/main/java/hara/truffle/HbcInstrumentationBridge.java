package hara.truffle;

import hara.truffle.InstrumentationModel.EventKind;
import hara.truffle.bytecode.HbcNativeInstruction;
import hara.truffle.bytecode.HbcProgram;
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
        Map.of("opcode", function.code().get(location.instructionPointer()).opcode().name()));
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
          Map.of("status", "return"));
    }
    return value;
  }
}
