package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertThrows;
import static org.junit.Assert.assertTrue;

import org.graalvm.polyglot.Context;
import org.graalvm.polyglot.PolyglotException;
import org.graalvm.polyglot.Source;
import org.junit.Test;

public class HaraCoreFormsTest {
  @Test
  public void varReturnsTheNamespaceVarAndDerefReadsItsCurrentRoot() {
    try (Context context = context()) {
      assertEquals(
          42,
          context
              .eval(HaraLanguage.ID, "(def answer 41) (def answer 42) (deref (var answer))")
              .asLong());
      assertEquals(42, context.eval(HaraLanguage.ID, "@(var answer)").asLong());
      assertTrue(context.eval(HaraLanguage.ID, "(var answer)").toString().contains("user/answer"));
    }
  }

  @Test
  public void varSupportsQualifiedLookupsWithoutChangingTheCurrentNamespace() {
    try (Context context = context()) {
      assertEquals(
          42,
          context
              .eval(
                  HaraLanguage.ID,
                  "(ns alpha) (def answer 42) (ns user) (deref (var alpha/answer))")
              .asLong());
    }
  }

  @Test
  public void dashQualifierResolvesValuesAndVarsInTheCurrentNamespace() {
    try (Context context = context()) {
      assertEquals(
          "[42 42 true]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(ns example.current) "
                      + "(def answer 42) "
                      + "[answer -/answer (= #'answer #'-/answer)]")
              .toString());
      assertEquals(
          "-/answer", context.eval(HaraLanguage.ID, "(quote -/answer)").toString());
      assertTrue(
          assertThrows(
                  PolyglotException.class,
                  () -> context.eval(HaraLanguage.ID, "-/missing"))
              .getMessage()
              .contains("Unbound symbol"));
    }
  }

  @Test
  public void defnSchemaVarReferencesMustResolve() {
    try (Context context = context()) {
      assertEquals(
          42,
          context
              .eval(
                  HaraLanguage.ID,
                  "(def Customer [:map [:id :int]]) "
                      + "(defn ^{:schema #'-/Customer} customer-id [customer] "
                      + "  (get customer :id)) "
                      + "(customer-id {:id 42})")
              .asLong());
      assertTrue(
          assertThrows(
                  PolyglotException.class,
                  () ->
                      context.eval(
                          HaraLanguage.ID,
                          "(defn ^{:schema #'MissingSchema} invalid [value] value)"))
              .getMessage()
              .contains("schema Var does not exist: MissingSchema"));
      assertEquals(
          7,
          context
              .eval(
                  HaraLanguage.ID,
                  "(ns example.forward-schema) "
                      + "(defn ^{:schema #'Customer} customer-id [customer] "
                      + "  (get customer :id)) "
                      + "(def Customer [:map [:id :int]]) "
                      + "(customer-id {:id 7})")
              .asLong());
    }
  }

  @Test
  public void derefUsesTheLanguageProtocol() {
    try (Context context = context()) {
      assertEquals(
          42,
          context
              .eval(
                  HaraLanguage.ID,
                  "(defstruct Box [value]) "
                      + "(extend-type Box IDeref (deref [self] (:value self))) "
                      + "@(Box 42)")
              .asLong());
    }
  }

  @Test
  public void varsSupportMetadataAndProtocolRootReset() {
    try (Context context = context()) {
      assertEquals(
          42,
          context
              .eval(
                  HaraLanguage.ID,
                  "(def ^:dynamic answer 41) "
                      + "(IReset/reset (var answer) 42) "
                      + "(deref (var answer))")
              .asLong());
      assertTrue(
          context
              .eval(
                  HaraLanguage.ID,
                  "(ILookup/lookup "
                      + "(IObjType/meta (var answer)) :dynamic)")
              .asBoolean());
    }
  }

  @Test
  public void bindingTemporarilyShadowsDynamicVarsAndRestoresOnThrow() {
    try (Context context = context()) {
      assertEquals(
          42,
          context
              .eval(
                  HaraLanguage.ID,
                  "(def ^:dynamic *answer* 1) " + "(binding [*answer* 42] *answer*)")
              .asLong());
      assertEquals(1, context.eval(HaraLanguage.ID, "*answer*").asLong());
      assertThrows(
          PolyglotException.class,
          () -> context.eval(HaraLanguage.ID, "(binding [*answer* 9] (throw :failed))"));
      assertEquals(1, context.eval(HaraLanguage.ID, "*answer*").asLong());
    }
  }

  @Test
  public void bindingRejectsNonDynamicAndMissingVars() {
    try (Context context = context()) {
      context.eval(HaraLanguage.ID, "(def answer 1)");
      assertTrue(
          assertThrows(
                  PolyglotException.class,
                  () -> context.eval(HaraLanguage.ID, "(binding [answer 2] answer)"))
              .getMessage()
              .contains("requires a dynamic Var"));
      assertTrue(
          assertThrows(
                  PolyglotException.class,
                  () -> context.eval(HaraLanguage.ID, "(binding [*missing* 2] *missing*)"))
              .getMessage()
              .contains("Unbound dynamic var"));
    }
  }

  @Test
  public void setBangRebindsAvarRootWithoutChangingItsIdentity() {
    try (Context context = context()) {
      assertEquals(
          42,
          context
              .eval(
                  HaraLanguage.ID,
                  "(def answer 1) "
                      + "(let [v (var answer)] (set! answer 42) (if (= v (var answer)) answer 0))")
              .asLong());
      assertTrue(
          assertThrows(
                  PolyglotException.class, () -> context.eval(HaraLanguage.ID, "(set! missing 1)"))
              .getMessage()
              .contains("Unbound var"));
    }
  }

  @Test
  public void runtimeErrorsExposeTheAnalyzedFormSourceLocation() {
    try (Context context = context()) {
      PolyglotException error =
          assertThrows(
              PolyglotException.class, () -> context.eval(HaraLanguage.ID, "(do\n  (+ 1 :bad))"));
      assertTrue(error.getMessage().contains("expects two numbers"));
      assertTrue(error.getSourceLocation() != null);
      assertEquals(2, error.getSourceLocation().getStartLine());
    }
  }

  @Test
  public void runtimeErrorsPreserveSourceNameAndLeadingWhitespaceLocation() throws Exception {
    try (Context context = context()) {
      Source source =
          Source.newBuilder(HaraLanguage.ID, "  \n  (+ 1 :bad)", "diagnostic.hal").build();
      PolyglotException error = assertThrows(PolyglotException.class, () -> context.eval(source));
      assertTrue(error.getSourceLocation() != null);
      assertEquals("diagnostic.hal", error.getSourceLocation().getSource().getName());
      assertEquals(2, error.getSourceLocation().getStartLine());
    }
  }

  @Test
  public void malformedReaderErrorsPreserveSourceNameAndReaderPosition() throws Exception {
    try (Context context = context()) {
      Source source = Source.newBuilder(HaraLanguage.ID, "(+ 1 2", "broken.hal").build();
      PolyglotException error = assertThrows(PolyglotException.class, () -> context.eval(source));
      assertTrue(error.getMessage().contains("Unable to read Hara source broken.hal"));
      assertTrue(error.getMessage().contains("line 1"));
      assertTrue(error.getMessage().contains("column"));
    }
  }

  @Test
  public void compileErrorsExposeTheOffendingFormSourceLocation() throws Exception {
    try (Context context = context()) {
      Source source = Source.newBuilder(HaraLanguage.ID, "  \n  (if true)", "compile.hal").build();
      PolyglotException error = assertThrows(PolyglotException.class, () -> context.eval(source));
      assertTrue(error.getMessage().contains("if expects two or three arguments"));
      assertTrue(error.getSourceLocation() != null);
      assertEquals("compile.hal", error.getSourceLocation().getSource().getName());
      assertEquals(2, error.getSourceLocation().getStartLine());
    }
  }

  @Test
  public void condBranchesPreserveLoopTailPosition() {
    try (Context context = context()) {
      assertEquals(
          3,
          context
              .eval(
                  HaraLanguage.ID,
                  "(loop [value 0] (cond (< value 3) (recur (inc value)) :else value))")
              .asLong());
    }
  }

  @Test
  public void qualifiedCatchCallsRemainTryBodyExpressions() {
    try (Context context = context()) {
      assertEquals(
          42,
          context
              .eval(
                  HaraLanguage.ID,
                  "(deref (try (promise/catch (promise/from 42) (fn [_] 0)) "
                      + "            (catch Throwable error (promise/from -1))))")
              .asLong());
    }
  }

  @Test
  public void varAndDerefFailuresAreDeterministic() {
    try (Context context = context()) {
      assertTrue(
          assertThrows(
                  PolyglotException.class, () -> context.eval(HaraLanguage.ID, "(var missing)"))
              .getMessage()
              .contains("Unbound var: missing"));
      assertTrue(
          assertThrows(PolyglotException.class, () -> context.eval(HaraLanguage.ID, "(var 1)"))
              .getMessage()
              .contains("var expects a symbol"));
      assertTrue(
          assertThrows(PolyglotException.class, () -> context.eval(HaraLanguage.ID, "(deref 1)"))
              .getMessage()
              .contains("No std.protocol.ideref.IDeref/deref implementation"));
    }
  }

  private static Context context() {
    return Context.newBuilder(HaraLanguage.ID).build();
  }
}
