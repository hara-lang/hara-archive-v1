package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertTrue;

import hara.lang.data.Keyword;
import org.graalvm.polyglot.Context;
import org.junit.Test;

public class SyntaxEnvironmentResultTest {
  @Test
  public void commentSuppressesAnalysisAndEvaluation() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      assertEquals(
          "[nil true]",
          context
              .eval(
                  HaraLanguage.ID,
                  "[(comment missing-symbol (throw (ex-info \"boom\" {})) (def leaked 1)) "
                      + "(special-symbol? 'comment)]")
              .toString());
    }
  }

  @Test
  public void nativeRuntimeAndFoundationEnvironmentContractsAreAvailable() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      assertEquals(
          "[user 42 42 42 true :loaded nil nil nil nil true :success nil]",
          context
              .eval(
                  HaraLanguage.ID,
                  "[(Runtime/current) "
                      + "(Runtime/eval-in 'user '[(+ 19 23)]) "
                      + "(Runtime/eval '(+ 19 23)) "
                      + "(Runtime/load-string \"(+ 19 23)\") "
                      + "(map? (std.foundation/env-snapshot)) "
                      + "(get (Runtime/namespace 'std.native.Runtime) :namespace/state) "
                      + "(Runtime/namespace 'std.native.Env) "
                      + "(std.foundation/resolve 'std.native.Env/current) "
                      + "(std.foundation/resolve 'std.native.Runtime/resolve) "
                      + "(std.foundation/resolve 'std.native.Base/eval) "
                      + "(result? (Result/create :success 1)) "
                      + "(Result/status (Result/create :success 1)) "
                      + "(Result/context (Result/create :success 1))]")
              .toString());
    }
  }

  @Test
  public void resultRoundTripsThroughHtaAsANativeResult() {
    HaraResult result =
        HaraResult.success(
            42L,
            hara.lang.data.Map.Standard.from(
                null, Keyword.create("source"), Keyword.create("hta")));

    Object decoded = HtaValueCodec.decodeCanonical(HtaValueCodec.encode(result));

    assertTrue(decoded instanceof HaraResult);
    assertTrue(result.equality(decoded));
    assertEquals(
        Keyword.create("hta"), ((HaraResult) decoded).context().lookup(Keyword.create("source")));
  }
}
