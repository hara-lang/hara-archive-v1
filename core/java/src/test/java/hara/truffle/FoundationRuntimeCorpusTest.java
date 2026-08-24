package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertTrue;

import hara.kernel.base.Parser;
import hara.lang.data.Keyword;
import hara.lang.data.Symbol;
import hara.lang.protocol.ILinearType;
import hara.lang.protocol.IMapType;
import hara.spec.SpecRegistry;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.HashSet;
import java.util.Set;
import org.graalvm.polyglot.Context;
import org.junit.Test;
import org.junit.experimental.categories.Category;

/** Executes the specs-owned Foundation surface and behavioral corpus. */
@Category(hara.spec.RegistryConformance.class)
public class FoundationRuntimeCorpusTest {
  private static final Path REPOSITORY = repositoryRoot();
  private static final Path FOUNDATION =
      specsRegistry().resolve("01-lang/004-foundation/draft/conformance");
  private static final Path SURFACE = FOUNDATION.resolve("foundation-surface.edn");
  private static final Path CORPUS =
      FOUNDATION.resolve("fixtures/foundation_behavioral.hal");
  private static final Path FOUNDATION_SOURCE =
      REPOSITORY.resolve("core/lib/src/std/foundation");

  @Test
  @SuppressWarnings("rawtypes")
  public void specsOwnedFoundationCorpusClosesAndPassesOnTheJvm() throws Exception {
    Set<String> specified = surfaceSymbols();
    String source =
        String.join(
                "\n",
                Files.readString(FOUNDATION_SOURCE.resolve("bytes.hal")),
                Files.readString(FOUNDATION_SOURCE.resolve("coroutine.hal")),
                Files.readString(FOUNDATION_SOURCE.resolve("pretty.hal")),
                Files.readString(FOUNDATION_SOURCE.resolve("promise.hal")),
                Files.readString(FOUNDATION_SOURCE.resolve("string.hal")),
                Files.readString(CORPUS))
            + """

            (let [report (foundation-host-assertion-report)
                  profile (:profile report)]
              {:corpus-valid (:corpus-valid report)
               :calibration-failed (:calibration-failed report)
               :boundary-failed (:boundary-failed report)
               :portable (:portable profile)
               :capability-specific (:capability-specific profile)
               :inventory-only (:inventory-only profile)
               :passed (:passed profile)
               :failed (:failed profile)
               :skipped (:skipped profile)
               :vars foundation-source-vars})
            """;

    IMapType report;
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      report =
          (IMapType)
              Parser.LispReader.readString(
                  context.eval(HaraLanguage.ID, source).toString(), null);
    }

    assertEquals(true, report.lookup(keyword("corpus-valid")));
    assertEquals(0L, report.lookup(keyword("calibration-failed")));
    assertEquals(0L, report.lookup(keyword("boundary-failed")));
    assertEquals(0L, report.lookup(keyword("failed")));
    long portable = ((Number) report.lookup(keyword("portable"))).longValue();
    long capability = ((Number) report.lookup(keyword("capability-specific"))).longValue();
    long inventory = ((Number) report.lookup(keyword("inventory-only"))).longValue();
    assertEquals(specified.size(), portable + capability + inventory);
    assertEquals(portable, ((Number) report.lookup(keyword("passed"))).longValue());
    assertEquals(
        capability + inventory,
        ((Number) report.lookup(keyword("skipped"))).longValue());

    Set<String> corpusSymbols = new HashSet<>();
    for (Object symbol : (ILinearType) report.lookup(keyword("vars"))) {
      String display = ((Symbol) symbol).display();
      assertTrue("duplicate corpus symbol " + display, corpusSymbols.add(display));
    }
    assertEquals(specified, corpusSymbols);
  }

  @SuppressWarnings("rawtypes")
  private static Set<String> surfaceSymbols() throws Exception {
    IMapType surface =
        (IMapType) Parser.LispReader.readString(Files.readString(SURFACE), null);
    assertEquals(Keyword.create("specs-owned"), surface.lookup(keyword("authority")));
    Set<String> symbols = new HashSet<>();
    for (Object rawNamespace : (ILinearType) surface.lookup(keyword("namespaces"))) {
      IMapType namespace = (IMapType) rawNamespace;
      String namespaceName = ((Symbol) namespace.lookup(keyword("namespace"))).display();
      for (Object rawVar : (ILinearType) namespace.lookup(keyword("vars"))) {
        IMapType var = (IMapType) rawVar;
        String symbol =
            namespaceName + "/" + ((Symbol) var.lookup(keyword("name"))).display();
        assertTrue("duplicate surface symbol " + symbol, symbols.add(symbol));
      }
    }
    return symbols;
  }

  private static Keyword keyword(String name) {
    return Keyword.create(name);
  }

  private static Path specsRegistry() {
    return SpecRegistry.root();
  }

  private static Path repositoryRoot() {
    Path candidate = Path.of("").toAbsolutePath();
    while (candidate != null) {
      if (Files.isDirectory(candidate.resolve("core/lib/src/std"))) return candidate;
      candidate = candidate.getParent();
    }
    throw new IllegalStateException("cannot locate the Hara repository root");
  }
}
