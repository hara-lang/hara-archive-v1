package hara.truffle;

import hara.spec.SpecRegistry;
import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertNotNull;
import static org.junit.Assert.assertTrue;

import java.nio.file.Files;
import java.nio.file.Path;
import org.graalvm.polyglot.Context;
import org.junit.Test;

public class EvaluationJournalTest {
  @Test
  @org.junit.experimental.categories.Category(hara.spec.RegistryConformance.class)
  public void consumesSharedCorpusAndRecordsPortableNestedOperations() throws Exception {
    String corpus =
        Files.readString(
            SpecRegistry.resolve(
                "00-unsorted/diagnostics/draft/conformance/evaluation-journal.edn"));
    assertEquals(1, HaraLanguage.readAll(corpus, "evaluation-journal.edn").length);

    try (Context context = context()) {
      EvaluationJournal.Journal journal =
          EvaluationJournal.collect(
              () ->
                  context
                      .eval(
                          HaraLanguage.ID,
                          "(do (defn inner [x] (+ x 1)) (defn outer [x] (inner x)) (outer 41))")
                      .asLong());
      assertEquals(EvaluationJournal.SCHEMA, journal.schema());
      assertEquals("ok", journal.status());
      assertEquals("integer", journal.result().type());
      assertEquals("42", journal.result().display());
      assertEquals(
          EvaluationJournal.SCHEMA,
          journal.portableData().get(hara.lang.data.Keyword.create("journal/schema")));
      assertTrue(journal.events().stream().anyMatch(event -> event.kind().equals("operation/enter")));
      assertTrue(
          journal.events().stream()
              .anyMatch(event -> event.kind().equals("operation/enter") && event.parent() != null));
    }
  }

  @Test
  public void recordsMacrosFailuresAndBoundedUnicodePreviews() {
    try (Context context = context()) {
      EvaluationJournal.Journal macro =
          EvaluationJournal.collect(
              new EvaluationJournal.Limits(100, 20, 4),
              () -> context.eval(HaraLanguage.ID, "(if-not false \"abcdef\")").asString());
      assertTrue(macro.events().stream().anyMatch(event -> event.kind().equals("macro/expand")));
      assertEquals("abc…", macro.result().display());
      assertTrue(macro.result().truncated());

      EvaluationJournal.Journal failure =
          EvaluationJournal.collect(() -> context.eval(HaraLanguage.ID, "(/ 1 0)"));
      assertEquals("error", failure.status());
      assertNotNull(failure.error());
      assertEquals("evaluation/error", failure.events().get(failure.events().size() - 1).kind());
    }
  }

  @Test
  public void truncationDoesNotStopEvaluationAndDisabledResultsAreEquivalent() {
    try (Context context = context()) {
      EvaluationJournal.Journal journal =
          EvaluationJournal.collect(
              new EvaluationJournal.Limits(1, 100, 100),
              () -> context.eval(HaraLanguage.ID, "(def journal-value 42)"));
      assertEquals("truncated", journal.status());
      assertEquals(42, context.eval(HaraLanguage.ID, "journal-value").asLong());
      assertEquals(42, context.eval(HaraLanguage.ID, "(let [x 19] (+ x 23))").asLong());
    }
  }

  private static Context context() {
    return Context.newBuilder(HaraLanguage.ID)
        .option("engine.WarnInterpreterOnly", "false")
        .build();
  }
}
