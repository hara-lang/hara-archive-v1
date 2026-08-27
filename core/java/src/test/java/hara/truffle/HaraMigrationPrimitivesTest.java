package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertThrows;
import static org.junit.Assert.assertTrue;

import java.nio.file.Files;
import java.nio.file.Path;
import org.graalvm.polyglot.Context;
import org.graalvm.polyglot.PolyglotException;
import org.graalvm.polyglot.Value;
import org.graalvm.polyglot.io.IOAccess;
import org.junit.Test;

public class HaraMigrationPrimitivesTest {
  @Test
  public void defstructDefinesPositionalAndMapConstructors() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      assertEquals(
          3,
          context
              .eval(
                  HaraLanguage.ID,
                  "(defstruct Point [x y]) "
                      + "(+ (:x (->Point 1 2)) "
                      + "   (:x (map->Point {:x 2 :y 4})))")
              .asInt());
    }
  }

  @Test
  public void keywordInvocationUsesDefstructMapSemantics() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      assertEquals(
          "[1 7 nil 1 :user.Point]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(do (defstruct Point [x y]) "
                      + "(let [point (map->Point {:x 1 :extra 9})] "
                      + "[(:x point) (:missing point 7) (:extra point) "
                      + " (get point :x) (type point)]))")
              .toString());
    }
  }

  @Test
  public void mapDestructuringUsesDefstructLookupSemantics() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      assertEquals(
          "[[1 2 7 :user.Point] [3 4]]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(do (defstruct Point [x y]) "
                      + "[(let [{:keys [x y missing] :or {missing 7} :as point} "
                      + "       (Point 1 2)] "
                      + "   [x y missing (type point)]) "
                      + " ((fn [{:keys [x y]}] [x y]) (Point 3 4))])")
              .toString());
    }
  }

  @Test
  public void mapConstructorUsesFoundationGetWhenNamespaceShadowsGet() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      assertEquals(
          "[1 2 [1 2]]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(do (ns migration.shadow (:config {:override [get]})) "
                      + "(defstruct Point [x y]) "
                      + "(defn get [value key] :shadowed) "
                      + "(let [{:keys [x y]} (map->Point {:x 1 :y 2})] "
                      + "  [x y [(:x (map->Point {:x 1 :y 2})) "
                      + "        (:y (map->Point {:x 1 :y 2}))]]))")
              .toString());
    }
  }

  @Test
  public void instancePredicateIsRestrictedToHaraStructTypes() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      Value result =
          context.eval(
              HaraLanguage.ID,
              "(defstruct Point [x y]) "
                  + "(defstruct Other [x y]) "
                  + "[(instance? Point (->Point 1 2)) "
                  + " (instance? Other (->Point 1 2)) "
                  + " (instance? Point {:x 1 :y 2})]");
      assertTrue(result.getArrayElement(0).asBoolean());
      assertFalse(result.getArrayElement(1).asBoolean());
      assertFalse(result.getArrayElement(2).asBoolean());
    }
  }

  @Test
  public void namedDeclarationRollsBackWhenAnInlineProtocolClauseIsInvalid() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      PolyglotException error =
          assertThrows(
              PolyglotException.class,
              () ->
                  context.eval(
                      HaraLanguage.ID,
                      "(defstruct Atomic [value] "
                          + "ICount (count [self extra] (:value self)))"));
      assertFalse(error.getMessage().isEmpty());
      assertEquals(
          "[nil nil nil]",
          context
              .eval(
                  HaraLanguage.ID,
                  "[(resolve 'Atomic) (resolve '->Atomic) (resolve 'map->Atomic)]")
              .toString());
    }
  }

  @Test
  public void namespaceIntrospectionIsNarrowAndDeterministic() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      Value result =
          context.eval(
              HaraLanguage.ID,
              "(ns sample.alpha) "
                  + "(def zed 1) "
                  + "(def alpha 2) "
                  + "(let [entries (iter (ns-publics 'sample.alpha))] "
                  + " (let [first-entry (iter-next entries)] "
                  + "  (let [second-entry (iter-next entries)] "
                  + "   [(ns-name 'sample.alpha) "
                  + "    (nth first-entry 0) "
                  + "    (nth second-entry 0) "
                  + "    (ns-find 'missing.namespace)])))");
      assertEquals("sample.alpha", result.getArrayElement(0).toString());
      assertEquals("alpha", result.getArrayElement(1).toString());
      assertEquals("zed", result.getArrayElement(2).toString());
      assertTrue(result.getArrayElement(3).isNull());
    }
  }

  @Test
  public void namespaceFindAndCreateUseCanonicalNamespaceHandles() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      assertEquals(
          "[:std.native.Namespace sample.created sample.created nil]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(let [created (Runtime/ns-create 'sample.created)] "
                      + "[(type created) "
                      + " (Runtime/ns-name created) "
                      + " (Runtime/ns-name (Runtime/ns-find 'sample.created)) "
                      + " (Runtime/ns-find 'missing.namespace)])")
              .toString());
    }
  }

  @Test
  public void removedNamespaceOperationNamesDoNotResolve() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      assertEquals(
          "[true true true true true true true]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(mapv nil? [(resolve 'the-ns) "
                      + "(resolve 'env-resolve) "
                      + "(resolve 'ns:create) "
                      + "(resolve 'ns:list) "
                      + "(resolve 'ns:map) "
                      + "(resolve 'ns:name) "
                      + "(resolve 'ns:imports)])")
              .toString());
    }
  }

  @Test
  public void readFormsRequiresIoAndPreservesSourceSpans() throws Exception {
    Path source = Files.createTempFile("hara-read-forms", ".hal");
    Files.writeString(source, "(def first-value 1)\n\n(def second-value 2)\n");
    String expression =
        "(let [forms (read-forms \""
            + source.toString().replace("\\", "\\\\")
            + "\")] "
            + "[(count forms) "
            + " (get (meta (nth forms 1)) :file) "
            + " (get (meta (nth forms 1)) :line)])";

    try (Context denied = Context.newBuilder(HaraLanguage.ID).build()) {
      PolyglotException error =
          assertThrows(PolyglotException.class, () -> denied.eval(HaraLanguage.ID, expression));
      assertFalse(error.getMessage().isEmpty());
    }

    try (Context allowed =
        Context.newBuilder(HaraLanguage.ID).allowIO(IOAccess.ALL).build()) {
      Value result = allowed.eval(HaraLanguage.ID, expression);
      assertEquals(2, result.getArrayElement(0).asInt());
      assertEquals(source.toAbsolutePath().normalize().toString(), result.getArrayElement(1).asString());
      assertEquals(3, result.getArrayElement(2).asInt());
    }
  }
}
