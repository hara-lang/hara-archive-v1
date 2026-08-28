package hara.truffle.bytecode;

import com.oracle.truffle.api.RootCallTarget;
import com.oracle.truffle.api.CompilerDirectives.TruffleBoundary;
import com.oracle.truffle.api.bytecode.BytecodeConfig;
import com.oracle.truffle.api.bytecode.BytecodeLocal;
import com.oracle.truffle.api.bytecode.BytecodeRootNode;
import com.oracle.truffle.api.bytecode.BytecodeRootNodes;
import com.oracle.truffle.api.bytecode.ConstantOperand;
import com.oracle.truffle.api.bytecode.GenerateBytecode;
import com.oracle.truffle.api.bytecode.Operation;
import com.oracle.truffle.api.bytecode.Variadic;
import com.oracle.truffle.api.dsl.Specialization;
import com.oracle.truffle.api.frame.FrameDescriptor;
import com.oracle.truffle.api.nodes.RootNode;
import hara.truffle.HaraBox;
import hara.truffle.HaraContext;
import hara.truffle.HaraLanguage;
import hara.truffle.HbcMachine;
import hara.truffle.HbcPrimitiveRuntime;
import hara.truffle.HbcInstrumentationBridge;
import hara.lang.data.Symbol;

/** Truffle Bytecode DSL entry point for the portable HBC0 instruction set. */
@GenerateBytecode(
    languageClass = HaraLanguage.class,
    enableUncachedInterpreter = true,
    defaultUncachedThreshold = "16",
    enableQuickening = true,
    enableYield = true)
public abstract class HbcBytecodeRootNode extends RootNode implements BytecodeRootNode {
  protected HbcBytecodeRootNode(HaraLanguage language, FrameDescriptor frameDescriptor) {
    super(language, frameDescriptor);
  }

  public static RootCallTarget compile(HaraLanguage language, HbcProgram program) {
    HbcCodegenPlan plan = HbcCodegenPlan.analyze(program);
    HaraContext context = HaraLanguage.currentContext();
    if (!plan.entryEligible() || !context.hbcNativeExecutionAllowed()) {
      return compileFallback(language, program);
    }
    return compileNative(language, plan, program.entry());
  }

  /** Returns a native call target for an eligible HBC prototype, or {@code null} for fallback. */
  public static RootCallTarget compileFunction(
      HaraLanguage language, HbcProgram program, int functionIndex) {
    HbcCodegenPlan plan = HbcCodegenPlan.analyze(program);
    HaraContext context = HaraLanguage.currentContext();
    if (functionIndex < 0
        || functionIndex >= plan.functions().size()
        || !plan.functions().get(functionIndex).eligible()
        || !context.hbcNativeExecutionAllowed()) {
      return null;
    }
    return compileNative(language, plan, functionIndex);
  }

  private static RootCallTarget compileFallback(HaraLanguage language, HbcProgram program) {
    BytecodeRootNodes<HbcBytecodeRootNode> roots =
        HbcBytecodeRootNodeGen.create(
            language,
            BytecodeConfig.DEFAULT,
            builder -> {
              builder.beginRoot();
              builder.beginReturn();
              builder.emitExecute(program);
              builder.endReturn();
              builder.endRoot();
            });
    return roots.getNode(0).getCallTarget();
  }

  private static RootCallTarget compileNative(
      HaraLanguage language, HbcCodegenPlan plan, int functionIndex) {
    HbcProgram program = plan.program();
    HbcProgram.Function function = program.functions().get(functionIndex);
    HbcCodegenPlan.FunctionPlan functionPlan = plan.functions().get(functionIndex);
    int[] stackHeights = functionPlan.stackHeights();
    BytecodeRootNodes<HbcBytecodeRootNode> roots =
        HbcBytecodeRootNodeGen.create(
            language,
            BytecodeConfig.DEFAULT,
            builder -> {
              builder.beginRoot();
              BytecodeLocal[] locals = new BytecodeLocal[function.localCount()];
              for (int i = 0; i < locals.length; i++) {
                locals[i] = builder.createLocal("local-" + i, null);
              }
              BytecodeLocal[] stack = new BytecodeLocal[function.maxStack()];
              for (int i = 0; i < stack.length; i++) {
                stack[i] = builder.createLocal("stack-" + i, null);
              }
              for (int i = 0; i < locals.length; i++) {
                int localIndex = i;
                store(
                    builder,
                    locals[i],
                    i < function.arity()
                        ? () -> builder.emitLoadArgument(localIndex)
                        : builder::emitLoadNull);
              }
              for (BytecodeLocal local : stack) {
                store(builder, local, builder::emitLoadNull);
              }
              builder.emitSetup(program);
              emitStructuredRange(
                  builder,
                  program,
                  function,
                  functionIndex,
                  0,
                  function.code().size(),
                  stackHeights,
                  locals,
                  stack);
              builder.endRoot();
            });
    return roots.getNode(0).getCallTarget();
  }

