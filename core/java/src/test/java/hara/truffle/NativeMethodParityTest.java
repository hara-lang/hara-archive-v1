package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertNotNull;
import static org.junit.Assert.assertTrue;

import hara.kernel.base.Parser;
import hara.lang.data.Keyword;
import hara.lang.data.Symbol;
import hara.lang.protocol.ILinearType;
import hara.lang.protocol.IMapType;
import hara.spec.SpecRegistry;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import org.graalvm.polyglot.Context;
import org.junit.Test;

/** Keeps the closed std.native inventory and its direct method surface aligned with the spec. */
@org.junit.experimental.categories.Category(hara.spec.RegistryConformance.class)
public class NativeMethodParityTest {
  private static final Path CONTRACT =
      specsRegistry()
          .resolve("01-lang/001-language/draft/conformance/native.edn");
  private static final Path FIXTURE =
      specsRegistry()
          .resolve(
              "01-lang/001-language/draft/conformance/fixtures/native_behavioral.hal");

  @Test
  public void nativeInventoryIsClosedAndClassified() throws Exception {
    IMapType contract = readMap(CONTRACT);
    IMapType inventory = map(contract, "inventory");
    Map<String, NativeTypeSpec> types = types(contract);

    assertEquals(Boolean.TRUE, inventory.lookup(keyword("closed")));
    assertEquals("Native type count must be derived", null, inventory.lookup(keyword("type-count")));
    assertEquals("Native method count must be derived", null, inventory.lookup(keyword("method-count")));
    assertNotNull("Native count derivation policy is required", inventory.lookup(keyword("counting")));

    Map<String, List<String>> runtimeTypes = HaraNativeDeclarations.METHODS;
    Map<String, List<String>> specifiedTypes = new LinkedHashMap<>();
    types.forEach((name, type) -> specifiedTypes.put(name, type.methods));
    assertEquals("Truffle native inventory differs from native.edn", specifiedTypes, runtimeTypes);

    for (NativeTypeSpec type : types.values()) {
      assertTrue(
          "Unsupported availability classification for " + type.name,
          Set.of("implemented", "capability-gated").contains(type.availability));
      Set<String> classified = new LinkedHashSet<>(type.halWrappers);
      for (String primitive : type.foundationPrimitives) {
        assertTrue("Duplicate method classification: " + type.name + "/" + primitive,
            classified.add(primitive));
      }
      for (String nativeOnly : type.nativeOnly) {
        assertTrue("Duplicate method classification: " + type.name + "/" + nativeOnly,
            classified.add(nativeOnly));
      }
      assertEquals(
          "Every native method must have exactly one Foundation exposure: " + type.name,
          new LinkedHashSet<>(type.methods),
          classified);
      if (!type.halWrappers.isEmpty()) {
        assertNotNull("HAL wrappers require a source: " + type.name, type.wrapperSource);
        Path wrapperSource = resolveWrapperSource(type.wrapperSource);
        String source = Files.readString(wrapperSource);
        for (String method : type.halWrappers) {
          assertTrue(
              "Missing HAL wrapper call " + type.name + "/" + method,
              source.contains(type.name + "/" + method));
        }
      }
    }
  }

  @Test
  public void languageBuiltinAccountingMatchesTheSharedContract() throws Exception {
    IMapType builtins = map(readMap(CONTRACT), "language-builtins");
    Map<String, List<String>> runtime = HaraBuiltinCatalog.LANGUAGE_BUILTINS;
    Map<String, List<String>> specified = new LinkedHashMap<>();
    for (String category : List.of("evaluation", "definitions", "namespaces", "interop")) {
      specified.put(category, symbols(builtins.lookup(keyword(category)), category));
    }
    assertEquals("Truffle builtin accounting differs from native.edn", specified, runtime);
    assertTrue("Builtins must not be a native type", !types(readMap(CONTRACT)).containsKey("Builtins"));
  }

  @Test
  public void everySpecifiedNativeMethodHasOnePassingSpecsOwnedClassification() throws Exception {
    Map<String, NativeTypeSpec> types = types(readMap(CONTRACT));
    String corpus = Files.readString(FIXTURE);

    try (Context context = Context.newBuilder(HaraLanguage.ID).allowAllAccess(true).build()) {
      String result =
          context.eval(HaraLanguage.ID, corpus + "\n(native-method-results)").toString();
      assertTrue(result, !result.contains(":pass false"));
      assertEquals(
          types.values().stream().mapToInt(type -> type.methods.size()).sum(),
          result.split(":pass true", -1).length - 1);
      assertEquals(
          calibrationExpected(context, corpus, "exact-error-class"),
          context
              .eval(
                  HaraLanguage.ID,
                  corpus + "\n" + calibrationSource(context, corpus, "exact-error-class"))
              .toString());
    }
  }

