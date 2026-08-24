package hara.truffle.bytecode;

import java.util.List;

/** Recognizes the reducible HBC shapes which can be represented by the Bytecode DSL. */
final class HbcControlFlow {
  private HbcControlFlow() {}

  static String validate(HbcProgram.Function function) {
    try {
      walk(function.code(), 0, function.code().size());
      return null;
    } catch (UnsupportedControlFlow error) {
      return error.getMessage();
    }
  }

  private static void walk(List<HbcProgram.Instruction> code, int start, int end) {
    int ip = start;
    while (ip < end) {
      LoopShape loop = loopShape(code, ip, end);
      if (loop != null) {
        walk(code, loop.bodyStart(), loop.bodyEnd());
        ip = loop.exitIp();
        continue;
      }

      HbcProgram.Instruction instruction = code.get(ip);
      if (instruction.opcode() == HbcProgram.Opcode.JUMP_IF_FALSE) {
        IfShape branch = ifShape(code, ip, end);
        if (branch == null) unsupported("conditional at " + ip + " is not reducible");
        walk(code, branch.thenStart(), branch.thenEnd());
        if (branch.hasElse()) walk(code, branch.elseStart(), branch.mergeIp());
        ip = branch.mergeIp();
        continue;
      }
      if (instruction.opcode() == HbcProgram.Opcode.JUMP) {
        unsupported("unstructured jump at " + ip);
      }
      ip++;
    }
  }

  static IfShape ifShape(List<HbcProgram.Instruction> code, int conditionalIp, int end) {
    HbcProgram.Instruction conditional = code.get(conditionalIp);
    if (conditional.opcode() != HbcProgram.Opcode.JUMP_IF_FALSE) return null;
    int elseStart = index(conditional.first());
    if (elseStart <= conditionalIp || elseStart > end) return null;

    for (int ip = conditionalIp + 1; ip < elseStart; ip++) {
      HbcProgram.Instruction instruction = code.get(ip);
      if (instruction.opcode() != HbcProgram.Opcode.JUMP) continue;
      int mergeIp = index(instruction.first());
      if (mergeIp <= elseStart || mergeIp > end) continue;
      return new IfShape(conditionalIp, conditionalIp + 1, ip, elseStart, mergeIp, ip);
    }
    return new IfShape(conditionalIp, conditionalIp + 1, elseStart, elseStart, elseStart, -1);
  }

  static LoopShape loopShape(List<HbcProgram.Instruction> code, int loopStart, int end) {
    for (int conditionalIp = loopStart; conditionalIp < end; conditionalIp++) {
      HbcProgram.Instruction conditional = code.get(conditionalIp);
      if (conditional.opcode() != HbcProgram.Opcode.JUMP_IF_FALSE) {
        if (isControl(conditional.opcode())) return null;
        continue;
      }
      int exitIp = index(conditional.first());
      if (exitIp <= conditionalIp || exitIp > end) return null;
      if (conditionalIp == loopStart) {
        // A one-instruction condition is valid; there is no linear setup to skip.
      }
      for (int backJumpIp = conditionalIp + 1; backJumpIp < exitIp; backJumpIp++) {
        HbcProgram.Instruction backJump = code.get(backJumpIp);
        if (backJump.opcode() != HbcProgram.Opcode.JUMP) continue;
        if (index(backJump.first()) != loopStart || backJumpIp != exitIp - 1) continue;
        return new LoopShape(loopStart, conditionalIp, conditionalIp + 1, backJumpIp, exitIp);
      }
      return null;
    }
    return null;
  }

  private static boolean isControl(HbcProgram.Opcode opcode) {
    return opcode == HbcProgram.Opcode.JUMP
        || opcode == HbcProgram.Opcode.JUMP_IF_FALSE
        || opcode == HbcProgram.Opcode.RETURN
        || opcode == HbcProgram.Opcode.THROW
        || opcode == HbcProgram.Opcode.RETHROW;
  }

  private static int index(long value) {
    return Math.toIntExact(value);
  }

  private static void unsupported(String message) {
    throw new UnsupportedControlFlow(message);
  }

  record IfShape(
      int conditionalIp,
      int thenStart,
      int thenEnd,
      int elseStart,
      int mergeIp,
      int endJumpIp) {
    boolean hasElse() {
      return endJumpIp >= 0;
    }
  }

  record LoopShape(int loopStart, int conditionalIp, int bodyStart, int bodyEnd, int exitIp) {}

  private static final class UnsupportedControlFlow extends RuntimeException {
    UnsupportedControlFlow(String message) {
      super(message);
    }
  }
}