  private static void emitStructuredRange(
      HbcBytecodeRootNodeGen.Builder builder,
      HbcProgram program,
      HbcProgram.Function function,
      int functionIndex,
      int start,
      int end,
      int[] stackHeights,
      BytecodeLocal[] locals,
      BytecodeLocal[] stack) {
    int ip = start;
    while (ip < end) {
      HbcControlFlow.LoopShape loop = HbcControlFlow.loopShape(function.code(), ip, end);
      if (loop != null) {
        builder.beginWhile();
        builder.beginBlock();
        emitLinearRange(
            builder,
            program,
            function,
            functionIndex,
            loop.loopStart(),
            loop.conditionalIp(),
            stackHeights,
            locals,
            stack);
        emitProbe(builder, program, functionIndex, loop.conditionalIp());
        builder.emitLoadLocal(stack[stackHeights[loop.conditionalIp()] - 1]);
        builder.endBlock();
        builder.beginBlock();
        emitStructuredRange(
            builder,
            program,
            function,
            functionIndex,
            loop.bodyStart(),
            loop.bodyEnd(),
            stackHeights,
            locals,
            stack);
        emitProbe(builder, program, functionIndex, loop.exitIp() - 1);
        builder.endBlock();
        builder.endWhile();
        ip = loop.exitIp();
        continue;
      }

      HbcProgram.Instruction instruction = function.code().get(ip);
      if (instruction.opcode() == HbcProgram.Opcode.JUMP_IF_FALSE) {
        HbcControlFlow.IfShape branch = HbcControlFlow.ifShape(function.code(), ip, end);
        if (branch == null) {
          throw new IllegalStateException("unplanned conditional HBC shape at " + ip);
        }
        int condition = stackHeights[ip] - 1;
        if (branch.hasElse()) {
          builder.beginIfThenElse();
          builder.beginBlock();
          emitProbe(builder, program, functionIndex, branch.conditionalIp());
          emitTruthy(builder, stack[condition]);
          builder.endBlock();
          builder.beginBlock();
          emitStructuredRange(
              builder,
              program,
              function,
              functionIndex,
              branch.thenStart(),
              branch.thenEnd(),
              stackHeights,
              locals,
              stack);
          emitProbe(builder, program, functionIndex, branch.endJumpIp());
          builder.endBlock();
          builder.beginBlock();
          emitStructuredRange(
              builder,
              program,
              function,
              functionIndex,
              branch.elseStart(),
              branch.mergeIp(),
              stackHeights,
              locals,
              stack);
          builder.endBlock();
          builder.endIfThenElse();
        } else {
          builder.beginIfThen();
          builder.beginBlock();
          emitProbe(builder, program, functionIndex, branch.conditionalIp());
          emitTruthy(builder, stack[condition]);
          builder.endBlock();
          builder.beginBlock();
          emitStructuredRange(
              builder,
              program,
              function,
              functionIndex,
              branch.thenStart(),
              branch.thenEnd(),
              stackHeights,
              locals,
              stack);
          builder.endBlock();
          builder.endIfThen();
        }
        ip = branch.mergeIp();
        continue;
      }

      if (instruction.opcode() == HbcProgram.Opcode.JUMP) {
        throw new IllegalStateException("unplanned jump HBC shape at " + ip);
      }
      emitInstruction(
          builder, program, instruction, stackHeights[ip], locals, stack, functionIndex, ip);
      ip++;
    }
  }

  private static void emitLinearRange(
      HbcBytecodeRootNodeGen.Builder builder,
      HbcProgram program,
      HbcProgram.Function function,
      int functionIndex,
      int start,
      int end,
      int[] stackHeights,
      BytecodeLocal[] locals,
      BytecodeLocal[] stack) {
    for (int ip = start; ip < end; ip++) {
      HbcProgram.Instruction instruction = function.code().get(ip);
      if (instruction.opcode() == HbcProgram.Opcode.JUMP
          || instruction.opcode() == HbcProgram.Opcode.JUMP_IF_FALSE) {
        throw new IllegalStateException("control opcode in linear HBC range at " + ip);
      }
      emitInstruction(
          builder, program, instruction, stackHeights[ip], locals, stack, functionIndex, ip);
    }
  }

