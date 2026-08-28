package hara.truffle.bytecode;

import hara.truffle.HbcMachine;
import java.util.ArrayList;
import java.util.List;

/**
 * Describes the small, deliberately conservative native tier for an HBC program.
 *
 * <p>The portable HBC representation remains the authority. This plan only decides whether a
 * function can be represented by the Truffle Bytecode DSL without changing its control-flow or
 * calling semantics. Functions outside this tier continue through {@link HbcMachine}.
 */
public final class HbcCodegenPlan {
  private final HbcProgram program;
  private final List<FunctionPlan> functions;

  private HbcCodegenPlan(HbcProgram program, List<FunctionPlan> functions) {
    this.program = program;
    this.functions = List.copyOf(functions);
  }

  public static HbcCodegenPlan analyze(HbcProgram program) {
    try {
      HbcValidator.validate(program);
    } catch (HbcFormatException error) {
      // Session.executeHbc also accepts in-memory programs. Keep that API's historical
      // portable-machine behavior for programs which have not gone through HbcCodec yet.
      return rejectedProgram(program, error.getMessage());
    }
    ArrayList<FunctionPlan> plans = new ArrayList<>(program.functions().size());
    for (int index = 0; index < program.functions().size(); index++) {
      plans.add(analyzeFunction(program, index));
    }
    rejectRecursiveStaticCalls(program, plans);
    boolean changed;
    do {
      changed = false;
      for (int index = 0; index < plans.size(); index++) {
        FunctionPlan plan = plans.get(index);
        if (!plan.eligible()) continue;
        for (HbcProgram.Instruction instruction : program.functions().get(index).code()) {
          if (instruction.opcode() != HbcProgram.Opcode.CALL_STATIC) continue;
          int target = Math.toIntExact(instruction.first());
          if (!plans.get(target).eligible()) {
            plans.set(index, reject(plan, "static call to ineligible function " + target));
            changed = true;
            break;
          }
        }
      }
    } while (changed);
    return new HbcCodegenPlan(program, plans);
  }

  private static void rejectRecursiveStaticCalls(
      HbcProgram program, ArrayList<FunctionPlan> plans) {
    int[] state = new int[plans.size()];
    ArrayList<Integer> path = new ArrayList<>();
    for (int index = 0; index < plans.size(); index++) {
      if (state[index] == 0) visitStaticCallGraph(program, plans, index, state, path);
    }
  }

  private static void visitStaticCallGraph(
      HbcProgram program,
      ArrayList<FunctionPlan> plans,
      int index,
      int[] state,
      ArrayList<Integer> path) {
    if (state[index] != 0 || !plans.get(index).eligible()) return;
    state[index] = 1;
    path.add(index);
    for (HbcProgram.Instruction instruction : program.functions().get(index).code()) {
      if (instruction.opcode() != HbcProgram.Opcode.CALL_STATIC) continue;
      int target = Math.toIntExact(instruction.first());
      if (!plans.get(target).eligible()) continue;
      if (state[target] == 0) {
        visitStaticCallGraph(program, plans, target, state, path);
      } else if (state[target] == 1) {
        int cycleStart = path.lastIndexOf(target);
        for (int cycleIndex = cycleStart; cycleIndex < path.size(); cycleIndex++) {
          int cycleFunction = path.get(cycleIndex);
          plans.set(
              cycleFunction,
              reject(plans.get(cycleFunction), "recursive static call requires portable machine"));
        }
      }
    }
    path.remove(path.size() - 1);
    state[index] = 2;
  }

  public HbcProgram program() {
    return program;
  }

  public List<FunctionPlan> functions() {
    return functions;
  }

  public FunctionPlan entry() {
    return functions.get(program.entry());
  }

  public boolean entryEligible() {
    return program.entry() >= 0
        && program.entry() < functions.size()
        && entry().eligible();
  }

  private static HbcCodegenPlan rejectedProgram(HbcProgram program, String reason) {
    ArrayList<FunctionPlan> plans = new ArrayList<>(program.functions().size());
    for (int index = 0; index < program.functions().size(); index++) {
      plans.add(rejected(index, new int[program.functions().get(index).code().size()], reason));
    }
    return new HbcCodegenPlan(program, plans);
  }

  private static FunctionPlan analyzeFunction(HbcProgram program, int index) {
    HbcProgram.Function function = program.functions().get(index);
    int[] stackHeights = HbcValidator.stackHeightsForCodegen(program, function);
    if (index == program.entry() && function.arity() != 0) {
      return rejected(index, stackHeights, "entry arguments are not materialized yet");
    }
    if (function.asyncFunction()) return rejected(index, stackHeights, "async function");
    if (function.variadic()) return rejected(index, stackHeights, "variadic function");
    if (function.captureCount() != 0) return rejected(index, stackHeights, "capturing function");
    if (!function.handlers().isEmpty()) return rejected(index, stackHeights, "exception handlers");

    String controlFlowReason = HbcControlFlow.validate(function);
    if (controlFlowReason != null) return rejected(index, stackHeights, controlFlowReason);

    for (int ip = 0; ip < function.code().size(); ip++) {
      HbcProgram.Instruction instruction = function.code().get(ip);
      String reason = unsupportedReason(instruction, ip, program);
      if (reason != null) return rejected(index, stackHeights, reason);
    }
    boolean continuationCapable =
        function.code().stream()
            .anyMatch(instruction -> instruction.opcode() == HbcProgram.Opcode.YIELD);
    return new FunctionPlan(index, true, null, stackHeights, continuationCapable);
  }

  private static String unsupportedReason(
      HbcProgram.Instruction instruction,
      int ip,
      HbcProgram program) {
    return switch (instruction.opcode()) {
      case CONSTANT ->
          program.constants().get(Math.toIntExact(instruction.first())) == null
              ? "null constant (use NIL)"
              : null;
      case NIL, TRUE, FALSE, LOAD_LOCAL, STORE_LOCAL, POP, DUP, RETURN, YIELD -> null;
      case PRIMITIVE -> {
        int argc = Math.toIntExact(instruction.second());
        HbcProgram.Primitive primitive = HbcProgram.Primitive.fromId(Math.toIntExact(instruction.first()));
        yield argc == 1 || argc == 2
            ? null
            : "primitive " + primitive + " with arity " + argc;
      }
      case PRIMITIVE_LOCAL_CONST -> null;
      case CALL ->
          Math.toIntExact(instruction.first()) <= 2
              ? null
              : "dynamic call with arity " + instruction.first();
      case CALL_STATIC ->
          Math.toIntExact(instruction.second()) <= 2
              ? null
              : "static call with arity " + instruction.second();
      case BUILD_VECTOR, BUILD_MAP, BUILD_SET, BUILD_LIST, CONCAT_LIST, TO_VECTOR -> null;
      case JUMP, JUMP_IF_FALSE -> null;
      default -> "opcode " + instruction.opcode();
    };
  }

  private static FunctionPlan rejected(int index, int[] stackHeights, String reason) {
    return new FunctionPlan(index, false, reason, stackHeights, false);
  }

  private static FunctionPlan reject(FunctionPlan plan, String reason) {
    return new FunctionPlan(plan.index(), false, reason, plan.stackHeights(), false);
  }

  public record FunctionPlan(
      int index, boolean eligible, String reason, int[] stackHeights, boolean continuationCapable) {
    public FunctionPlan {
      stackHeights = stackHeights.clone();
    }

    @Override
    public int[] stackHeights() {
      return stackHeights.clone();
    }
  }
}
