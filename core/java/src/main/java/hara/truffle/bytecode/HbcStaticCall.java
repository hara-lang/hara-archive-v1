package hara.truffle.bytecode;

import com.oracle.truffle.api.RootCallTarget;
import hara.truffle.HaraContext;
import hara.truffle.HaraLanguage;

/** Static-call identity and lazily compiled target carried by a generated HBC operation. */
public final class HbcStaticCall {
  private final HbcProgram program;
  private final int functionIndex;
  private volatile RootCallTarget nativeTarget;

  public HbcStaticCall(HbcProgram program, int functionIndex) {
    this.program = program;
    this.functionIndex = functionIndex;
  }

  public HbcProgram program() {
    return program;
  }

  public int functionIndex() {
    return functionIndex;
  }

  public RootCallTarget nativeTarget(HaraLanguage language) {
    HaraContext context = HaraLanguage.currentContext();
    if (context != null && !context.hbcNativeExecutionAllowed()) return null;
    RootCallTarget target = nativeTarget;
    if (target != null) return target;
    target = HbcBytecodeRootNode.compileFunction(language, program, functionIndex);
    if (target != null) nativeTarget = target;
    return target;
  }
}
