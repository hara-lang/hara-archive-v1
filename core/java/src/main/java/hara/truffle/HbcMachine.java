package hara.truffle;

import com.oracle.truffle.api.CompilerDirectives.TruffleBoundary;
import com.oracle.truffle.api.RootCallTarget;
import com.oracle.truffle.api.interop.TruffleObject;
import com.oracle.truffle.api.library.ExportLibrary;
import com.oracle.truffle.api.library.ExportMessage;
import com.oracle.truffle.api.interop.InteropLibrary;
import hara.lang.data.Symbol;
import hara.lang.data.Keyword;
import hara.lang.data.TaggedLiteral;
import hara.lang.base.primitive.Num;
import hara.kernel.builtin.BuiltinStruct;
import hara.lang.protocol.IMetadata;
import hara.lang.protocol.IExInfo;
import hara.lang.protocol.ILinearType;
import hara.lang.protocol.IMapType;
import hara.truffle.bytecode.HbcFormatException;
import hara.truffle.bytecode.HbcBytecodeRootNode;
import hara.truffle.bytecode.HbcProgram;
import hara.truffle.bytecode.HbcProgram.Function;
import hara.truffle.bytecode.HbcProgram.Instruction;
import hara.truffle.bytecode.HbcProgram.TryEntry;
import java.util.ArrayList;
import java.util.ArrayDeque;
import java.util.Arrays;
import java.util.HashSet;
import java.util.Iterator;
import java.util.Map;
import java.util.Set;

/** Executes a validated portable HBC0 program using the ordinary Hara runtime boundaries. */
public final class HbcMachine {
  private HbcMachine() {}

  public static Object execute(HbcProgram program, HaraContext context) {
    return context.withDeclarationTransaction(() -> executeTransactional(program, context));
  }

  private static Object executeTransactional(HbcProgram program, HaraContext context) {
    HbcContinuation continuation = context.hbcContinuation(program);
    if (continuation != null
        && continuation.pendingAwait == null
        && context.hbcInstrumentationEnabled(InstrumentationModel.EventKind.MACHINE_RESUME)) {
      HbcBoundary.Transition resume =
          new HbcBoundary.Transition(
              HbcBoundary.TransitionKind.MACHINE_RESUME,
              continuation.functionIndex,
              continuation.instructionPointer,
              continuation.functionIndex,
              continuation.instructionPointer,
              continuation.stack == null ? 0 : continuation.stack.size(),
              continuation.calls == null ? 0 : continuation.calls.size());
      HbcInstrumentationBridge.machineEvent(
          context,
          InstrumentationModel.EventKind.MACHINE_RESUME,
          program,
          continuation.functionIndex,
          continuation.function,
          continuation.instructionPointer,
          HbcInstrumentationBridge.transitionData(resume),
          continuation.locals,
          continuation.stack,
          new ArrayList<>(continuation.calls));
    }
    if (continuation == null) continuation = new HbcContinuation(program);
    try {
      Object result =
          call(
              program,
              context,
              continuation.functionIndex,
              continuation.arguments,
              continuation.captures,
              continuation);
      context.clearHbcContinuation(continuation);
      return HaraBox.export(result);
    } catch (HbcSuspended suspended) {
      context.retainHbcContinuation(suspended.continuation);
      return suspended.identity;
    } catch (RuntimeException failure) {
      context.clearHbcContinuation(continuation);
      throw failure;
    }
  }

  private static Object call(
      HbcProgram program, HaraContext context, int functionIndex, Object[] arguments, Object[] captures) {
    return call(
        program,
        context,
        functionIndex,
        arguments,
        captures,
        new HbcContinuation(program, functionIndex, arguments, captures));
  }

  /** Executes a static HBC function at a generated call boundary without exporting its result. */
  public static Object invokeFunction(
      HbcProgram program, HaraContext context, int functionIndex, Object[] arguments) {
    return call(program, context, functionIndex, arguments, new Object[0]);
  }

