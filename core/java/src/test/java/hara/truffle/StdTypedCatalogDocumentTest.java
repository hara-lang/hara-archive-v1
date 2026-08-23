package hara.truffle;

import static org.junit.Assert.assertEquals;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import org.graalvm.polyglot.Context;
import org.junit.Test;

public class StdTypedCatalogDocumentTest {
  private static final String FIXTURE_PATH =
      "01-lang/011-typed-catalog/draft/conformance/catalog-v1.json";
  private static final String ACCOUNT_HASH =
      "sha256:87304c8b522dece5a0fb44ba26a9489435fd8bb79c3f28640733dbfe81ffb65f";
  private static final String NODE_HASH =
      "sha256:66c65596b131761401ba324de8adcd1252f49dbabea566277acf46ed13f2d7f0";
  private static final String ACCOUNT_COMPONENT =
      "sha256:2932e3b99be0a918adc4adc939bef7c0966c77a0007b86afd9a47fe732d7f01d";
  private static final String STALE_HASH =
      "sha256:07304c8b522dece5a0fb44ba26a9489435fd8bb79c3f28640733dbfe81ffb65f";
  private static final String STALE_NODE_HASH =
      "sha256:06c65596b131761401ba324de8adcd1252f49dbabea566277acf46ed13f2d7f0";
  private static final String FORGED_COMPONENT =
      "sha256:0932e3b99be0a918adc4adc939bef7c0966c77a0007b86afd9a47fe732d7f01d";

  private static Path fixturePath() {
    Path nested = Path.of("hara-specs-registry").resolve(FIXTURE_PATH);
    if (Files.isRegularFile(nested)) {
      return nested;
    }
    return Path.of("..", "hara-specs-registry").resolve(FIXTURE_PATH);
  }

  private static String fixtureText() throws IOException {
    return Files.readString(fixturePath(), StandardCharsets.UTF_8);
  }

  private static String haraString(String value) {
    StringBuilder output = new StringBuilder(value.length() + 2).append('"');
    for (int index = 0; index < value.length(); index++) {
      char character = value.charAt(index);
      switch (character) {
        case '\\' -> output.append("\\\\");
        case '"' -> output.append("\\\"");
        case '\n' -> output.append("\\n");
        case '\r' -> output.append("\\r");
        case '\t' -> output.append("\\t");
        default -> output.append(character);
      }
    }
    return output.append('"').toString();
  }

  private static String replaceFirstN(
      String source, String target, String replacement, int count) {
    StringBuilder output = new StringBuilder(source);
    int offset = 0;
    for (int index = 0; index < count; index++) {
      int found = output.indexOf(target, offset);
      if (found < 0) {
        throw new IllegalArgumentException(
            "Expected " + count + " occurrences of " + target);
      }
      output.replace(found, found + target.length(), replacement);
      offset = found + replacement.length();
    }
    return output.toString();
  }

