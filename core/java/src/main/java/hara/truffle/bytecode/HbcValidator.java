package hara.truffle.bytecode;

import hara.lang.protocol.ILinearType;
import hara.truffle.HalcSchema;
import hara.truffle.bytecode.HbcProgram.Function;
import hara.truffle.bytecode.HbcProgram.Instruction;
import hara.truffle.bytecode.HbcProgram.Opcode;
import hara.truffle.bytecode.HbcProgram.TryEntry;
import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

/** Rust-compatible structural and abstract-stack validation for HBC0. */
public final class HbcValidator {
  public static final int MAX_CONSTANTS = 1 << 24;
  public static final int MAX_INSTRUCTIONS = 1 << 24;
  public static final int MAX_OPERAND_STACK = 4096;

  private HbcValidator() {}

  public static void validate(HbcProgram program) {
    if (program.constants().size() > MAX_CONSTANTS) fail("constant pool exceeds limit of " + MAX_CONSTANTS);
    if (program.functions().isEmpty()) fail("program has no functions");
    if (program.entry() < 0 || program.entry() >= program.functions().size()) {
      fail("entry function index out of range");
    }
    boolean multiple = program.functions().size() > 1;
    for (int index = 0; index < program.functions().size(); index++) {
      try {
        validateDeclaredArity(program, index);
        validateFunction(program, program.functions().get(index));
      } catch (HbcFormatException error) {
        if (!multiple) throw error;
        throw new HbcFormatException("validation failed: function " + index + ": " + detail(error));
      }
    }
  }

  private static void validateDeclaredArity(HbcProgram program, int index) {
    Function function = program.functions().get(index);
    if (!(program.functionSchema(index) instanceof HalcSchema.FunctionType schema)) return;
    boolean compatible =
        schema.arities().stream()
            .anyMatch(
                arity ->
                    arity.fixed().size() == function.arity()
                        && (arity.rest() != null) == function.variadic());
    if (!compatible) {
      fail(
          "function schema for "
              + function.name()
              + " has no "
              + function.arity()
              + "-argument arity"
              + (function.variadic() ? " with rest arguments" : ""));
    }
  }

  private static void validateFunction(HbcProgram program, Function function) {
    if (function.sourceMap().size() != function.code().size()) {
      fail("source map length does not match code length");
    }
    int[] heights = stackHeights(program, function);
    int computed = 0;
    for (int height : heights) computed = Math.max(computed, height);
    if (computed != function.maxStack()) {
      fail("declared max_stack " + function.maxStack() + " disagrees with computed " + computed);
    }
    validateHandlers(function, heights);
  }

  static int[] stackHeightsForCodegen(HbcProgram program, Function function) {
    return stackHeights(program, function);
  }

  private static int[] stackHeights(HbcProgram program, Function function) {
    List<Instruction> code = function.code();
    if (code.isEmpty()) fail("function has no code");
    if (code.size() > MAX_INSTRUCTIONS) fail("code exceeds limit of " + MAX_INSTRUCTIONS + " instructions");
    int[] heights = new int[code.size()];
    java.util.Arrays.fill(heights, -1);
    ArrayDeque<State> work = new ArrayDeque<>();
    work.push(new State(0, 0));
    Map<Integer, List<TryEntry>> handlerStarts = new HashMap<>();
    for (TryEntry handler : function.handlers()) {
      if (handler.start() <= Integer.MAX_VALUE) {
        handlerStarts.computeIfAbsent((int) handler.start(), ignored -> new ArrayList<>()).add(handler);
      }
    }

    while (!work.isEmpty()) {
      State state = work.pop();
      if (state.ip < 0 || state.ip >= code.size()) failAt("instruction pointer out of range", state.ip);
      int existing = heights[state.ip];
      if (existing >= 0) {
        if (existing != state.height) {
          failAt("inconsistent stack heights " + existing + " and " + state.height + " at join", state.ip);
        }
        continue;
      }
      heights[state.ip] = state.height;

      for (TryEntry handler : handlerStarts.getOrDefault(state.ip, List.of())) {
        for (HbcProgram.CatchEntry clause : handler.catches()) {
          work.push(new State(index(clause.target(), code.size(), "catch target"), state.height));
        }
        if (handler.finallyTarget() != null) {
          work.push(new State(index(handler.finallyTarget(), code.size(), "finally target"), state.height));
        }
      }

      Instruction instruction = code.get(state.ip);
      validateOperands(program, function, instruction, state.ip);
      if (instruction.opcode() == Opcode.RETURN) {
        if (state.height != 1) failAt("return with stack height " + state.height + ", expected 1", state.ip);
        continue;
      }
      if (instruction.opcode() == Opcode.THROW || instruction.opcode() == Opcode.RETHROW) {
        if (state.height < 1) failAt("stack underflow", state.ip);
        continue;
      }
      int nextHeight = state.height + stackEffect(instruction);
      if (nextHeight < 0) failAt("stack underflow", state.ip);
      if (nextHeight > MAX_OPERAND_STACK) {
        failAt("operand stack exceeds limit of " + MAX_OPERAND_STACK, state.ip);
      }
      if (instruction.opcode() == Opcode.JUMP) {
        work.push(new State(index(instruction.first(), code.size(), "jump target"), nextHeight));
      } else if (instruction.opcode() == Opcode.JUMP_IF_FALSE) {
        work.push(new State(index(instruction.first(), code.size(), "jump target"), nextHeight));
        pushFallthrough(code, state.ip, nextHeight, work);
      } else {
        pushFallthrough(code, state.ip, nextHeight, work);
      }
    }

    for (int ip = 0; ip < heights.length; ip++) if (heights[ip] < 0) failAt("unreachable instruction", ip);
    return heights;
  }