  private static void emitInstruction(
      HbcBytecodeRootNodeGen.Builder builder,
      HbcProgram program,
      HbcProgram.Instruction instruction,
      int height,
      BytecodeLocal[] locals,
      BytecodeLocal[] stack,
      int functionIndex,
      int ip) {
    emitProbe(builder, program, functionIndex, ip);
    switch (instruction.opcode()) {
      case CONSTANT ->
          store(
              builder,
              stack[height],
              () -> emitConstant(builder, program.constants().get(Math.toIntExact(instruction.first()))));
      case NIL -> store(builder, stack[height], builder::emitLoadNull);
      case TRUE -> store(builder, stack[height], () -> builder.emitLoadConstant(true));
      case FALSE -> store(builder, stack[height], () -> builder.emitLoadConstant(false));
      case LOAD_LOCAL ->
          store(
              builder,
              stack[height],
              () -> builder.emitLoadLocal(locals[Math.toIntExact(instruction.first())]));
      case STORE_LOCAL ->
          store(
              builder,
              locals[Math.toIntExact(instruction.first())],
              () -> builder.emitLoadLocal(stack[height - 1]));
      case POP -> {}
      case DUP ->
          store(builder, stack[height], () -> builder.emitLoadLocal(stack[height - 1]));
      case PRIMITIVE -> {
        HbcProgram.Primitive primitive =
            HbcProgram.Primitive.fromId(Math.toIntExact(instruction.first()));
        int argc = Math.toIntExact(instruction.second());
        int result = height - argc;
        storeResult(
            builder,
            stack[result],
            () -> {
              if (argc == 1) {
                builder.beginPrimitiveUnary(primitive);
                loadArguments(builder, stack, result, argc);
                builder.endPrimitiveUnary();
              } else {
                builder.beginPrimitiveBinary(primitive);
                loadArguments(builder, stack, result, argc);
                builder.endPrimitiveBinary();
              }
            });
      }
      case PRIMITIVE_LOCAL_CONST -> {
        HbcProgram.Primitive primitive =
            HbcProgram.Primitive.fromId(Math.toIntExact(instruction.first()));
        storeResult(
            builder,
            stack[height],
            () -> {
              builder.beginPrimitiveBinary(primitive);
              builder.emitLoadLocal(locals[Math.toIntExact(instruction.second())]);
              emitConstant(builder, program.constants().get(Math.toIntExact(instruction.third())));
              builder.endPrimitiveBinary();
            });
      }
      case CALL -> {
        int argc = Math.toIntExact(instruction.first());
        int result = height - argc - 1;
        storeResult(
            builder,
            stack[result],
            () -> {
              if (argc == 0) {
                builder.beginCall0();
                builder.emitLoadLocal(stack[result]);
                builder.endCall0();
              } else if (argc == 1) {
                builder.beginCall1();
                loadArguments(builder, stack, result, 2);
                builder.endCall1();
              } else {
                builder.beginCall2();
                loadArguments(builder, stack, result, 3);
                builder.endCall2();
              }
            });
      }
      case CALL_STATIC -> {
        int argc = Math.toIntExact(instruction.second());
        int result = height - argc;
        HbcStaticCall target =
            new HbcStaticCall(program, Math.toIntExact(instruction.first()));
        storeResult(
            builder,
            stack[result],
            () -> {
              if (argc == 0) {
                builder.emitStaticCall0(target);
              } else if (argc == 1) {
                builder.beginStaticCall1(target);
                loadArguments(builder, stack, result, argc);
                builder.endStaticCall1();
              } else {
                builder.beginStaticCall2(target);
                loadArguments(builder, stack, result, argc);
                builder.endStaticCall2();
              }
            });
      }
      case BUILD_VECTOR, BUILD_MAP, BUILD_SET, BUILD_LIST, CONCAT_LIST -> {
        int count = Math.toIntExact(instruction.first());
        int values = instruction.opcode() == HbcProgram.Opcode.BUILD_MAP ? count * 2 : count;
        int result = height - values;
        CollectionKind kind =
            switch (instruction.opcode()) {
              case BUILD_VECTOR -> CollectionKind.VECTOR;
              case BUILD_MAP -> CollectionKind.MAP;
              case BUILD_SET -> CollectionKind.SET;
              case BUILD_LIST -> CollectionKind.LIST;
              case CONCAT_LIST -> CollectionKind.CONCAT_LIST;
              default -> throw new AssertionError(instruction.opcode());
            };
        storeResult(
            builder,
            stack[result],
            () -> {
              builder.beginCollection(kind);
              loadArguments(builder, stack, result, values);
              builder.endCollection();
            });
      }
      case TO_VECTOR -> {
        int result = height - 1;
        storeResult(
            builder,
            stack[result],
            () -> {
              builder.beginCollection(CollectionKind.TO_VECTOR);
              builder.emitLoadLocal(stack[result]);
              builder.endCollection();
          });
      }
      case YIELD -> {
        int result = height - 1;
        storeResult(
            builder,
            stack[result],
            () -> {
              builder.beginYield();
              builder.beginRequireYield();
              builder.emitLoadLocal(stack[result]);
              builder.endRequireYield();
              builder.endYield();
            });
      }
      case JUMP, JUMP_IF_FALSE ->
          throw new IllegalStateException("structured control opcode emitted directly");
      case RETURN -> {
        builder.beginReturn();
        builder.beginExport();
        builder.beginTerminal(new HbcNativeInstruction(program, functionIndex, ip));
        builder.emitLoadLocal(stack[0]);
        builder.endTerminal();
        builder.endExport();
        builder.endReturn();
      }
      default -> throw new IllegalStateException("unplanned native HBC opcode " + instruction.opcode());
    }
  }