  private static String evaluate(String source) {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      return context.eval(HaraLanguage.ID, source).asString();
    }
  }

  private static String verifyExpression(String fixture, String body) {
    return evaluate(
        "(ns std-typed-catalog-document-truffle-probe "
            + "  (:require [std.typed :as typed])) "
            + "(require 'std.typed.catalog.document {:reload true}) "
            + "(require 'std.typed {:reload true}) "
            + "(let [verified (typed/catalog-document-verify-json "
            + haraString(fixture)
            + ")] "
            + "  (pr-str "
            + body
            + "))");
  }

  private static String rejectionExpression(String fixture) {
    return evaluate(
        "(ns std-typed-catalog-document-truffle-rejection "
            + "  (:require [std.typed :as typed])) "
            + "(require 'std.typed.catalog.document {:reload true}) "
            + "(require 'std.typed {:reload true}) "
            + "(pr-str "
            + " (try "
            + "   (typed/catalog-document-verify-json "
            + haraString(fixture)
            + ") "
            + "   :unexpected-success "
            + "   (catch Throwable error "
            + "     [(:type (ex-data error)) "
            + "      (:finding/type (ex-data error)) "
            + "      (:cause/type (ex-data error))])))");
  }

  private static String rejectionByCodeExpression(String fixture) {
    return evaluate(
        "(ns std-typed-catalog-document-truffle-code-rejection "
            + "  (:require [std.typed :as typed])) "
            + "(require 'std.typed.catalog.document {:reload true}) "
            + "(require 'std.typed {:reload true}) "
            + "(pr-str "
            + " (try "
            + "   (typed/catalog-document-verify-json "
            + haraString(fixture)
            + ") "
            + "   :unexpected-success "
            + "   (catch :hara/argument error "
            + "     [(:cause/type (ex-data error)) "
            + "      (:finding/type (ex-data error))])))");
  }

  @Test
  public void exactRegistryBytesRoundTripThroughStdTypedCatalog() throws IOException {
    assertEquals(
        "[:verified 5 5 true true true true]",
        verifyExpression(
            fixtureText(),
            "(let [report (:verification verified) "
                + "      admitted (:catalog verified) "
                + "      account-coordinate (first (:catalog/coordinates report)) "
                + "      account (typed/catalog-resolve admitted account-coordinate) "
                + "      order (typed/catalog-dependency-order admitted)] "
                + "  [(:status report) "
                + "   (:catalog/entry-count report) "
                + "   (:catalog/component-count report) "
                + "   (:catalog/latest-tooling-only? report) "
                + "   (= account-coordinate (:schema/coordinate account)) "
                + "   (= 5 (count order)) "
                + "   (= (:catalog/component-order report) "
                + "      (vec "
                + "       (map :component/id "
                + "            (typed/catalog-dependency-components admitted))))])"));
  }

  @Test
  public void stalePublishedHashIsRejectedByCanonicalCatalog() throws IOException {
    String fixture = replaceFirstN(fixtureText(), ACCOUNT_HASH, STALE_HASH, 2);
    assertEquals(
        "[:std.typed.catalog/invalid-document "
            + ":std.typed.catalog.document/catalog-rejected "
            + ":std.typed.catalog/invalid-catalog]",
        rejectionExpression(fixture));
  }

  @Test
  public void hbcRejectionMatchesStructuredCodeAndPreservesData() throws IOException {
    String fixture = replaceFirstN(fixtureText(), ACCOUNT_HASH, STALE_HASH, 2);
    assertEquals(
        "[:std.typed.catalog/invalid-catalog :std.typed.catalog.document/catalog-rejected]",
        rejectionByCodeExpression(fixture));
  }

  @Test
  public void staleExactDependencyIsRejectedAfterRecomputation() throws IOException {
    String fixture = replaceFirstN(fixtureText(), NODE_HASH, STALE_NODE_HASH, 1);
    assertEquals(
        "[:std.typed.catalog/invalid-document "
            + ":std.typed.catalog.document/schema-dependencies-mismatch nil]",
        rejectionExpression(fixture));
  }

  @Test
  public void forgedComponentEvidenceIsRejectedAfterRecomputation()
      throws IOException {
    String fixture =
        replaceFirstN(
            fixtureText(), ACCOUNT_COMPONENT, FORGED_COMPONENT, 2);
    assertEquals(
        "[:std.typed.catalog/invalid-document "
            + ":std.typed.catalog.document/component-evidence-mismatch nil]",
        rejectionExpression(fixture));
  }

  @Test
  public void unsupportedHashEpochFailsBeforeCatalogAdmission()
      throws IOException {
    String fixture =
        replaceFirstN(
            fixtureText(),
            "\"std.typed.schema/catalog-v1\"",
            "\"std.typed.schema/catalog-v2\"",
            1);
    assertEquals(
        "[:std.typed.catalog/invalid-document "
            + ":std.typed.catalog.document/hash-epoch-unsupported nil]",
        rejectionExpression(fixture));
  }
}
