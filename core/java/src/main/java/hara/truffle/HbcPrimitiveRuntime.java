package hara.truffle;

import hara.truffle.bytecode.HbcProgram;
import hara.truffle.bytecode.HbcStaticCall;
import java.util.ArrayList;
import java.util.Iterator;

/** Shared primitive boundary used by both the portable HBC machine and generated HBC operations. */
public final class HbcPrimitiveRuntime {
  private HbcPrimitiveRuntime() {}

  public static Object invoke(HaraContext context, HbcProgram.Primitive primitive, Object[] arguments) {
    return HbcMachine.invokePrimitive(context, primitive.id(), arguments);
  }

  /** Executes a canonical protocol/native target through the shared Java bridge dispatcher. */
  public static Object invokeTarget(
      HaraContext context,
      String target,
      Object[] arguments,
      HaraTargetRuntime.ResultMode resultMode) {
    return HaraTargetRuntime.invoke(context, target, arguments, resultMode);
  }

  public static Object concatList(HaraContext context, Object[] values) {
    ArrayList<Object> concatenated = new ArrayList<>();
    for (Object value : values) {
      Iterator<?> iterator = (Iterator<?>) context.iterValue(value);
      while (iterator.hasNext()) concatenated.add(iterator.next());
    }
    return hara.lang.data.List.Standard.from(null, concatenated.toArray());
  }

  public static Object invokeStatic(
      HbcStaticCall target, HaraContext context, Object[] arguments) {
    com.oracle.truffle.api.RootCallTarget nativeTarget =
        target.nativeTarget(HaraLanguage.currentLanguage());
    return nativeTarget == null
        ? HbcMachine.invokeFunction(
            target.program(), context, target.functionIndex(), arguments)
        : nativeTarget.call(arguments);
  }

  public static Object invokeStatic(
      HbcProgram program, HaraContext context, int functionIndex, Object[] arguments) {
    return HbcMachine.invokeFunction(program, context, functionIndex, arguments);
  }

  public static Object toVector(HaraContext context, Object value) {
    return HbcMachine.invokeGlobal(context, "vec", new Object[] {value});
  }
}
