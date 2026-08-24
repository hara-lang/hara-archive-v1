package hara.spec;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertNotNull;
import static org.junit.Assert.assertTrue;

import hara.kernel.base.Parser;
import hara.lang.data.Keyword;
import hara.lang.protocol.ILinearType;
import hara.lang.protocol.IMapType;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.HashMap;
import java.util.HashSet;
import java.util.Map;
import java.util.Set;
import org.junit.Test;
import org.junit.experimental.categories.Category;

/** Structural checks for the draft HAL meta-spec and its first language document. */
@Category(RegistryConformance.class)
public class HalSpecificationContractTest {
  private static final Path METASPEC =
      specsRegistry().resolve("01-lang/001-language/metaspec/language-metaspec.edn");
  private static final Path LANGSPEC =
      specsRegistry().resolve("01-lang/001-language/draft/hal-langspec.edn");
  private static final Path CORE_LANGUAGE_CORPUS =
      specsRegistry().resolve("01-lang/001-language/draft/conformance/core.edn");
  private static final Path EXCEPTION_CORPUS =
      specsRegistry().resolve("01-lang/001-language/draft/conformance/exceptions.edn");
  private static final Path READER_CORPUS =
      specsRegistry().resolve("01-lang/001-language/draft/conformance/reader.edn");
  private static final Path MODULE_CORPUS =
      specsRegistry().resolve("01-lang/001-language/draft/conformance/modules.edn");
  private static final Path STACK_SAFETY_CORPUS =
      specsRegistry().resolve("01-lang/001-language/draft/conformance/stack-safety.edn");

  @Test
  public void languageDraftHasResolvableIdentityStructureAndEvidence() throws Exception {
    IMapType metaspec = readMap(METASPEC);
    IMapType langspec = readMap(LANGSPEC);

    assertEquals(
        Keyword.create("hal", "metaspec"), metaspec.lookup(key("document", "id")));
    assertEquals(
        Keyword.create("hal", "language-metaspec"),
        metaspec.lookup(key("document", "type")));
    assertEquals(
        Keyword.create("hal", "language"), langspec.lookup(key("document", "id")));
    assertEquals(
        Keyword.create("language-spec"), langspec.lookup(key("document", "type")));
    assertEquals(Keyword.create("draft"), langspec.lookup(key("document", "status")));

    IMapType conformsTo = map(langspec, key("spec", "conforms-to"));
    assertEquals(
        metaspec.lookup(key("document", "id")),
        conformsTo.lookup(key("spec", "id")));
    assertEquals(
        metaspec.lookup(key("document", "version")),
        conformsTo.lookup(key("spec", "version")));

    ILinearType sections = linear(langspec, key("spec", "sections"));
    ILinearType sectionOrder = linear(langspec, key("spec", "section-order"));
    assertEquals(sectionOrder.count(), sections.count());

    Map<Object, IMapType> sectionsById = index(sections, key("section", "id"));
    for (int i = 0; i < sectionOrder.count(); i++) {
      assertNotNull(
          "Unknown section in :spec/section-order: " + sectionOrder.nth(i),
          sectionsById.get(sectionOrder.nth(i)));
    }

    ILinearType forms = linear(langspec, key("spec", "forms"));
    Map<Object, IMapType> formsById = index(forms, key("form", "id"));
    Map<Object, Set<Object>> suiteCases =
        Map.of(
            Keyword.create("hal", "reader"), caseIds(READER_CORPUS),
            Keyword.create("hal", "core"), caseIds(CORE_LANGUAGE_CORPUS),
            Keyword.create("hal", "exceptions"), caseIds(EXCEPTION_CORPUS),
            Keyword.create("hal", "modules"), caseIds(MODULE_CORPUS),
            Keyword.create("hal", "stack-safety"), caseIds(STACK_SAFETY_CORPUS),
            Keyword.create("hal", "native"), nativeCaseIds());
    Set<Object> requirementIds = new HashSet<>();
    collectRequirements(
        linear(langspec, key("spec", "invariants")), requirementIds, suiteCases);
    for (int i = 0; i < sections.count(); i++) {
      IMapType section = (IMapType) sections.nth(i);
      collectRequirements(
          linear(section, key("section", "requirements")), requirementIds, suiteCases);
      Object sectionForms = section.lookup(key("section", "forms"));
      if (sectionForms instanceof ILinearType referencedForms) {
        for (int j = 0; j < referencedForms.count(); j++) {
          assertNotNull(
              "Unknown form in section: " + referencedForms.nth(j),
              formsById.get(referencedForms.nth(j)));
        }
      }
    }

    ILinearType references = linear(langspec, key("spec", "references"));
    Map<Object, IMapType> referencesById = index(references, key("reference", "id"));
    for (int i = 0; i < references.count(); i++) {
      IMapType reference = (IMapType) references.nth(i);
      String relativePath = (String) reference.lookup(key("reference", "path"));
      Path target = LANGSPEC.getParent().resolve(relativePath).normalize();
      assertTrue("Missing specification reference: " + target, Files.exists(target));
      if (target.startsWith(specsRegistry().resolve("99-archive/planning").normalize())) {
        assertEquals(
            "Planning archive references must be historical",
            Keyword.create("historical"),
            reference.lookup(key("reference", "authority")));
        assertEquals(
            "Planning archive references must be provenance",
            Keyword.create("provenance"),
            reference.lookup(key("reference", "kind")));
      }
    }

    assertReferencesResolve(
        linear(langspec, key("spec", "conformance")), referencesById);
    assertReferencesResolve(
        linear(langspec, key("spec", "provenance")), referencesById);
  }