  private static void emitProbe(
      HbcBytecodeRootNodeGen.Builder builder, HbcProgram program, int functionIndex, int ip) {
    builder.emitInstructionProbe(new HbcNativeInstruction(program, functionIndex, ip));
  }

  private static void emitTruthy(
      HbcBytecodeRootNodeGen.Builder builder, BytecodeLocal value) {
    builder.beginTruthy();
    builder.emitLoadLocal(value);
    builder.endTruthy();
  }

  private static void loadArguments(
      HbcBytecodeRootNodeGen.Builder builder, BytecodeLocal[] stack, int start, int count) {
    for (int index = 0; index < count; index++) {
      builder.emitLoadLocal(stack[start + index]);
    }
  }

  private static void storeResult(
      HbcBytecodeRootNodeGen.Builder builder, BytecodeLocal local, Runnable computation) {
    builder.beginStoreLocal(local);
    computation.run();
    builder.endStoreLocal();
  }

  private static void store(
      HbcBytecodeRootNodeGen.Builder builder, BytecodeLocal local, Runnable value) {
    builder.beginStoreLocal(local);
    value.run();
    builder.endStoreLocal();
  }

  private static void emitConstant(HbcBytecodeRootNodeGen.Builder builder, Object value) {
    if (value == null) builder.emitLoadNull();
    else builder.emitLoadConstant(value);
  }

  enum CollectionKind {
    VECTOR,
    MAP,
    SET,
    LIST,
    CONCAT_LIST,
    TO_VECTOR
  }

  @Operation
  @ConstantOperand(type = HbcProgram.class)
  public static final class Execute {
    @Specialization
    @TruffleBoundary
    public static Object execute(HbcProgram program) {
      HaraContext context = HaraLanguage.currentContext();
      if (program.namespace() != null) {
        context.setCurrentNamespace(Symbol.create(program.namespace()));
      }
      context.installHbcTypes(
          program.schemaTypes(), program.functionTypes(), program.inferredFunctionTypes());
      return HbcMachine.execute(program, context);
    }
  }

  @Operation
  @ConstantOperand(type = HbcProgram.class)
  public static final class Setup {
    @Specialization
    @TruffleBoundary
    public static void execute(HbcProgram program) {
      HaraContext context = HaraLanguage.currentContext();
      if (program.namespace() != null) {
        context.setCurrentNamespace(Symbol.create(program.namespace()));
      }
      context.installHbcTypes(
          program.schemaTypes(), program.functionTypes(), program.inferredFunctionTypes());
    }
  }

  @Operation
  @ConstantOperand(type = HbcNativeInstruction.class)
  public static final class InstructionProbe {
    @Specialization
    @TruffleBoundary
    public static void execute(HbcNativeInstruction instruction) {
      HbcInstrumentationBridge.instruction(HaraLanguage.currentContext(), instruction);
    }
  }