  private static Object call(
      HbcProgram program,
      HaraContext context,
      int functionIndex,
      Object[] arguments,
      Object[] captures,
      HbcContinuation continuation) {
    continuation.context = context;
    Function function = program.functions().get(functionIndex);
    Object[] locals = bindLocals(function, arguments, captures);
    ArrayList<Object> stack = new ArrayList<>(function.maxStack());
    ArrayDeque<CallFrame> calls = new ArrayDeque<>();
    int ip = 0;
    boolean stepAfterInstruction = false;
    if (continuation.initialized) {
      functionIndex = continuation.functionIndex;
      function = continuation.function;
      locals = continuation.locals;
      stack = continuation.stack;
      calls = continuation.calls;
      ip = continuation.instructionPointer;
      stepAfterInstruction = continuation.stepAfterInstruction;
    }
    if (continuation.initialized && continuation.pendingAwait != null) {
      if (context.hbcPromisePending(continuation.pendingAwait)) {
        throw suspend(
            continuation,
            program,
            functionIndex,
            function,
            locals,
            stack,
            calls,
            ip,
            false,
            false);
      }
      Object resumed = context.hbcPromiseValue(continuation.pendingAwait);
      continuation.pendingAwait = null;
      if (stack.isEmpty()) throw new HaraException("HBC await continuation stack underflow");
      pop(stack);
      HbcBoundary.Transition resume =
          new HbcBoundary.Transition(
              HbcBoundary.TransitionKind.MACHINE_RESUME,
              functionIndex,
              ip,
              functionIndex,
              ip + 1,
              stack.size(),
              calls.size());
      HbcInstrumentationBridge.machineEvent(
          context,
          InstrumentationModel.EventKind.MACHINE_RESUME,
          program,
          functionIndex,
          function,
          ip,
          HbcInstrumentationBridge.transitionData(resume),
          locals,
          stack,
          new ArrayList<>(calls));
      stack.add(resumed);
      ip++;
    }
    while (true) {
      if (stepAfterInstruction) {
        throw suspend(
            continuation,
            program,
            functionIndex,
            function,
            locals,
            stack,
            calls,
            ip,
            true,
            false);
      }
      Instruction instruction = function.code().get(ip);
      InstrumentationModel.InstrumentDirective directive = context.pollHbcDirective();
      if (continuation.initialized && continuation.paused) {
        if (directive == null) {
          throw suspend(
              continuation,
              program,
              functionIndex,
              function,
              locals,
              stack,
              calls,
              ip,
              true,
              false);
        }
        if (directive == InstrumentationModel.InstrumentDirective.CONTINUE) {
          continuation.paused = false;
        } else if (directive == InstrumentationModel.InstrumentDirective.STEP_NEXT) {
          continuation.paused = false;
          stepAfterInstruction = true;
        } else if (directive == InstrumentationModel.InstrumentDirective.SUSPEND) {
          throw suspend(
              continuation,
              program,
              functionIndex,
              function,
              locals,
              stack,
              calls,
              ip,
              true,
              false);
        } else if (directive == InstrumentationModel.InstrumentDirective.TERMINATE) {
          terminate(context, program, functionIndex, function, ip, locals, stack, calls);
        }
      }
      if (directive == InstrumentationModel.InstrumentDirective.SUSPEND) {
        throw suspend(
            continuation,
            program,
            functionIndex,
            function,
            locals,
            stack,
            calls,
            ip,
            true,
            false);
      }
      if (directive == InstrumentationModel.InstrumentDirective.STEP_NEXT) {
        stepAfterInstruction = true;
      }
      if (directive == InstrumentationModel.InstrumentDirective.TERMINATE) {
        terminate(context, program, functionIndex, function, ip, locals, stack, calls);
      }
      HbcInstrumentationBridge.machineEvent(
          context,
          InstrumentationModel.EventKind.INSTRUCTION_EXECUTE,
          program,
          functionIndex,
          function,
          ip,
          HbcInstrumentationBridge.instructionData(
              instruction.opcode(), stack.size(), calls.size()),
          locals,
          stack,
          new ArrayList<>(calls));
      try {
        switch (instruction.opcode()) {
        case CONSTANT -> stack.add(program.constants().get(index(instruction.first())));
        case NIL -> stack.add(null);
        case TRUE -> stack.add(true);
        case FALSE -> stack.add(false);
        case LOAD_LOCAL -> stack.add(locals[index(instruction.first())]);
        case STORE_LOCAL -> locals[index(instruction.first())] = pop(stack);
        case POP -> pop(stack);
        case DUP -> stack.add(peek(stack));
        case PRIMITIVE -> {
          Object[] args = popArguments(stack, index(instruction.second()));
          stack.add(invokePrimitive(context, index(instruction.first()), args));
        }
        case PRIMITIVE_LOCAL_CONST ->
            stack.add(
                invokePrimitive(
                    context,
                    index(instruction.first()),
                    new Object[] {
                      locals[index(instruction.second())],
                      program.constants().get(index(instruction.third()))
                    }));
        case PRIMITIVE_VALUE -> {
          int primitive = index(instruction.first());
          stack.add(new HbcNativeCallable(args -> invokePrimitive(context, primitive, args)));
        }
        case INTRINSIC_VALUE -> {
          String name = stringConstant(program, instruction.first());
          Integer primitive = primitiveId(name);
          if (primitive == null) {
            throw new HaraException("Unknown intrinsic value: " + name);
          }
          stack.add(new HbcNativeCallable(args -> invokePrimitive(context, primitive, args)));
        }
        case INTRINSIC_CALL, PROTOCOL_CALL -> {
          String target = stringConstant(program, instruction.first());
          int argumentCount = index(instruction.second());
          if (instruction.opcode() == HbcProgram.Opcode.PROTOCOL_CALL
              && argumentCount == 1
              && isPendingDerefTarget(target)
              && context.hbcPromisePending(peek(stack))) {
            continuation.pendingAwait = peek(stack);
            throw suspend(
                continuation,
                program,
                functionIndex,
                function,
                locals,
                stack,
                calls,
                ip,
                false,
                true);
          }
          Object[] callArguments = popArguments(stack, argumentCount);
          Integer primitive = primitiveId(target);
          stack.add(
              primitive == null
                  ? HbcPrimitiveRuntime.invokeTarget(
                      context, target, callArguments, HaraTargetRuntime.ResultMode.HANDLE)
                  : invokePrimitive(context, primitive, callArguments));
        }
        case BUILTIN_VALUE -> {
          String name = stringConstant(program, instruction.first());
          Integer primitive = primitiveId(name);
          stack.add(
              primitive == null
                  ? resolve(context, name).deref()
                  : new HbcNativeCallable(args -> invokePrimitive(context, primitive, args)));
        }
        case DYNAMIC_BIND -> {
          HaraVar variable = resolve(context, stringConstant(program, instruction.first()));
          if (!variable.isDynamic()) throw new HaraException("binding requires a dynamic Var");
          variable.bind(pop(stack));
          stack.add(null);
        }
        case DYNAMIC_UNBIND -> {
          resolve(context, stringConstant(program, instruction.first())).unbind();
          stack.add(null);
        }
        case JUMP -> {
          ip = index(instruction.first());
          continue;
        }
        case JUMP_IF_FALSE -> {
          Object condition = pop(stack);
          if (!truthy(condition)) {
            ip = index(instruction.first());
            continue;
          }
        }
        case CLOSURE -> {
          Object[] closed = popArguments(stack, index(instruction.second()));
          stack.add(new HbcClosure(program, context, index(instruction.first()), closed));
        }
        case CALL -> {
          Object[] args = popArguments(stack, index(instruction.first()));
          Object callee = HaraBox.unwrap(pop(stack));
          HbcClosure closure = selectClosure(callee, args.length);
          int targetFunctionIndex =
              closure != null
                      && closure.program == program
                      && closure.context == context
                      && !program.functions().get(closure.prototype).asyncFunction()
                  ? closure.prototype
                  : -1;
          HbcBoundary.Transition callEnter =
              new HbcBoundary.Transition(
                  HbcBoundary.TransitionKind.CALL_ENTER,
                  functionIndex,
                  ip,
                  targetFunctionIndex,
                  0,
                  stack.size(),
                  calls.size());
          HbcInstrumentationBridge.machineEvent(
              context,
              InstrumentationModel.EventKind.CALL_ENTER,
              program,
              functionIndex,
              function,
              ip,
              HbcInstrumentationBridge.callData(callEnter),
              locals,
              stack,
              new ArrayList<>(calls));
          if (closure != null
              && closure.program == program
              && closure.context == context
              && !program.functions().get(closure.prototype).asyncFunction()) {
            calls.push(new CallFrame(functionIndex, function, locals, stack, ip + 1));
            functionIndex = closure.prototype;
            function = program.functions().get(functionIndex);
            locals = bindLocals(function, args, closure.captures);
            stack = new ArrayList<>(function.maxStack());
            ip = 0;
            continue;
          }
          try {
            stack.add(
                context.invokeCallable(
                    callee,
                    args,
                    new hara.lang.base.Ex.Info.Site(
                        program.namespace(),
                        null,
                        sourcePosition(function, ip).line(),
                        sourcePosition(function, ip).column())));
          } catch (RuntimeException failure) {
            if (Boolean.getBoolean("hara.hbc.trace")) {
              System.err.println(
                  "HBC call failure "
                      + (program.namespace() == null ? "<anonymous>" : program.namespace())
                      + "/"
                      + (function.name() == null ? "<entry>" : function.name())
                      + " ip="
                      + ip
                      + " callee="
                      + (callee == null ? "nil" : callee.getClass().getName())
                      + ": "
                      + failure.getMessage());
              if (Boolean.getBoolean("hara.hbc.trace.stack")) {
                failure.printStackTrace(System.err);
              }
            }
            throw failure;
          }
        }
        case CALL_STATIC -> {
          Object[] args = popArguments(stack, index(instruction.second()));
          Function target = program.functions().get(index(instruction.first()));
          HbcBoundary.Transition callEnter =
              new HbcBoundary.Transition(
                  HbcBoundary.TransitionKind.CALL_ENTER,
                  functionIndex,
                  ip,
                  index(instruction.first()),
                  0,
                  stack.size(),
                  calls.size());
          HbcInstrumentationBridge.machineEvent(
              context,
              InstrumentationModel.EventKind.CALL_ENTER,
              program,
              functionIndex,
              function,
              ip,
              HbcInstrumentationBridge.callData(callEnter),
              locals,
              stack,
              new ArrayList<>(calls));
          int currentCaptureBase = function.arity() + (function.variadic() ? 1 : 0);
          Object[] inherited =
              Arrays.copyOfRange(
                  locals,
                  currentCaptureBase,
                  Math.min(locals.length, currentCaptureBase + target.captureCount()));
          if (target.asyncFunction()) {
            int targetIndex = index(instruction.first());
            stack.add(context.hbcAsync(() -> call(program, context, targetIndex, args, inherited)));
          } else {
            calls.push(new CallFrame(functionIndex, function, locals, stack, ip + 1));
            functionIndex = index(instruction.first());
            function = target;
            locals = bindLocals(function, args, inherited);
            stack = new ArrayList<>(function.maxStack());
            ip = 0;
            continue;
          }
        }
        case GET_GLOBAL -> {
          String name = stringConstant(program, instruction.first());
          Integer primitive = primitiveId(name);
          Object value =
              primitive == null
                  ? resolve(context, name).deref()
                  : new HbcNativeCallable(args -> invokePrimitive(context, primitive, args));
          stack.add(value);
        }
        case DEF_GLOBAL -> {
          Object value = pop(stack);
          Symbol symbol = Symbol.create(stringConstant(program, instruction.first()));
          IMetadata metadata = metadata(program, instruction.second());
          if (metadata != null) symbol = symbol.withMeta(metadata);
          // HBC0 follows HAL `def`: the expression returns the newly interned
          // Var, not its root value.  Rust's VM uses the same contract and the
          // portable conformance corpus observes its printed `#'ns/name` form.
          stack.add(context.define(symbol, value));
        }
        case SET_GLOBAL -> {
          Object value = pop(stack);
          resolve(context, stringConstant(program, instruction.first())).reset(value);
          stack.add(value);
        }
        case VAR_GLOBAL -> stack.add(resolve(context, stringConstant(program, instruction.first())));
        case DECLARE_GLOBAL -> {
          context.declareCurrent(Symbol.create(stringConstant(program, instruction.first())));
          stack.add(null);
        }
        case DEF_STRUCT, DEF_MUTABLE -> {
          String name = stringConstant(program, instruction.first());
          Object fieldsValue = program.constants().get(index(instruction.second()));
          if (!(fieldsValue instanceof ILinearType<?> fields)) {
            throw new HaraException(
                (instruction.opcode() == HbcProgram.Opcode.DEF_MUTABLE
                        ? "defmutable"
                        : "defstruct")
                    + " fields constant is not a vector");
          }
          HalcSchema.NamedField[] specifications =
              new HalcSchema.NamedField[Math.toIntExact(fields.count())];
          Set<String> seen = new HashSet<>();
          for (int i = 0; i < specifications.length; i++) {
            Object field = fields.nth(i);
            HalcSchema.NamedField specification;
            try {
              specification = HalcSchema.normalizeNamedField(field);
            } catch (HaraException error) {
              throw new HaraException(
                  (instruction.opcode() == HbcProgram.Opcode.DEF_MUTABLE
                          ? "defmutable"
                          : "defstruct")
                      + " field is invalid: "
                      + error.getMessage());
            }
            if (!seen.add(specification.name())) {
              throw new HaraException(
                  "Duplicate named value field: " + specification.name());
            }
            specifications[i] = specification;
          }
          stack.add(
              context.defineNamedType(
                  Symbol.create(name),
                  specifications,
                  instruction.opcode() == HbcProgram.Opcode.DEF_MUTABLE));
        }
        case BUILD_VECTOR -> {
          Object[] values = popArguments(stack, index(instruction.first()));
          stack.add(
              values.length <= 8
                  ? hara.kernel.builtin.BuiltinStruct.tuple(values)
                  : hara.lang.data.Vector.Standard.from(null, values));
        }
        case BUILD_LIST ->
            stack.add(hara.lang.data.List.Standard.from(null, popArguments(stack, index(instruction.first()))));
        case BUILD_MAP ->
            stack.add(
                hara.lang.data.Map.Standard.from(
                    null, popArguments(stack, index(instruction.first()) * 2)));
        case BUILD_SET ->
            stack.add(
                hara.lang.data.OrderedSet.Standard.from(
                    null, popArguments(stack, index(instruction.first()))));
        case CONCAT_LIST -> {
          Object[] values = popArguments(stack, index(instruction.first()));
          stack.add(HbcPrimitiveRuntime.concatList(context, values));
        }
        case TO_VECTOR -> stack.add(invokeGlobal(context, "vec", new Object[] {pop(stack)}));
        case MAKE_MULTI_ARITY -> {
          Object[] clauses = popArguments(stack, index(instruction.second()));
          HbcClosure[] closures = new HbcClosure[clauses.length];
          for (int i = 0; i < clauses.length; i++) {
            if (!(clauses[i] instanceof HbcClosure closure)) {
              throw new HaraException("multi-arity clauses must be functions");
            }
            closures[i] = closure;
          }
          stack.add(new HbcMultiArity(stringConstant(program, instruction.first()), closures));
        }
        case DEF_MACRO -> {
          Object value = pop(stack);
          Symbol symbol = Symbol.create(stringConstant(program, instruction.first()));
          IMetadata metadata = metadata(program, instruction.second());
          if (metadata != null) symbol = symbol.withMeta(metadata);
          context.defineMacro(
              symbol, new HaraMacro(context, context.currentNamespaceName(), symbol, value));
          stack.add(value);
        }
        case DEF_PROTOCOL ->
            stack.add(
                context.executeBytecodeDeclaration(
                    "defprotocol", program.constants().get(index(instruction.first()))));
        case EXTEND_TYPE ->
            stack.add(
                context.executeBytecodeDeclaration(
                    "extend-type", program.constants().get(index(instruction.first()))));
        case DEF_MULTI ->
            stack.add(
                context.executeBytecodeDeclaration(
                    "defmulti", program.constants().get(index(instruction.first()))));
        case DEF_METHOD ->
            stack.add(
                context.executeBytecodeDeclaration(
                    "defmethod", program.constants().get(index(instruction.first()))));
        case MUTABLE_FIELD_GET -> {
          Object target = pop(stack);
          if (!(target instanceof HaraMutable mutable)) {
            throw new HaraException("field expects a mutable value");
          }
          try {
            stack.add(mutable.read(stringConstant(program, instruction.first())));
          } catch (com.oracle.truffle.api.interop.UnknownIdentifierException error) {
            throw new HaraException(
                "Unknown mutable field: " + stringConstant(program, instruction.first()));
          }
        }
        case MUTABLE_FIELD_SET -> {
          Object replacement = pop(stack);
          Object target = pop(stack);
          if (!(target instanceof HaraMutable mutable)) {
            throw new HaraException("set! field expects a mutable value");
          }
          try {
            mutable.write(stringConstant(program, instruction.first()), replacement);
            stack.add(replacement);
          } catch (com.oracle.truffle.api.interop.UnknownIdentifierException error) {
            throw new HaraException(
                "Unknown mutable field: " + stringConstant(program, instruction.first()));
          }
        }
        case INSTANCE_OF -> {
          Object value = pop(stack);
          Object type = pop(stack);
          if (!(type instanceof HaraType)) {
            throw new HaraException("instance? expects a named value type");
          }
          stack.add(
              value instanceof HaraStruct struct
                  ? struct.type() == type
                  : value instanceof HaraMutable mutable && mutable.type() == type);
        }
        case HOST_CALL -> {
          Object argumentsValue = pop(stack);
          Object method = pop(stack);
          Object service = pop(stack);
          stack.add(
              invokeGlobal(
                  context,
                  "std.native.Host/call",
                  new Object[] {service, method, argumentsValue}));
        }
        case DOT_CALL -> {
          int argumentCount = index(instruction.second());
          Object[] methodArguments = popArguments(stack, argumentCount);
          Object receiver = pop(stack);
          stack.add(
              context.invokeMarkerMethod(
                  receiver, stringConstant(program, instruction.first()), methodArguments));
        }
        case AWAIT -> {
          Object awaitable = peek(stack);
          if (context.hbcPromisePending(awaitable)) {
            continuation.pendingAwait = awaitable;
            throw suspend(
                continuation,
                program,
                functionIndex,
                function,
                locals,
                stack,
                calls,
                ip,
                false,
                true);
          }
          pop(stack);
          stack.add(
              invokeGlobal(
                  context, "std.foundation.coroutine/await", new Object[] {awaitable}));
        }
        case YIELD -> {
          StdFoundationCoroutine.requireYieldContext();
          Object yielded = pop(stack);
          HbcBoundary.Transition suspend =
              new HbcBoundary.Transition(
                  HbcBoundary.TransitionKind.MACHINE_SUSPEND,
                  functionIndex,
                  ip,
                  functionIndex,
                  ip,
                  stack.size(),
                  calls.size());
          HbcInstrumentationBridge.machineEvent(
              context,
              InstrumentationModel.EventKind.MACHINE_SUSPEND,
              program,
              functionIndex,
              function,
              ip,
              HbcInstrumentationBridge.transitionData(suspend),
              locals,
              stack,
              new ArrayList<>(calls));
          Object resumed =
              invokeGlobal(context, "std.foundation.coroutine/yield", new Object[] {yielded});
          HbcBoundary.Transition resume =
              new HbcBoundary.Transition(
                  HbcBoundary.TransitionKind.MACHINE_RESUME,
                  functionIndex,
                  ip,
                  functionIndex,
                  ip + 1,
                  stack.size(),
                  calls.size());
          HbcInstrumentationBridge.machineEvent(
              context,
              InstrumentationModel.EventKind.MACHINE_RESUME,
              program,
              functionIndex,
              function,
              ip,
              HbcInstrumentationBridge.transitionData(resume),
              locals,
              stack,
              new ArrayList<>(calls));
          stack.add(resumed);
        }
        case RETURN -> {
          Object result = pop(stack);
          if (calls.isEmpty()) {
            HbcBoundary.Terminal terminal =
                new HbcBoundary.Terminal(
                    HbcBoundary.TerminalKind.RETURN,
                    functionIndex,
                    ip,
                    stack.size(),
                    calls.size());
            HbcInstrumentationBridge.machineTerminal(
                context,
                program,
                functionIndex,
                function,
                ip,
                HbcInstrumentationBridge.terminalData(terminal, "returned"),
                locals,
                stack,
                new ArrayList<>(calls),
                "returned",
                result,
                null);
            return result;
          }
          CallFrame caller = calls.peek();
          HbcBoundary.Transition callReturn =
              new HbcBoundary.Transition(
                  HbcBoundary.TransitionKind.CALL_RETURN,
                  functionIndex,
                  ip,
                  caller.functionIndex,
                  caller.returnIp,
                  stack.size(),
                  calls.size());
          HbcInstrumentationBridge.machineEvent(
              context,
              InstrumentationModel.EventKind.CALL_RETURN,
              program,
              functionIndex,
              function,
              ip,
              HbcInstrumentationBridge.transitionData(callReturn),
              locals,
              stack,
              new ArrayList<>(calls));
          caller = calls.pop();
          functionIndex = caller.functionIndex;
          function = caller.function;
          locals = caller.locals;
          stack = caller.stack;
          stack.add(result);
          ip = caller.returnIp;
          continue;
        }
        case THROW -> throwValue(program, function, ip, pop(stack));
        case RETHROW -> throwValue(program, function, ip, pop(stack));
        }
      } catch (RuntimeException failure) {
        int fromFunctionIndex = functionIndex;
        int fromInstructionPointer = ip;
        Function fromFunction = function;
        Integer target = routeFailure(function, ip, failure, locals, stack);
        while (target == null && !calls.isEmpty()) {
          CallFrame caller = calls.pop();
          functionIndex = caller.functionIndex;
          function = caller.function;
          locals = caller.locals;
          stack = caller.stack;
          target = routeFailure(function, caller.returnIp - 1, failure, locals, stack);
        }
        int toInstructionPointer = target == null ? ip : target;
        HbcBoundary.Transition unwind =
            new HbcBoundary.Transition(
                HbcBoundary.TransitionKind.EXCEPTION_UNWIND,
                fromFunctionIndex,
                fromInstructionPointer,
                functionIndex,
                toInstructionPointer,
                stack.size(),
                calls.size());
        HbcInstrumentationBridge.machineEventAt(
            context,
            InstrumentationModel.EventKind.EXCEPTION_UNWIND,
            program,
            functionIndex,
            function,
            toInstructionPointer,
            fromFunction,
            fromInstructionPointer,
            HbcInstrumentationBridge.transitionData(unwind),
            locals,
            stack,
            new ArrayList<>(calls));
        if (target == null) {
          HbcBoundary.Terminal terminal =
              new HbcBoundary.Terminal(
                  HbcBoundary.TerminalKind.FAIL,
                  functionIndex,
                  toInstructionPointer,
                  stack.size(),
                  calls.size());
          HbcInstrumentationBridge.machineTerminal(
              context,
              program,
              functionIndex,
              function,
              toInstructionPointer,
                HbcInstrumentationBridge.terminalData(terminal, "failed"),
              locals,
              stack,
              new ArrayList<>(calls),
              "failed",
              null,
              failure.getClass().getName());
        }
        if (target == null) throw failure;
        ip = target;
        continue;
      }
      ip++;
    }
  }

