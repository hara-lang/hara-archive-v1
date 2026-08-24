package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertThrows;
import static org.junit.Assert.assertTrue;

import java.util.Set;
import org.graalvm.polyglot.Context;
import org.graalvm.polyglot.PolyglotException;
import org.junit.Test;

public class FoundationFallbackDemandTest {
  @Test
  public void indexesPortableDefinitionsThatShadowJavaExports() {
    for (String name : new String[] {"apply-with", "has?", "identity", "if-not", "long", "map"}) {
      assertTrue(name, FoundationFallbackDefinitions.defines(name));
    }
    for (String name : new String[] {"assoc", "nth", "read-string"}) {
      assertFalse(name, FoundationFallbackDefinitions.defines(name));
      assertFalse(name, FoundationFallbackDefinitions.isInitializationDependency(name));
    }
    assertFalse(FoundationFallbackDefinitions.defines("+"));
  }

  @Test
  public void semanticDependencyPassOnlySeesPortableSpecialForms() {
    Set<String> special = Set.of("when");
    assertTrue(
        FoundationFallbackDefinitions.requiresInitialization(
            HaraLanguage.readAll("(when true 42)", "demand-test"),
            symbol -> special.contains(symbol.getName())));
    for (String source :
        new String[] {
          "(read-string \"12.5\")",
          "(assoc [1 2 3] 0 :x)",
          "(nth [10 20] 1)",
          "(+ 19 23)"
        }) {
      assertFalse(
          source,
          FoundationFallbackDefinitions.requiresInitialization(
              HaraLanguage.readAll(source, "demand-test"),
              symbol -> special.contains(symbol.getName())));
    }
  }

  @Test
  public void builtinAndClosedLexicalSourceDoesNotMaterializeFoundation() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      assertEquals(42L, context.eval(HaraLanguage.ID, "(+ 19 23)").asLong());
      assertTrue(context.eval(HaraLanguage.ID, "(= nil (resolve 'map))").asBoolean());
      assertEquals(
          42L,
          context
              .eval(
                  HaraLanguage.ID,
                  "(do (defn local-successor [x] (+ x 1)) (local-successor 41))")
              .asLong());
      assertTrue(context.eval(HaraLanguage.ID, "(= nil (resolve 'map))").asBoolean());
    }
  }

  @Test
  public void javaBackedReaderAndTupleOperationsRemainOnTheLazyPath() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      assertTrue(
          context
              .eval(HaraLanguage.ID, "(= 12.5 (read-string \"12.5\"))")
              .asBoolean());
      assertEquals(
          "[:x 2 3]",
          context.eval(HaraLanguage.ID, "(str (assoc [1 2 3] 0 :x))").asString());
      assertEquals(20L, context.eval(HaraLanguage.ID, "(nth [10 20] 1)").asLong());
      assertTrue(context.eval(HaraLanguage.ID, "(= nil (resolve 'map))").asBoolean());
    }
  }

  @Test
  public void firstFallbackFunctionReferenceMaterializesFoundation() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      assertTrue(context.eval(HaraLanguage.ID, "(= nil (resolve 'map))").asBoolean());
      assertTrue(
          context
              .eval(
                  HaraLanguage.ID,
                  "(= [2 3] (map (fn [value] (+ value 1)) [1 2]))")
              .asBoolean());
      assertFalse(context.eval(HaraLanguage.ID, "(= nil (resolve 'map))").asBoolean());
    }
  }

  @Test
  public void automaticQualifiedAliasAndReferAccessMaterializePortableDefinitions() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      assertEquals(
          42L,
          context
              .eval(HaraLanguage.ID, "(ns foundation-automatic) (apply-with 2 + 19 21)")
              .asLong());

      assertEquals(
          42L,
          context.eval(HaraLanguage.ID, "(std.foundation/apply-with 2 + 19 21)").asLong());

      assertEquals(
          42L,
          context
              .eval(
                  HaraLanguage.ID,
                  "(ns foundation-alias (:require [std.foundation :as f])) (f/apply-with 2 + 19 21)")
              .asLong());

      assertEquals(
          42L,
          context
              .eval(
                  HaraLanguage.ID,
                  "(ns foundation-refer (:require [std.foundation :refer [apply-with]])) "
                      + "(apply-with 2 + 19 21)")
              .asLong());

    }
  }

  @Test
  public void previouslyEstablishedAliasDemandsPortableDefinition() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      context.eval(HaraLanguage.ID, "(ns foundation-prior-alias) (alias f std.foundation)");
      assertTrue(context.eval(HaraLanguage.ID, "(= nil (resolve 'apply-with))").asBoolean());
      assertEquals(42L, context.eval(HaraLanguage.ID, "(f/apply-with 2 + 19 21)").asLong());
    }
  }

  @Test
  public void firstFallbackMacroReferenceMaterializesFoundation() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      assertTrue(context.eval(HaraLanguage.ID, "(= nil (resolve 'if-not))").asBoolean());
      assertEquals(42L, context.eval(HaraLanguage.ID, "(if-not false 42)").asLong());
      assertFalse(context.eval(HaraLanguage.ID, "(= nil (resolve 'if-not))").asBoolean());
    }
  }

  @Test
  public void syntaxQuotedFallbackReferenceLoadsBeforeMacroDefinition() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      assertEquals(
          9L,
          context
              .eval(
                  HaraLanguage.ID,
                  "(do (defmacro documented [value] `(identity ~value)) (documented 9))")
              .asLong());
    }
  }

  @Test
  public void portableOverridesRestoreReaderNumericAndCollectionSemantics() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      assertTrue(
          context
              .eval(HaraLanguage.ID, "(= 12.5 (read-string \"12.5\"))")
              .asBoolean());
      assertEquals(
          12.5,
          context
              .eval(HaraLanguage.ID, "(double (read-string \"12.5\"))")
              .asDouble(),
          0.0);
      assertEquals(1L, context.eval(HaraLanguage.ID, "(long 1.0)").asLong());
      assertThrows(PolyglotException.class, () -> context.eval(HaraLanguage.ID, "(long 1.9)"));
      assertEquals(
          "[:x 2 3]",
          context.eval(HaraLanguage.ID, "(str (assoc [1 2 3] 0 :x))").asString());
      assertTrue(context.eval(HaraLanguage.ID, "(has? [10 20] 1)").asBoolean());
      assertEquals(20L, context.eval(HaraLanguage.ID, "(nth [10 20] 1)").asLong());
    }
  }

  @Test
  public void selectiveNamespacePolicySurvivesLaterFallbackUse() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      context.eval(HaraLanguage.ID, "(ns startup-selective (:config {:only [inc]}))");
      assertEquals(42L, context.eval(HaraLanguage.ID, "(inc 41)").asLong());
      PolyglotException missing =
          assertThrows(PolyglotException.class, () -> context.eval(HaraLanguage.ID, "map"));
      assertTrue(missing.getMessage().contains("Unbound symbol: map"));
    }
  }

  @Test
  public void foundationChildResourceLoadsAfterTheRootPath() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      assertEquals(
          80L,
          context
              .eval(
                  HaraLanguage.ID,
                  "(do (load-resource \"std/foundation/pretty.hal\") "
                      + "(:width std.foundation.pretty/default-options))")
              .asLong());
    }
  }
}
