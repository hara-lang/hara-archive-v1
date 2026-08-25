package hara.truffle;

import static org.junit.Assert.assertEquals;

import hara.lang.data.Vector;
import org.graalvm.polyglot.Context;
import org.junit.Test;

/** Verifies the Java target dispatcher used by the Whole-Wasm/HBC boundary. */
public class HaraTargetRuntimeTest {
  @Test
  public void dispatchesProtocolAndNativeTargetsWithTypedResults() {
    try (Context polyglot = Context.newBuilder(HaraLanguage.ID).build()) {
      polyglot.enter();
      try {
        polyglot.eval(HaraLanguage.ID, "nil");
        HaraContext context = HaraLanguage.currentContext();
        assertEquals(
            3L,
            HbcPrimitiveRuntime.invokeTarget(
                context,
                "std.protocol.icount.ICount/count",
                new Object[] {Vector.Standard.from(null, 1L, 2L, 3L)},
                HaraTargetRuntime.ResultMode.I64));
        assertEquals(
            Boolean.TRUE,
            HbcPrimitiveRuntime.invokeTarget(
                context,
                "std.native.Base/number?",
                new Object[] {1L},
                HaraTargetRuntime.ResultMode.BOOL));
      } finally {
        polyglot.leave();
      }
    }
  }
}
