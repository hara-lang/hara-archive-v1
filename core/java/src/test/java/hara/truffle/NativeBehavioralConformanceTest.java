package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.stream.Collectors;
import hara.spec.SpecRegistry;
import org.graalvm.polyglot.Context;
import org.graalvm.polyglot.Value;
import org.junit.Test;

/** Runs the specs-owned native behavioral corpus against the Truffle runtime. */
@org.junit.experimental.categories.Category(hara.spec.RegistryConformance.class)
public class NativeBehavioralConformanceTest {
  private static final Path CORPUS =
      specsRegistry()
          .resolve(
              "01-lang/001-language/draft/conformance/fixtures/native_behavioral.hal");

  @Test
  public void sharedCorpusClosesOverTheLiveManifestAndRejectsDrift() throws Exception {
    String corpus = Files.readString(CORPUS);
    Set<String> live = liveMethods();
    try (Context context = Context.newBuilder(HaraLanguage.ID).allowAllAccess(true).build()) {
      assertEquals(
          "true",
          context.eval(HaraLanguage.ID, corpus + "\n(native-corpus-valid?)").toString());
      Set<String> classified =
          methodSet(context.eval(HaraLanguage.ID, corpus + "\n(native-method-keys)"));
      assertEquals(live, classified);
      assertClosure(context, corpus, live, true);

      String first = classified.iterator().next();
      Set<String> removed = new LinkedHashSet<>(classified);
      removed.remove(first);
      assertClosure(context, corpus, removed, false);

      Set<String> added = new LinkedHashSet<>(classified);
      added.add("Unclassified/addition");
      assertClosure(context, corpus, added, false);

      Set<String> renamed = new LinkedHashSet<>(classified);
      renamed.remove(first);
      renamed.add(first + "-renamed");
      assertClosure(context, corpus, renamed, false);
    }
  }

  @Test
  public void truffleRunsEverySpecsOwnedClassificationBoundaryAndProfile() throws Exception {
    String corpus = Files.readString(CORPUS);
    try (Context context = Context.newBuilder(HaraLanguage.ID).allowAllAccess(true).build()) {
      Set<String> methods =
          methodSet(context.eval(HaraLanguage.ID, corpus + "\n(native-method-keys)"));
      String results =
          context.eval(HaraLanguage.ID, corpus + "\n(native-method-results)").toString();
      assertTrue(results, !results.contains(":pass false"));
      assertEquals(methods.size(), results.split(":pass true", -1).length - 1);

      assertTrue(
          context
              .eval(
                  HaraLanguage.ID,
                  corpus
                      + "\n(every? (fn [case] (= true (get case :pass))) "
                      + "(native-boundary-results))")
              .asBoolean());
      assertTrue(
          context
              .eval(
                  HaraLanguage.ID,
                  corpus
                      + "\n(let [report (native-profile-report)] "
                      + "(and (= 0 (get report :failed)) "
                      + "(= (+ (get report :passed) (get report :failed) (get report :skipped)) "
                      + "(+ (get report :portable) (get report :capability-specific) "
                      + "(get report :inventory-only)))))")
              .asBoolean());
    }
  }

  private static void assertClosure(
      Context context, String corpus, Set<String> live, boolean expected) {
    String literal =
        live.stream().map(method -> "'" + method).collect(Collectors.joining(" ", "[", "]"));
    assertEquals(
        expected,
        context
            .eval(
                HaraLanguage.ID,
                corpus + "\n(get (native-closure-report " + literal + ") :pass)")
            .asBoolean());
  }

  private static Set<String> methodSet(Value value) {
    assertTrue("native method keys must be a vector", value.hasArrayElements());
    Set<String> methods = new LinkedHashSet<>();
    for (long index = 0; index < value.getArraySize(); index++) {
      String method = value.getArrayElement(index).toString();
      assertTrue("duplicate native corpus method " + method, methods.add(method));
    }
    assertFalse("the native corpus must not be empty", methods.isEmpty());
    return methods;
  }

  private static Set<String> liveMethods() {
    Map<String, List<String>> runtimeTypes = HaraNativeDeclarations.METHODS;
    Set<String> methods = new LinkedHashSet<>();
    for (Map.Entry<String, List<String>> type : runtimeTypes.entrySet()) {
      for (String method : type.getValue()) {
        assertTrue(
            "Duplicate live native method: " + type.getKey() + "/" + method,
            methods.add(type.getKey() + "/" + method));
      }
    }
    return methods;
  }

  private static Path specsRegistry() {
    return SpecRegistry.root();
  }
}