  private static HbcSuspended suspend(
      HbcContinuation continuation,
      HbcProgram program,
      int functionIndex,
      Function function,
      Object[] locals,
      ArrayList<Object> stack,
      ArrayDeque<CallFrame> calls,
      int instructionPointer,
      boolean paused,
      boolean guestSuspension) {
    continuation.capture(
        functionIndex, function, locals, stack, calls, instructionPointer, paused);
    if (guestSuspension) {
      HbcBoundary.Transition transition =
          new HbcBoundary.Transition(
              HbcBoundary.TransitionKind.MACHINE_SUSPEND,
              functionIndex,
              instructionPointer,
              functionIndex,
              instructionPointer,
              stack.size(),
              calls.size());
      HbcInstrumentationBridge.machineEvent(
          continuation.context,
          InstrumentationModel.EventKind.MACHINE_SUSPEND,
          program,
          functionIndex,
          function,
          instructionPointer,
          HbcInstrumentationBridge.transitionData(transition),
          locals,
          stack,
          new ArrayList<>(calls));
    }
    return new HbcSuspended(
        continuation,
        new HbcSuspension(
            continuation.id,
            program.namespace(),
            function.name(),
            instructionPointer,
            guestSuspension ? SuspensionKind.AWAIT : SuspensionKind.CONTROL_PAUSE));
  }

