package hara.truffle;

import hara.truffle.bytecode.HbcProgram;

/** Scalar metadata for one authoritative HBC dispatch boundary. */
final class HbcBoundary {
  enum TransitionKind {
    CALL_ENTER,
    CALL_RETURN,
    EXCEPTION_UNWIND,
    MACHINE_SUSPEND,
    MACHINE_RESUME
  }

  enum TerminalKind {
    RETURN,
    FAIL
  }

  enum OutcomeKind {
    CONTINUE,
    SUSPENDED,
    YIELDED,
    RETURNED,
    FAILED,
    CANCELLED
  }

  record Instruction(
      int function,
      int instructionPointer,
      HbcProgram.Opcode opcode,
      int stackDepth,
      int callDepth) {}

  record Transition(
      TransitionKind kind,
      int fromFunction,
      int fromInstructionPointer,
      int toFunction,
      int toInstructionPointer,
      int stackDepth,
      int callDepth) {}

  record Terminal(
      TerminalKind kind,
      int function,
      int instructionPointer,
      int stackDepth,
      int callDepth) {}

  record Result(
      Instruction instruction,
      Transition transition,
      Terminal terminal,
      OutcomeKind outcome) {}

  private HbcBoundary() {}
}
