package hara.spec;

import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;
import static org.junit.Assume.assumeTrue;

import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import java.util.ServiceLoader;
import hara.kernel.base.Parser;
import hara.lang.data.Keyword;
import hara.lang.protocol.IMapType;
import hara.truffle.HaraLibraryProvider;
import org.junit.Test;
import org.junit.experimental.categories.Category;

/** Verifies that active specification documents and published examples describe the current slice. */
public class DocumentationContractTest {
  private static final List<String> ACTIVE_SPECIFICATION_PATHS =
      List.of(
          "01-lang/000-metaspec/draft/metaspec-metaspec.edn",
          "01-lang/001-language/metaspec/language-metaspec.edn",
          "01-lang/001-language/draft/hal-langspec.edn");

  private static void assumeSpecsSubmodule() {
    assumeTrue(
        "hara-specs-registry is unavailable; set HARA_SPECS_REGISTRY",
        SpecRegistry.available());
  }

  private static void assumeDocsSubmodule() {
    assumeTrue(
        "docs submodule not initialized: git submodule update --init docs",
        Files.isRegularFile(Path.of("docs/docs/user-guide.md")));
  }

  @Test
  @Category(RegistryConformance.class)
  public void activeSpecificationsAreReadableEdnWithRenderedCompanions() throws Exception {
    assumeSpecsSubmodule();
    for (String relative : ACTIVE_SPECIFICATION_PATHS) {
      Path specification = SpecRegistry.resolve(relative);
      assertTrue("Missing active specification: " + specification, Files.exists(specification));
      Object document =
          Parser.LispReader.readString(
              Files.readString(specification, StandardCharsets.UTF_8), null);
      assertTrue("Specification is not an EDN map: " + specification, document instanceof IMapType);
      IMapType map = (IMapType) document;
      assertTrue(
          "Missing document id: " + specification,
          map.lookup(Keyword.create("document", "id")) != null);
      assertTrue(
          "Missing document status: " + specification,
          map.lookup(Keyword.create("document", "status")) != null);

      Path companion = specification.resolveSibling("README.md");
      assertTrue("Missing rendered companion: " + companion, Files.exists(companion));
      assertTrue(
          "Rendered companion does not link the authoritative EDN: " + companion,
          Files.readString(companion, StandardCharsets.UTF_8)
              .contains(specification.getFileName().toString()));
    }
  }

  @Test
  public void publishedExamplesUseSupportedMarkerSyntaxAndExistingFiles() throws Exception {
    assumeDocsSubmodule();
    String userGuide = Files.readString(Path.of("docs/docs/user-guide.md"), StandardCharsets.UTF_8);
    assertTrue(userGuide.contains("(Arr/push-last a 4)"));
    assertTrue(userGuide.contains("(Arr/get a 3)"));
    assertFalse(userGuide.contains("(array:push-last"));
    assertFalse(userGuide.contains("(array:get"));
    assertTrue(Files.exists(Path.of("lib/examples/hello.hal")));
    assertFalse(Files.exists(Path.of("lib/examples/hello.hara")));
  }

  @Test
  public void exampleProjectsUseCurrentEdnManifests() throws Exception {
    Path examples = Path.of("lib/examples");
    try (var paths = Files.walk(examples)) {
      assertFalse(
          "Legacy project manifest remains under " + examples,
          paths.anyMatch(
              path ->
                  "project.hal".equals(path.getFileName().toString())
                      || "project.hara".equals(path.getFileName().toString())));
    }
    assertTrue(Files.isRegularFile(examples.resolve("cluster/project.edn")));
    assertTrue(Files.isRegularFile(examples.resolve("services/project.edn")));
  }

  @Test
  public void namespaceCatalogTracksEveryRegisteredProvider() throws Exception {
    assumeDocsSubmodule();
    String catalog =
        Files.readString(Path.of("docs/docs/reference/namespaces.md"), StandardCharsets.UTF_8);
    int providers = 0;
    for (HaraLibraryProvider provider : ServiceLoader.load(HaraLibraryProvider.class)) {
      providers++;
      assertTrue(
          "Missing provider namespace from catalog: " + provider.namespace(),
          catalog.contains("`" + provider.namespace() + "`"));
    }
    assertTrue("No Hara library providers were discovered", providers > 0);
  }

  @Test
  public void currentGuidesUseCurrentNamespaceAndLauncherConventions() throws Exception {
    assumeDocsSubmodule();
    List<Path> currentGuides =
        List.of(
            Path.of("README.md"),
            Path.of("GETTING_STARTED.md"),
            Path.of("docs/docs/namespaces.md"),
            Path.of("docs/docs/user-guide.md"),
            Path.of("docs/docs/reference/namespaces.md"),
            Path.of("lib/examples/code-test/README.md"));
    for (Path guide : currentGuides) {
      String content = Files.readString(guide, StandardCharsets.UTF_8);
      assertFalse("Stale launcher in " + guide, content.contains("truffle-hara"));
      assertFalse("Stale project descriptor in " + guide, content.contains("project.hara"));
    }
    assertTrue(Files.readString(Path.of("hara")).contains("\"$@\""));
  }
}