  private static void terminate(
      HaraContext context,
      HbcProgram program,
      int functionIndex,
      Function function,
      int instructionPointer,
      Object[] locals,
      ArrayList<Object> stack,
      ArrayDeque<CallFrame> calls) {
    HbcInstrumentationBridge.machineTerminal(
        context,
        program,
        functionIndex,
        function,
        instructionPointer,
        HbcInstrumentationBridge.terminalData(
            new HbcBoundary.Terminal(
                HbcBoundary.TerminalKind.FAIL,
                functionIndex,
                instructionPointer,
                stack.size(),
                calls.size()),
            "cancelled"),
        locals,
        stack,
        new ArrayList<>(calls),
        "cancelled",
        null,
        "cancelled");
    throw new HaraException("HBC execution terminated by instrumentation");
  }

  public static Object invokeGlobal(HaraContext context, String name, Object[] arguments) {
    return context.invokeCallable(resolve(context, name).deref(), arguments);
  }

  private static HbcProgram.Position sourcePosition(Function function, int ip) {
    HbcProgram.Position position = function.sourceMap().get(ip);
    return position == null ? new HbcProgram.Position(0, 0, 0) : position;
  }

  private static void throwValue(HbcProgram program, Function function, int ip, Object value) {
    if (!(value instanceof hara.lang.protocol.IExInfo)) {
      throw new HaraException("throw expects an Exception value created by ex");
    }
    if (value instanceof hara.lang.base.Ex.Info info) {
      HbcProgram.Position position = sourcePosition(function, ip);
      info.recordThrow(
          new hara.lang.base.Ex.Info.Site(program.namespace(), null, position.line(), position.column()));
    }
    throw new HbcThrown(value);
  }

