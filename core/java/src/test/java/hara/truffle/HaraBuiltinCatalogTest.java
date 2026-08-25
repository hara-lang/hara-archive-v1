package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertThrows;
import static org.junit.Assert.assertTrue;

import java.util.List;
import java.util.Set;
import org.junit.Test;

/** Verifies the immutable catalog boundary used by {@link HaraContext} bootstrap. */
public class HaraBuiltinCatalogTest {
  @Test
  public void catalogContainsTheSeparateLanguageAndNativeSurfaces() {
    assertEquals(
        Set.of("evaluation", "definitions", "namespaces", "interop"),
        HaraBuiltinCatalog.LANGUAGE_BUILTINS.keySet());
    assertTrue(HaraBuiltinCatalog.SPECIAL_SYMBOLS.contains("def"));
    assertFalse(HaraBuiltinCatalog.SPECIAL_SYMBOLS.contains("std.foundation/def"));
    assertEquals(
        "std.foundation.string", HaraBuiltinCatalog.GENERATED_LIBRARIES.get("string"));
    assertEquals("str", HaraBuiltinCatalog.DEFAULT_LIBRARY_ALIASES.get("string"));
    assertTrue(HaraBuiltinCatalog.MARKER_METHOD_NAMES.contains("get"));
    assertTrue(HaraNativeDeclarations.METHODS.containsKey("Kernel"));
    assertTrue(
        HaraNativeDeclarations.bindings().stream()
            .anyMatch(binding -> binding.name().equals("String")));
  }

  @Test
  public void catalogMapsAndMethodListsCannotBeMutated() {
    assertThrows(
        UnsupportedOperationException.class,
        () -> HaraNativeDeclarations.METHODS.put("Unexpected", List.of("method")));
    assertThrows(
        UnsupportedOperationException.class,
        () -> HaraNativeDeclarations.METHODS.get("Kernel").add("unexpected"));
    assertThrows(
        UnsupportedOperationException.class,
        () -> HaraBuiltinCatalog.LANGUAGE_BUILTINS.put("unexpected", List.of("form")));
  }

  @Test
  public void eachNativeTypeHasOneEntryPerMethodName() {
    assertTrue(HaraNativeDeclarations.METHODS.size() > 0);
    HaraNativeDeclarations.METHODS.forEach(
        (type, methods) -> {
          assertTrue(type + " must have native methods", !methods.isEmpty());
          assertEquals(type + " contains duplicate methods", methods.size(), Set.copyOf(methods).size());
        });
  }

  @Test
  public void libraryProvidersAreUniqueAnnotatedAndDoNotAdvertiseDirectlyInstalledEdn() {
    assertFalse(new HaraLibraryLoader().provides("std.native.Edn"));

    HaraLibraryProvider duplicate =
        new HaraLibraryProvider() {
          @Override
          public String namespace() {
            return "std.native.String";
          }

          @Override
          public void install(HaraContext context) {}
        };
    assertThrows(
        HaraException.class, () -> new HaraLibraryLoader(List.of(duplicate, duplicate)));

    HaraLibraryProvider unannotated =
        new HaraLibraryProvider() {
          @Override
          public String namespace() {
            return "std.native.Unannotated";
          }

          @Override
          public void install(HaraContext context) {}
        };
    assertThrows(HaraException.class, () -> new HaraLibraryLoader(List.of(unannotated)));
  }
}