  private static void validateOperands(
      HbcProgram program, Function function, Instruction instruction, int ip) {
    Opcode opcode = instruction.opcode();
    switch (opcode) {
      case CONSTANT -> constant(program, instruction.first(), ip);
      case LOAD_LOCAL, STORE_LOCAL -> local(function, instruction.first(), ip);
      case PRIMITIVE -> {
        HbcProgram.Primitive.fromId(Math.toIntExact(instruction.first()));
        unsigned(instruction.second(), 0xff, "primitive argc", ip);
      }
      case PRIMITIVE_LOCAL_CONST -> {
        HbcProgram.Primitive.fromId(Math.toIntExact(instruction.first()));
        local(function, instruction.second(), ip);
        constant(program, instruction.third(), ip);
      }
      case JUMP, JUMP_IF_FALSE -> index(instruction.first(), function.code().size(), "jump target");
      case CLOSURE -> {
        Function target = prototype(program, instruction.first(), "closure prototype", ip);
        if (instruction.second() != target.captureCount()) {
          failAt("closure captures " + instruction.second() + " but prototype expects " + target.captureCount(), ip);
        }
      }
      case CALL -> unsigned(instruction.first(), 0xff, "call argc", ip);
      case CALL_STATIC -> {
        Function target = prototype(program, instruction.first(), "callstatic target", ip);
        long argc = instruction.second();
        if ((!target.variadic() && argc != target.arity()) || (target.variadic() && argc < target.arity())) {
          failAt("callstatic argc " + argc + " but prototype expects " + target.arity(), ip);
        }
        if (target.captureCount() != function.captureCount()) {
          failAt("callstatic capture count differs from current function", ip);
        }
      }
      case GET_GLOBAL, SET_GLOBAL, VAR_GLOBAL, DECLARE_GLOBAL, MUTABLE_FIELD_GET,
          MUTABLE_FIELD_SET ->
          stringConstant(program, instruction.first(), ip);
      case BUILTIN_VALUE, DYNAMIC_BIND, DYNAMIC_UNBIND ->
          stringConstant(program, instruction.first(), ip);
      case PRIMITIVE_VALUE ->
          HbcProgram.Primitive.fromId(Math.toIntExact(instruction.first()));
      case INTRINSIC_CALL, INTRINSIC_VALUE, PROTOCOL_CALL -> {
        stringConstant(program, instruction.first(), ip);
        if (opcode != Opcode.INTRINSIC_VALUE) {
          unsigned(instruction.second(), 0xff, "intrinsic/protocol call argc", ip);
        }
      }
      case DEF_PROTOCOL, EXTEND_TYPE, DEF_MULTI, DEF_METHOD ->
          constant(program, instruction.first(), ip);
      case DOT_CALL -> {
        stringConstant(program, instruction.first(), ip);
        unsigned(instruction.second(), 0xff, "dot-call argc", ip);
      }
      case DEF_GLOBAL, DEF_MACRO -> {
        stringConstant(program, instruction.first(), ip);
        if (instruction.second() >= 0 && instruction.second() >= program.varMetadata().size()) {
          failAt("var metadata index " + instruction.second() + " out of range", ip);
        }
      }
      case DEF_STRUCT, DEF_MUTABLE -> {
        String kind = opcode == Opcode.DEF_MUTABLE ? "defmutable" : "defstruct";
        stringConstant(program, instruction.first(), ip);
        Object fields = constant(program, instruction.second(), ip);
        if (!(fields instanceof ILinearType<?>)) {
          failAt(kind + " fields constant " + instruction.second() + " is not a string vector", ip);
        }
        ILinearType<?> values = (ILinearType<?>) fields;
        for (Object value : values) {
          if (!(value instanceof String)) {
            failAt(kind + " fields constant " + instruction.second() + " is not a string vector", ip);
          }
        }
      }
      case MAKE_MULTI_ARITY -> stringConstant(program, instruction.first(), ip);
      default -> {}
    }
  }