  private static IMetadata metadata(HbcProgram program, long encodedIndex) {
    if (encodedIndex < 0) return null;
    java.util.List<HbcProgram.MetadataEntry> entries =
        program.varMetadata().get(index(encodedIndex));
    Object[] values = new Object[entries.size() * 2];
    for (int i = 0; i < entries.size(); i++) {
      values[i * 2] = metadataValue(entries.get(i).key());
      values[i * 2 + 1] = metadataValue(entries.get(i).value());
    }
    return hara.lang.data.Map.Standard.from(null, values);
  }

  @SuppressWarnings("unchecked")
  private static Object metadataValue(HbcProgram.MetadataValue metadata) {
    Object value = metadata.value();
    return switch (metadata.kind()) {
      case NIL, BOOLEAN, NUMBER, FLOAT, BIG_INTEGER, REGEX, STRING, KEYWORD, SYMBOL -> value;
      case RESERVED_DECIMAL -> throw new HbcFormatException("reserved decimal metadata in bytecode");
      case CHARACTER -> {
        int codePoint = ((Number) value).intValue();
        yield hara.lang.data.HaraCharacter.of(codePoint);
      }
      case TAGGED -> {
        HbcProgram.TaggedMetadata tagged = (HbcProgram.TaggedMetadata) value;
        yield new TaggedLiteral(Symbol.create(tagged.tag()), metadataValue(tagged.value()));
      }
      case VECTOR ->
          hara.lang.data.Vector.Standard.from(
              null,
              ((java.util.List<HbcProgram.MetadataValue>) value)
                  .stream().map(HbcMachine::metadataValue).toArray());
      case LIST ->
          hara.lang.data.List.Standard.from(
              null,
              ((java.util.List<HbcProgram.MetadataValue>) value)
                  .stream().map(HbcMachine::metadataValue).toArray());
      case SET ->
          hara.lang.data.Set.Standard.from(
              null,
              ((java.util.List<HbcProgram.MetadataValue>) value)
                  .stream().map(HbcMachine::metadataValue).toArray());
      case MAP -> {
        java.util.List<HbcProgram.MetadataEntry> entries =
            (java.util.List<HbcProgram.MetadataEntry>) value;
        Object[] pairs = new Object[entries.size() * 2];
        for (int i = 0; i < entries.size(); i++) {
          pairs[i * 2] = metadataValue(entries.get(i).key());
          pairs[i * 2 + 1] = metadataValue(entries.get(i).value());
        }
        yield hara.lang.data.Map.Standard.from(null, pairs);
      }
    };
  }