  @Operation
  public static final class Truthy {
    @Specialization
    @TruffleBoundary
    public static boolean execute(Object value) {
      return !HaraBox.isNil(value) && !Boolean.FALSE.equals(HaraBox.unwrap(value));
    }
  }

  @Operation
  @ConstantOperand(type = HbcProgram.Primitive.class)
  public static final class PrimitiveUnary {
    @Specialization
    @TruffleBoundary
    public static Object execute(HbcProgram.Primitive primitive, Object value) {
      return HbcPrimitiveRuntime.invoke(
          HaraLanguage.currentContext(), primitive, new Object[] {value});
    }
  }

  @Operation
  @ConstantOperand(type = HbcProgram.Primitive.class)
  public static final class PrimitiveBinary {
    @Specialization
    @TruffleBoundary
    public static Object execute(HbcProgram.Primitive primitive, Object left, Object right) {
      return HbcPrimitiveRuntime.invoke(
          HaraLanguage.currentContext(), primitive, new Object[] {left, right});
    }
  }

  @Operation
  public static final class Call0 {
    @Specialization
    @TruffleBoundary
    public static Object execute(Object callable) {
      return HaraLanguage.currentContext().invokeCallable(callable, new Object[0]);
    }
  }

  @Operation
  public static final class Call1 {
    @Specialization
    @TruffleBoundary
    public static Object execute(Object callable, Object argument) {
      return HaraLanguage.currentContext().invokeCallable(callable, new Object[] {argument});
    }
  }

  @Operation
  public static final class Call2 {
    @Specialization
    @TruffleBoundary
    public static Object execute(Object callable, Object first, Object second) {
      return HaraLanguage.currentContext().invokeCallable(
          callable, new Object[] {first, second});
    }
  }

  @Operation
  @ConstantOperand(type = HbcStaticCall.class)
  public static final class StaticCall0 {
    @Specialization
    @TruffleBoundary
    public static Object execute(HbcStaticCall target) {
      return HbcPrimitiveRuntime.invokeStatic(
          target, HaraLanguage.currentContext(), new Object[0]);
    }
  }

  @Operation
  @ConstantOperand(type = HbcStaticCall.class)
  public static final class StaticCall1 {
    @Specialization
    @TruffleBoundary
    public static Object execute(HbcStaticCall target, Object argument) {
      return HbcPrimitiveRuntime.invokeStatic(
          target,
          HaraLanguage.currentContext(),
          new Object[] {argument});
    }
  }

  @Operation
  @ConstantOperand(type = HbcStaticCall.class)
  public static final class StaticCall2 {
    @Specialization
    @TruffleBoundary
    public static Object execute(HbcStaticCall target, Object first, Object second) {
      return HbcPrimitiveRuntime.invokeStatic(
          target,
          HaraLanguage.currentContext(),
          new Object[] {first, second});
    }
  }

  @Operation
  @ConstantOperand(type = HbcNativeInstruction.class)
  public static final class Terminal {
    @Specialization
    @TruffleBoundary
    public static Object execute(HbcNativeInstruction instruction, Object value) {
      return HbcInstrumentationBridge.terminal(
          HaraLanguage.currentContext(), instruction, value);
    }
  }

  @Operation
  public static final class Export {
    @Specialization
    @TruffleBoundary
    public static Object execute(Object value) {
      return HaraBox.export(value);
    }
  }

  @Operation
  public static final class RequireYield {
    @Specialization
    @TruffleBoundary
    public static Object execute(Object value) {
      hara.truffle.StdFoundationCoroutine.requireYieldContext();
      return value;
    }
  }

  @Operation
  @ConstantOperand(type = CollectionKind.class)
  public static final class Collection {
    @Specialization
    @TruffleBoundary
    public static Object execute(
        CollectionKind kind, @Variadic(startOffset = 1) Object[] values) {
      return switch (kind) {
        case VECTOR ->
            values.length <= 8
                ? hara.kernel.builtin.BuiltinStruct.tuple(values)
                : hara.lang.data.Vector.Standard.from(null, values);
        case MAP -> hara.lang.data.Map.Standard.from(null, values);
        case SET -> hara.lang.data.OrderedSet.Standard.from(null, values);
        case LIST -> hara.lang.data.List.Standard.from(null, values);
        case CONCAT_LIST ->
            HbcPrimitiveRuntime.concatList(HaraLanguage.currentContext(), values);
        case TO_VECTOR ->
            HbcPrimitiveRuntime.toVector(HaraLanguage.currentContext(), values[0]);
      };
    }
  }
}