  @Test
  public void nativeTypeObjectsAndAliasesAreUniversalIncludingBlankNamespaces() throws Exception {
    String corpus = Files.readString(FIXTURE);
    try (Context context = Context.newBuilder(HaraLanguage.ID).allowAllAccess(true).build()) {
      assertEquals(
          calibrationExpected(context, corpus, "blank-namespace-alias"),
          context
              .eval(
                  HaraLanguage.ID,
                  corpus + "\n" + calibrationSource(context, corpus, "blank-namespace-alias"))
              .toString());
    }
  }

  private static String calibrationSource(Context context, String corpus, String name) {
    return context
        .eval(
            HaraLanguage.ID,
            corpus + "\n(get (get native-calibration-snippets :" + name + ") :source)")
        .asString();
  }

  private static String calibrationExpected(Context context, String corpus, String name) {
    return context
        .eval(
            HaraLanguage.ID,
            corpus + "\n(get (get native-calibration-snippets :" + name + ") :expected)")
        .toString();
  }

  private static Map<String, NativeTypeSpec> types(IMapType contract) {
    ILinearType entries = linear(contract.lookup(keyword("types")), ":types");
    Map<String, NativeTypeSpec> output = new LinkedHashMap<>();
    for (int index = 0; index < entries.count(); index++) {
      IMapType entry = (IMapType) entries.nth(index);
      String name = ((Symbol) entry.lookup(keyword("name"))).getName();
      List<String> methods = symbols(entry.lookup(keyword("methods")), name + " :methods");
      assertEquals(
          "Duplicate methods declared for " + name,
          new LinkedHashSet<>(methods).size(),
          methods.size());
      String availability = ((Keyword) entry.lookup(keyword("availability"))).getName();
      IMapType classification = map(entry, "method-classification");
      List<String> halWrappers = classified(classification.lookup(keyword("hal-wrapper")), methods);
      List<String> primitives =
          classified(classification.lookup(keyword("foundation-primitive")), methods);
      List<String> nativeOnly =
          classified(classification.lookup(keyword("native-only")), methods);
      String wrapperSource = (String) entry.lookup(keyword("wrapper-source"));
      assertTrue("Duplicate native type: " + name, !output.containsKey(name));
      output.put(
          name,
          new NativeTypeSpec(
              name, methods, availability, halWrappers, primitives, nativeOnly, wrapperSource));
    }
    return output;
  }

  private static List<String> classified(Object value, List<String> all) {
    if (value == null) return List.of();
    if (value instanceof Keyword marker && "all".equals(marker.getName())) {
      return all;
    }
    return symbols(value, "method classification");
  }

  private static List<String> symbols(Object value, String label) {
    ILinearType values = linear(value, label);
    List<String> output = new ArrayList<>();
    for (int index = 0; index < values.count(); index++) {
      output.add(((Symbol) values.nth(index)).getName());
    }
    return List.copyOf(output);
  }

  private static IMapType readMap(Path path) throws Exception {
    Object value = Parser.LispReader.readString(Files.readString(path), null);
    assertTrue("Expected EDN map: " + path, value instanceof IMapType);
    return (IMapType) value;
  }

  private static IMapType map(IMapType parent, String name) {
    Object value = parent.lookup(keyword(name));
    assertTrue("Expected map at :" + name, value instanceof IMapType);
    return (IMapType) value;
  }

  private static ILinearType linear(Object value, String label) {
    assertTrue("Expected vector at " + label, value instanceof ILinearType);
    return (ILinearType) value;
  }

  private static Keyword keyword(String name) {
    return Keyword.create(name);
  }

  private static Path specsRegistry() {
    return SpecRegistry.root();
  }

  private static Path resolveWrapperSource(String source) {
    Path directory = Path.of("").toAbsolutePath().normalize();
    while (directory != null) {
      for (Path candidate :
          List.of(directory.resolve(source), directory.resolve("core").resolve(source))) {
        if (Files.isRegularFile(candidate)) return candidate;
      }
      directory = directory.getParent();
    }
    throw new IllegalStateException("Missing HAL wrapper source: " + source);
  }

  private record NativeTypeSpec(
      String name,
      List<String> methods,
      String availability,
      List<String> halWrappers,
      List<String> foundationPrimitives,
      List<String> nativeOnly,
      String wrapperSource) {}
}