  private static Object[] bindLocals(Function function, Object[] arguments, Object[] captures) {
    checkArity(function, arguments.length);
    Object[] locals = new Object[function.localCount()];
    int fixed = function.arity();
    System.arraycopy(arguments, 0, locals, 0, Math.min(fixed, arguments.length));
    int captureBase = fixed;
    if (function.variadic()) {
      locals[fixed] =
          hara.lang.data.List.Standard.from(
              null, Arrays.copyOfRange(arguments, fixed, arguments.length));
      captureBase++;
    }
    System.arraycopy(captures, 0, locals, captureBase, captures.length);
    return locals;
  }

  private static HbcClosure selectClosure(Object callee, int arity) {
    if (callee instanceof HbcClosure closure) return closure;
    if (callee instanceof HbcMultiArity multi) {
      for (HbcClosure closure : multi.clauses) {
        Function function = closure.program.functions().get(closure.prototype);
        if ((!function.variadic() && function.arity() == arity)
            || (function.variadic() && arity >= function.arity())) return closure;
      }
    }
    return null;
  }

  private static Integer routeFailure(
      Function function,
      int errorIp,
      RuntimeException failure,
      Object[] locals,
      ArrayList<Object> stack) {
    for (int i = function.handlers().size() - 1; i >= 0; i--) {
      TryEntry handler = function.handlers().get(i);
      if (errorIp < handler.start() || errorIp >= handler.end()) continue;
      for (HbcProgram.CatchEntry clause : handler.catches()) {
        if (!catchMatches(failure, clause.className())) continue;
        truncate(stack, handler.depth());
        locals[clause.binding()] = caughtValue(failure);
        return index(clause.target());
      }
      if (handler.finallyTarget() != null) {
        truncate(stack, handler.depth());
        locals[handler.pendingValue()] = caughtValue(failure);
        locals[handler.pendingError()] = true;
        return index(handler.finallyTarget());
      }
    }
    return null;
  }

  private static boolean catchMatches(RuntimeException failure, String className) {
    if ("Exception".equals(className) || "Throwable".equals(className)) return true;
    if (failure instanceof HbcThrown thrown) {
      if (className.startsWith(":") || className.startsWith("[")) {
        Object data = thrown.value instanceof IExInfo info ? info.getData() : null;
        Object code = data instanceof IMapType map
            ? map.lookup(Keyword.create("ex", "code"))
            : null;
        if (className.startsWith("[") && className.endsWith("]")) {
          for (String selector : className.substring(1, className.length() - 1).split(",")) {
            if (Keyword.create(selector.substring(1)).equals(code)) return true;
          }
          return false;
        }
        return Keyword.create(className.substring(1)).equals(code);
      }
      String type =
          thrown.value instanceof HaraStruct struct
              ? struct.type().name()
              : thrown.value instanceof HaraMutable mutable ? mutable.type().name() : null;
      return type != null && (type.equals(className) || type.endsWith("/" + className));
    }
    return false;
  }

  private static Object caughtValue(RuntimeException failure) {
    return failure instanceof HbcThrown thrown ? thrown.value : failure.getMessage();
  }

  /**
   * Returns the guest value carried by an HBC throw when it crosses into a Truffle AST, while
   * leaving ordinary Java runtime failures unchanged. HBC handlers use {@link #caughtValue} for
   * their local binding semantics; this boundary helper is for the source-level {@code try/catch}
   * implementation.
   */
  public static Object guestThrownValue(RuntimeException failure) {
    return failure instanceof HbcThrown thrown ? thrown.value : failure;
  }