  private static void validateHandlers(Function function, int[] heights) {
    int codeSize = function.code().size();
    for (int index = 0; index < function.handlers().size(); index++) {
      TryEntry handler = function.handlers().get(index);
      int start = index(handler.start(), codeSize, "try start");
      if (handler.end() <= handler.start() || handler.end() > codeSize) {
        failAt("try range [" + handler.start() + ", " + handler.end() + ") out of bounds or empty", start);
      }
      if (heights[start] != handler.depth()) {
        failAt("handler depth " + handler.depth() + " disagrees with computed " + heights[start], start);
      }
      for (HbcProgram.CatchEntry clause : handler.catches()) {
        index(clause.target(), codeSize, "catch target");
        if (clause.binding() >= function.localCount()) failAt("catch binding slot " + clause.binding() + " out of range", start);
      }
      boolean hasFinally = handler.finallyTarget() != null;
      if (hasFinally != (handler.pendingValue() != null) || hasFinally != (handler.pendingError() != null)) {
        failAt("pending slots must be present exactly when finally is present", start);
      }
      if (hasFinally) {
        index(handler.finallyTarget(), codeSize, "finally target");
        if (handler.pendingValue() >= function.localCount() || handler.pendingError() >= function.localCount()) {
          failAt("pending slot out of range", start);
        }
      }
      for (TryEntry other : function.handlers().subList(index + 1, function.handlers().size())) {
        boolean disjoint = handler.end() <= other.start() || other.end() <= handler.start();
        boolean nested =
            (handler.start() <= other.start() && other.end() <= handler.end())
                || (other.start() <= handler.start() && handler.end() <= other.end());
        if (!disjoint && !nested) failAt("try ranges must not partially overlap", start);
      }
    }
  }

  private static int stackEffect(Instruction instruction) {
    return switch (instruction.opcode()) {
      case CONSTANT, NIL, TRUE, FALSE, LOAD_LOCAL, DUP, PRIMITIVE_LOCAL_CONST,
          GET_GLOBAL, VAR_GLOBAL, DECLARE_GLOBAL, DEF_STRUCT, DEF_MUTABLE, PRIMITIVE_VALUE,
          BUILTIN_VALUE, DYNAMIC_UNBIND, DEF_PROTOCOL, EXTEND_TYPE, DEF_MULTI, DEF_METHOD,
          INTRINSIC_VALUE -> 1;
      case STORE_LOCAL, POP, JUMP_IF_FALSE -> -1;
      case PRIMITIVE, CALL_STATIC -> 1 - Math.toIntExact(instruction.second());
      case INTRINSIC_CALL, PROTOCOL_CALL -> 1 - Math.toIntExact(instruction.second());
      case CLOSURE -> 1 - Math.toIntExact(instruction.second());
      case CALL -> -Math.toIntExact(instruction.first());
      case DEF_GLOBAL, SET_GLOBAL, MUTABLE_FIELD_GET, DEF_MACRO, AWAIT, YIELD, JUMP, TO_VECTOR,
          DYNAMIC_BIND -> 0;
      case MUTABLE_FIELD_SET -> -1;
      case INSTANCE_OF -> -1;
      case MAKE_MULTI_ARITY -> 1 - Math.toIntExact(instruction.second());
      case BUILD_VECTOR, BUILD_SET, BUILD_LIST, CONCAT_LIST -> 1 - Math.toIntExact(instruction.first());
      case BUILD_MAP -> 1 - (2 * Math.toIntExact(instruction.first()));
      case HOST_CALL -> -2;
      case DOT_CALL -> -Math.toIntExact(instruction.second());
      case RETURN, THROW, RETHROW -> throw new AssertionError("terminal instruction has no stack effect");
    };
  }

  private static void pushFallthrough(
      List<Instruction> code, int ip, int height, ArrayDeque<State> work) {
    if (ip + 1 == code.size()) failAt("missing return: control falls off the end of the function", ip);
    work.push(new State(ip + 1, height));
  }

  private static Function prototype(HbcProgram program, long raw, String label, int ip) {
    if (raw < 0 || raw >= program.functions().size()) failAt(label + " " + raw + " out of range", ip);
    return program.functions().get((int) raw);
  }

  private static Object constant(HbcProgram program, long raw, int ip) {
    if (raw < 0 || raw >= program.constants().size()) failAt("constant index " + raw + " out of range", ip);
    return program.constants().get((int) raw);
  }

  private static void stringConstant(HbcProgram program, long raw, int ip) {
    Object value = constant(program, raw, ip);
    if (!(value instanceof String)) failAt("global name constant " + raw + " is not a string", ip);
  }

  private static void local(Function function, long raw, int ip) {
    if (raw < 0 || raw >= function.localCount()) failAt("local slot " + raw + " out of range", ip);
  }

  private static int index(long raw, int size, String label) {
    if (raw < 0 || raw >= size) fail(label + " " + raw + " out of range");
    return (int) raw;
  }

  private static void unsigned(long value, long maximum, String label, int ip) {
    if (value < 0 || value > maximum) failAt(label + " out of range", ip);
  }

  private static String detail(HbcFormatException error) {
    return error.getMessage().replaceFirst("^validation failed(?:: at [0-9]{4})?: ", "");
  }

  private static void fail(String message) {
    throw new HbcFormatException("validation failed: " + message);
  }

  private static void failAt(String message, long ip) {
    throw new HbcFormatException("validation failed at " + String.format("%04d", ip) + ": " + message);
  }

  private record State(int ip, int height) {}
}
