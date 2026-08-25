package hara.truffle.bytecode;

import hara.truffle.bytecode.HbcProgram.Function;
import hara.truffle.bytecode.HbcProgram.Instruction;

/** Stable, human-readable HBC0 disassembly for diagnostics and cross-runtime fixtures. */
public final class HbcDisassembler {
  private HbcDisassembler() {}

  public static String disassemble(HbcProgram program) {
    StringBuilder output = new StringBuilder("HBC0 entry=").append(program.entry()).append('\n');
    for (int constant = 0; constant < program.constants().size(); constant++) {
      output
          .append("const ")
          .append(constant)
          .append(' ')
          .append(String.valueOf(program.constants().get(constant)))
          .append('\n');
    }
    for (int functionIndex = 0; functionIndex < program.functions().size(); functionIndex++) {
      Function function = program.functions().get(functionIndex);
      output
          .append("fn ")
          .append(functionIndex)
          .append(' ')
          .append(function.name() == null ? "<anonymous>" : function.name())
          .append(" arity=")
          .append(function.arity())
          .append(function.variadic() ? "+" : "")
          .append(" captures=")
          .append(function.captureCount())
          .append(" locals=")
          .append(function.localCount())
          .append(" stack=")
          .append(function.maxStack())
          .append('\n');
      for (int ip = 0; ip < function.code().size(); ip++) {
        Instruction instruction = function.code().get(ip);
        output.append(String.format("  %04d  %-22s", ip, instruction.opcode()));
        int operands = operandCount(instruction.opcode());
        if (operands > 0) output.append(instruction.first());
        if (operands > 1) output.append(' ').append(instruction.second());
        if (operands > 2) output.append(' ').append(instruction.third());
        output.append('\n');
      }
    }
    return output.toString();
  }

  private static int operandCount(HbcProgram.Opcode opcode) {
    return switch (opcode) {
      case NIL, TRUE, FALSE, POP, THROW, RETHROW, RETURN, AWAIT, YIELD, HOST_CALL, DUP, INSTANCE_OF,
          TO_VECTOR -> 0;
      case CONSTANT, LOAD_LOCAL, STORE_LOCAL, JUMP, JUMP_IF_FALSE, CALL, GET_GLOBAL, SET_GLOBAL,
          VAR_GLOBAL, DECLARE_GLOBAL, MUTABLE_FIELD_GET, MUTABLE_FIELD_SET, BUILD_VECTOR, BUILD_MAP,
          BUILD_SET, BUILD_LIST, CONCAT_LIST, PRIMITIVE_VALUE, BUILTIN_VALUE, DYNAMIC_BIND, DYNAMIC_UNBIND,
          DEF_PROTOCOL, EXTEND_TYPE, DEF_MULTI, DEF_METHOD, INTRINSIC_VALUE -> 1;
      case PRIMITIVE, CLOSURE, CALL_STATIC, DEF_GLOBAL, DEF_STRUCT, DEF_MUTABLE,
          MAKE_MULTI_ARITY, DEF_MACRO, INTRINSIC_CALL, PROTOCOL_CALL -> 2;
      case DOT_CALL -> 2;
      case PRIMITIVE_LOCAL_CONST -> 3;
    };
  }
}