  private static void truncate(ArrayList<Object> stack, int depth) {
    while (stack.size() > depth) stack.remove(stack.size() - 1);
  }

  private static HaraVar resolve(HaraContext context, String name) {
    HaraVar variable = context.resolve(context.canonicalSymbol(Symbol.create(name)));
    if (variable == null) throw new HaraException("Unbound var: " + name);
    return variable;
  }

  private static String stringConstant(HbcProgram program, long operand) {
    Object value = program.constants().get(index(operand));
    if (!(value instanceof String string)) throw new HaraException("HBC0 name constant is not a string");
    return string;
  }

  private static boolean isPendingDerefTarget(String target) {
    return "std.protocol.ideref.IDeref/deref".equals(target);
  }

  private static Object[] popArguments(ArrayList<Object> stack, int count) {
    int start = stack.size() - count;
    if (start < 0) throw new HaraException("HBC0 stack underflow");
    Object[] values = new Object[count];
    for (int i = 0; i < count; i++) values[i] = stack.remove(start);
    return values;
  }

  private static Object pop(ArrayList<Object> stack) {
    if (stack.isEmpty()) throw new HaraException("HBC0 stack underflow");
    return stack.remove(stack.size() - 1);
  }

  private static Object peek(ArrayList<Object> stack) {
    if (stack.isEmpty()) throw new HaraException("HBC0 stack underflow");
    return stack.get(stack.size() - 1);
  }

  private static int index(long value) {
    return Math.toIntExact(value);
  }

  private static boolean truthy(Object value) {
    return !HaraBox.isNil(value) && !Boolean.FALSE.equals(HaraBox.unwrap(value));
  }

  private static void checkArity(Function function, int actual) {
    if ((!function.variadic() && actual != function.arity())
        || (function.variadic() && actual < function.arity())) {
      String expected =
          function.variadic()
              ? "at least " + function.arity()
              : Integer.toString(function.arity());
      throw new HaraException("Expected " + expected + " arguments, received " + actual);
    }
  }

  private static String primitiveName(int id) {
    return switch (HbcProgram.Primitive.fromId(id)) {
      case ADD -> "+";
      case SUBTRACT -> "-";
      case MULTIPLY -> "*";
      case DIVIDE -> "/";
      case REMAINDER -> "rem";
      case MODULO -> "mod";
      case EQUAL -> "=";
      case LESS -> "<";
      case LESS_OR_EQUAL -> "<=";
      case GREATER -> ">";
      case GREATER_OR_EQUAL -> ">=";
      case COUNT -> "count";
      case GET -> "get";
      case META -> "meta";
      case NTH -> "nth";
      case ASSOC -> "assoc";
      case FIRST -> "first";
      case REST -> "rest";
      case SECOND -> "second";
      case TO_MUTABLE -> "to-mutable";
      case TO_PERSISTENT -> "to-persistent";
      case NUMBER_PREDICATE -> "number?";
      case ARRAY_NEW -> "array";
      case ARRAY_GET -> "std.native.Arr/get";
      case ARRAY_SET -> "std.native.Arr/set";
      case OBJECT_NEW -> "object";
      case OBJECT_GET -> "std.native.Obj/get";
      case OBJECT_SET -> "std.native.Obj/set";
    };
  }

  public static Object invokePrimitive(HaraContext context, int id, Object[] arguments) {
    HbcProgram.Primitive primitive = HbcProgram.Primitive.fromId(id);
    if (primitive == HbcProgram.Primitive.EQUAL) {
      if (arguments.length < 2) throw new HaraException("= expects at least 2 arguments");
      Object first = HaraBox.unwrap(arguments[0]);
      for (int i = 1; i < arguments.length; i++) {
        Object value = HaraBox.unwrap(arguments[i]);
        if (first instanceof Number left && value instanceof Number right) {
          if (!hara.lang.base.primitive.Num.eq(left, right)) return false;
        } else if (!hara.lang.base.Eq.eq(first, value)) {
          return false;
        }
      }
      return true;
    }
    if (primitive == HbcProgram.Primitive.FIRST
        || primitive == HbcProgram.Primitive.REST
        || primitive == HbcProgram.Primitive.SECOND) {
      if (arguments.length != 1) {
        throw new HaraException(primitiveName(id) + " expects one argument");
      }
      Object value = HaraBox.unwrap(arguments[0]);
      Iterator<?> iterator = (Iterator<?>) context.iterValue(value);
      if (primitive == HbcProgram.Primitive.REST) return context.restSequence(iterator);
      if (!iterator.hasNext()) return null;
      Object first = iterator.next();
      if (primitive == HbcProgram.Primitive.FIRST) return first;
      return iterator.hasNext() ? iterator.next() : null;
    }
    if (primitive == HbcProgram.Primitive.REMAINDER
        || primitive == HbcProgram.Primitive.MODULO) {
      if (arguments.length != 2) {
        throw new HaraException(primitiveName(id) + " expects two numbers");
      }
      Object left = HaraBox.unwrap(arguments[0]);
      Object right = HaraBox.unwrap(arguments[1]);
      if (!(left instanceof Number) || !(right instanceof Number)) {
        throw new HaraException(primitiveName(id) + " expects two numbers");
      }
      return primitive == HbcProgram.Primitive.REMAINDER
          ? Num.remainder(left, right)
          : Num.mod(left, right);
    }
    try {
      String name = primitiveName(id);
      return invokeGlobal(
          context,
          name.contains("/") ? name : "std.foundation/" + name,
          arguments);
    } catch (RuntimeException failure) {
      if ((primitive == HbcProgram.Primitive.DIVIDE
              || primitive == HbcProgram.Primitive.REMAINDER
              || primitive == HbcProgram.Primitive.MODULO)
          && failure.getMessage() != null
          && failure.getMessage().toLowerCase(java.util.Locale.ROOT).contains("divide by zero")) {
        throw new HaraException("division by zero");
      }
      throw failure;
    }
  }