  private static IMapType readMap(Path path) throws Exception {
    Object value = Parser.LispReader.readString(Files.readString(path), null);
    assertTrue("Expected EDN map: " + path, value instanceof IMapType);
    return (IMapType) value;
  }

  private static Path specsRegistry() {
    return SpecRegistry.root();
  }

  private static IMapType map(IMapType parent, Keyword key) {
    Object value = parent.lookup(key);
    assertTrue("Expected map at " + key, value instanceof IMapType);
    return (IMapType) value;
  }

  private static ILinearType linear(IMapType parent, Keyword key) {
    Object value = parent.lookup(key);
    assertTrue("Expected vector at " + key, value instanceof ILinearType);
    return (ILinearType) value;
  }

  private static Map<Object, IMapType> index(ILinearType entries, Keyword idKey) {
    Map<Object, IMapType> indexed = new HashMap<>();
    for (int i = 0; i < entries.count(); i++) {
      assertTrue("Expected map entry at index " + i, entries.nth(i) instanceof IMapType);
      IMapType entry = (IMapType) entries.nth(i);
      Object id = entry.lookup(idKey);
      assertNotNull("Missing id " + idKey + " at index " + i, id);
      assertTrue("Duplicate id: " + id, indexed.put(id, entry) == null);
    }
    return indexed;
  }

  private static Set<Object> caseIds(Path corpusPath) throws Exception {
    ILinearType cases = linear(readMap(corpusPath), Keyword.create("cases"));
    return index(cases, Keyword.create("id")).keySet();
  }

  private static Set<Object> nativeCaseIds() {
    return Set.of(
        Keyword.create("result-deref-success"), Keyword.create("result-deref-error"));
  }

  private static void collectRequirements(
      ILinearType requirements, Set<Object> ids, Map<Object, Set<Object>> suiteCases) {
    for (int i = 0; i < requirements.count(); i++) {
      IMapType requirement = (IMapType) requirements.nth(i);
      Object id = requirement.lookup(key("requirement", "id"));
      assertNotNull("Requirement is missing an id", id);
      assertTrue("Duplicate requirement id: " + id, ids.add(id));
      assertNotNull(
          "Requirement is missing a level: " + id,
          requirement.lookup(key("requirement", "level")));
      assertNotNull(
          "Requirement is missing text: " + id,
          requirement.lookup(key("requirement", "text")));
      Object evidenceValue = requirement.lookup(key("requirement", "evidence"));
      if (evidenceValue instanceof ILinearType evidence) {
        for (int j = 0; j < evidence.count(); j++) {
          IMapType link = (IMapType) evidence.nth(j);
          Object suite = link.lookup(key("conformance", "suite"));
          Set<Object> knownCases = suiteCases.get(suite);
          assertNotNull("Unknown evidence suite for " + id + ": " + suite, knownCases);
          ILinearType cases = linear(link, key("conformance", "cases"));
          for (int k = 0; k < cases.count(); k++) {
            assertTrue(
                "Unknown evidence case for " + id + ": " + suite + " " + cases.nth(k),
                knownCases.contains(cases.nth(k)));
          }
        }
      }
    }
  }

  private static void assertReferencesResolve(
      ILinearType ids, Map<Object, IMapType> referencesById) {
    for (int i = 0; i < ids.count(); i++) {
      assertNotNull(
          "Unknown specification reference: " + ids.nth(i),
          referencesById.get(ids.nth(i)));
    }
  }

  private static Keyword key(String namespace, String name) {
    return Keyword.create(namespace, name);
  }
}