  private static Integer primitiveId(String name) {
    boolean foundationQualified = name.startsWith("std.foundation/");
    String local =
        foundationQualified ? name.substring("std.foundation/".length()) : name;
    for (HbcProgram.Primitive primitive : HbcProgram.Primitive.values()) {
      String primitiveName = primitiveName(primitive.id());
      if (primitiveName.equals(name)
          || (foundationQualified && !primitiveName.contains("/") && primitiveName.equals(local))) {
        return primitive.id();
      }
    }
    return null;
  }

  @ExportLibrary(InteropLibrary.class)
  static final class HbcClosure implements TruffleObject {
    final HbcProgram program;
    final HaraContext context;
    final int prototype;
    final Object[] captures;
    final String namespace;
    private volatile RootCallTarget nativeTarget;

    HbcClosure(HbcProgram program, HaraContext context, int prototype, Object[] captures) {
      this.program = program;
      this.context = context;
      this.prototype = prototype;
      this.captures = captures;
      this.namespace = context.currentNamespaceName();
    }

    @TruffleBoundary
    Object invoke(Object[] arguments) {
      Function function = program.functions().get(prototype);
      RootCallTarget target = context.hbcNativeExecutionAllowed() ? nativeTarget : null;
      if (target == null) {
        if (context.hbcNativeExecutionAllowed()) {
          target =
              HbcBytecodeRootNode.compileFunction(
                  HaraLanguage.currentLanguage(), program, prototype);
          if (target != null) nativeTarget = target;
        }
      }
      if (target != null) return target.call(arguments);
      if (function.asyncFunction()) {
        return context.hbcAsync(() -> call(program, context, prototype, arguments, captures));
      }
      return call(program, context, prototype, arguments, captures);
    }

    @ExportMessage
    boolean isExecutable() {
      return true;
    }

    @ExportMessage
    Object execute(Object[] arguments) {
      return HaraBox.export(invoke(arguments));
    }

    @ExportMessage
    Object toDisplayString(boolean allowSideEffects) {
      return "<fn>";
    }

    @Override
    public String toString() {
      return "<fn>";
    }
  }

  @ExportLibrary(InteropLibrary.class)
  static final class HbcMultiArity implements TruffleObject {
    final String name;
    final HbcClosure[] clauses;

    HbcMultiArity(String name, HbcClosure[] clauses) {
      this.name = name;
      this.clauses = clauses;
    }

    @TruffleBoundary
    Object invoke(Object[] arguments) {
      for (HbcClosure clause : clauses) {
        Function function = clause.program.functions().get(clause.prototype);
        if ((!function.variadic() && function.arity() == arguments.length)
            || (function.variadic() && arguments.length >= function.arity())) {
          return clause.invoke(arguments);
        }
      }
      throw new HaraException(name + " has no arity " + arguments.length);
    }

    @ExportMessage
    boolean isExecutable() {
      return true;
    }

    @ExportMessage
    Object execute(Object[] arguments) {
      return HaraBox.export(invoke(arguments));
    }

    @ExportMessage
    Object toDisplayString(boolean allowSideEffects) {
      return "<fn>";
    }

    @Override
    public String toString() {
      return "<fn>";
    }
  }

  @ExportLibrary(InteropLibrary.class)
  static final class HbcNativeCallable implements TruffleObject {
    final java.util.function.Function<Object[], Object> implementation;

    HbcNativeCallable(java.util.function.Function<Object[], Object> implementation) {
      this.implementation = implementation;
    }

    @TruffleBoundary
    Object invoke(Object[] arguments) {
      return implementation.apply(arguments);
    }

    @ExportMessage
    boolean isExecutable() {
      return true;
    }

    @ExportMessage
    Object execute(Object[] arguments) {
      return HaraBox.export(invoke(arguments));
    }

    @ExportMessage
    Object toDisplayString(boolean allowSideEffects) {
      return "<fn>";
    }

    @Override
    public String toString() {
      return "<fn>";
    }
  }

  public enum SuspensionKind {
    CONTROL_PAUSE,
    AWAIT
  }

  public record HbcSuspension(
      long id,
      String namespace,
      String function,
      int instructionPointer,
      SuspensionKind kind) {}

  private static final class HbcSuspended extends RuntimeException {
    final HbcContinuation continuation;
    final HbcSuspension identity;

    HbcSuspended(HbcContinuation continuation, HbcSuspension identity) {
      this.continuation = continuation;
      this.identity = identity;
    }
  }

  static final class HbcContinuation {
    private static final java.util.concurrent.atomic.AtomicLong IDS =
        new java.util.concurrent.atomic.AtomicLong();
    final long id = IDS.incrementAndGet();
    final HbcProgram program;
    final Object[] arguments;
    final Object[] captures;
    HaraContext context;
    int functionIndex;
    Function function;
    Object[] locals;
    ArrayList<Object> stack;
    ArrayDeque<CallFrame> calls;
    int instructionPointer;
    boolean initialized;
    boolean paused;
    boolean stepAfterInstruction;
    Object pendingAwait;

    HbcContinuation(HbcProgram program) {
      this(program, program.entry(), new Object[0], new Object[0]);
    }

    HbcContinuation(HbcProgram program, int functionIndex, Object[] arguments, Object[] captures) {
      this.program = program;
      this.arguments = arguments;
      this.captures = captures;
      this.functionIndex = functionIndex;
    }

    void capture(
        int functionIndex,
        Function function,
        Object[] locals,
        ArrayList<Object> stack,
        ArrayDeque<CallFrame> calls,
        int instructionPointer,
        boolean paused) {
      this.functionIndex = functionIndex;
      this.function = function;
      this.locals = locals;
      this.stack = stack;
      this.calls = calls;
      this.instructionPointer = instructionPointer;
      this.initialized = true;
      this.paused = paused;
      this.stepAfterInstruction = false;
    }
  }

  private static final class HbcThrown extends RuntimeException {
    final Object value;

    HbcThrown(Object value) {
      super("thrown: " + value);
      this.value = value;
    }
  }

  static record CallFrame(
      int functionIndex,
      Function function,
      Object[] locals,
      ArrayList<Object> stack,
      int returnIp) {}
}
