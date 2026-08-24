package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertNotNull;
import static org.junit.Assert.assertTrue;

import hara.kernel.base.Parser;
import hara.lang.data.Keyword;
import hara.lang.data.Symbol;
import hara.lang.data.types.ILinearType;
import hara.lang.data.types.IMapType;
import hara.lang.declaration.HaraAvailability;
import hara.lang.declaration.HaraHostSupport;
import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraNativeBinding;
import hara.lang.declaration.HaraProtocolBinding;
import java.io.InputStream;
import java.lang.reflect.Method;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.HashMap;
import java.util.HashSet;
import java.util.Iterator;
import java.util.LinkedHashMap;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;
import org.junit.Test;

/** Checks that the Java declaration surface is closed before runtime publication. */
public class HaraDeclarationCoverageTest {
  private static final String PROTOCOL_PACKAGE = "hara.lang.protocol.";
  private static final String NATIVE_NAMESPACE = "std.native";
  private static final String CAPABILITY = "native-runtime-protocols";

  @Test
  public void compatibilitySnapshotHasTheExpectedClosedCounts() throws Exception {
    String json;
    try (InputStream input =
        HaraDeclarationCoverageTest.class.getResourceAsStream(
            "/hara/declaration/protocol-native-compatibility.json")) {
      assertNotNull("compatibility snapshot is missing", input);
      json = new String(input.readAllBytes());
    }
    assertTrue(json, json.contains("\"protocolCount\": 70"));
    assertTrue(json, json.contains("\"declaredMethodCount\": 125"));
    assertTrue(json, json.contains("\"nativeTypeCount\": 33"));
    assertTrue(json, json.contains("\"IEncodeVisitor\""));
    assertTrue(json, json.contains("\"IStringLike\""));
  }

  @Test
  public void everySpecsProtocolHasOneAnnotatedJavaInterface() throws Exception {
    IMapType contract = readMap(specsRegistry().resolve(PROTOCOLS_SPEC));
    Map<String, ProtocolSpec> expected = protocolSpecs(contract);

    assertEquals("Java protocol closure must equal the specs inventory", 70, expected.size());

    for (ProtocolSpec spec : expected.values()) {
      Class<?> type = Class.forName(PROTOCOL_PACKAGE + spec.name);
      HaraProtocolBinding binding = type.getAnnotation(HaraProtocolBinding.class);
      assertNotNull("Missing protocol annotation: " + spec.name, binding);
      assertTrue("Protocol must be an interface: " + spec.name, type.isInterface());
      assertEquals(spec.name, binding.name());
      assertEquals("std.protocol." + spec.name.toLowerCase(), binding.namespace());
      assertEquals(spec.availability, binding.availability());
      assertEquals(spec.capability, binding.capability());
      assertEquals(spec.parents, Set.copyOf(Arrays.asList(binding.parents())));

      Set<String> javaParents = new LinkedHashSet<>();
      for (Class<?> parent : type.getInterfaces()) {
        if (parent.getPackageName().equals("hara.lang.protocol")) {
          javaParents.add(parent.getSimpleName());
        }
      }
      assertEquals("Java parent surface differs for " + spec.name, spec.parents, javaParents);

      Map<String, HaraMethod> methods = new LinkedHashMap<>();
      for (Method method : type.getDeclaredMethods()) {
        HaraMethod annotation = method.getAnnotation(HaraMethod.class);
        if (annotation == null) continue;
        assertFalse(
            "Duplicate Hara method " + spec.name + "/" + annotation.value(),
            methods.containsKey(annotation.value()));
        methods.put(annotation.value(), annotation);
      }
      assertEquals("Method surface differs for " + spec.name, spec.methods.keySet(), methods.keySet());
      for (Map.Entry<String, Integer> entry : spec.methods.entrySet()) {
        HaraMethod method = methods.get(entry.getKey());
        assertEquals(spec.name + "/" + entry.getKey(), entry.getValue().intValue(), method.arity());
        assertEquals(
            spec.name + "/" + entry.getKey(),
            entry.getValue() == -1,
            method.variadic());
      }
    }
  }

  @Test
  public void runtimeDiscoveryFindsTheSameClosedProtocolSet() {
    assertEquals(70, HaraProtocolDeclarations.discover().size());
  }

  @Test
  public void hostAndKernelInterfacesCannotAccidentallyBecomeGuestProtocols() {
    HaraHostSupport collection = hara.lang.protocol.IColl.class.getAnnotation(HaraHostSupport.class);
    HaraHostSupport metadata = hara.lang.protocol.IMetadata.class.getAnnotation(HaraHostSupport.class);
    assertNotNull(collection);
    assertNotNull(metadata);
    assertFalse(hara.kernel.protocol.IEnv.class.isAnnotationPresent(HaraProtocolBinding.class));
    assertFalse(hara.kernel.protocol.IRuntime.class.isAnnotationPresent(HaraProtocolBinding.class));
    assertFalse(hara.kernel.protocol.IRedirect.class.isAnnotationPresent(HaraProtocolBinding.class));
  }

  @Test
  public void nativeAnnotationsCoverTheClosedCatalogExactlyOnce() throws Exception {
    IMapType contract = readMap(specsRegistry().resolve(NATIVE_SPEC));
    Map<String, NativeSpec> expected = nativeSpecs(contract);
    HaraNativeBinding[] bindings =
        HaraBuiltinCatalog.class.getAnnotationsByType(HaraNativeBinding.class);

    assertEquals("Native annotation closure must equal native.edn", 33, expected.size());
    assertEquals(expected.size(), bindings.length);

    Map<String, HaraNativeBinding> actual = new LinkedHashMap<>();
    for (HaraNativeBinding binding : bindings) {
      assertEquals(NATIVE_NAMESPACE, binding.namespace());
      assertFalse("Duplicate native binding: " + binding.name(), actual.containsKey(binding.name()));
      actual.put(binding.name(), binding);
    }
    assertEquals(expected.keySet(), actual.keySet());
    for (NativeSpec spec : expected.values()) {
      HaraNativeBinding binding = actual.get(spec.name);
      assertEquals(spec.availability, binding.availability());
    }
    assertEquals(expected.keySet(), HaraBuiltinCatalog.NATIVE_METHODS.keySet());
  }

  private static Map<String, ProtocolSpec> protocolSpecs(IMapType contract) {
    Map<String, ProtocolSpec> result = new LinkedHashMap<>();
    readProtocolSection(contract, "protocols", HaraAvailability.PORTABLE, "", result);
    readProtocolSection(contract, "capability-protocols", HaraAvailability.CAPABILITY_GATED, CAPABILITY, result);
    return result;
  }

  private static void readProtocolSection(
      IMapType contract,
      String section,
      HaraAvailability availability,
      String capability,
      Map<String, ProtocolSpec> result) {
    ILinearType entries = linear(contract.lookup(keyword(section)), section);
    for (int index = 0; index < entries.count(); index++) {
      IMapType entry = (IMapType) entries.nth(index);
      String name = symbol(entry.lookup(keyword("name")));
      Map<String, Integer> methods = new LinkedHashMap<>();
      IMapType methodMap = (IMapType) entry.lookup(keyword("methods"));
      Iterator<?> keys = methodMap.keys();
      Iterator<?> vals = methodMap.vals();
      while (keys.hasNext()) {
        methods.put(symbol(keys.next()), ((Number) vals.next()).intValue());
      }
      Set<String> parents = new LinkedHashSet<>();
      Object parentValue = entry.lookup(keyword("extends"));
      if (parentValue != null) {
        ILinearType parentList = linear(parentValue, name + " :extends");
        for (int parentIndex = 0; parentIndex < parentList.count(); parentIndex++) {
          parents.add(symbol(parentList.nth(parentIndex)));
        }
      }
      assertFalse("Duplicate protocol: " + name, result.containsKey(name));
      result.put(name, new ProtocolSpec(name, methods, parents, availability, capability));
    }
  }

  private static Map<String, NativeSpec> nativeSpecs(IMapType contract) {
    Map<String, NativeSpec> result = new LinkedHashMap<>();
    ILinearType entries = linear(contract.lookup(keyword("types")), "native :types");
    for (int index = 0; index < entries.count(); index++) {
      IMapType entry = (IMapType) entries.nth(index);
      String name = symbol(entry.lookup(keyword("name")));
      String availability = ((Keyword) entry.lookup(keyword("availability"))).getName();
      HaraAvailability mapped =
          availability.equals("capability-gated")
              ? HaraAvailability.CAPABILITY_GATED
              : HaraAvailability.PORTABLE;
      assertFalse("Duplicate native type: " + name, result.containsKey(name));
      result.put(name, new NativeSpec(name, mapped));
    }
    return result;
  }

  private static IMapType readMap(Path path) throws Exception {
    Object value = Parser.LispReader.readString(Files.readString(path), null);
    assertTrue("Expected EDN map: " + path, value instanceof IMapType);
    return (IMapType) value;
  }

  private static ILinearType linear(Object value, String label) {
    assertTrue("Expected vector at " + label, value instanceof ILinearType);
    return (ILinearType) value;
  }

  private static String symbol(Object value) {
    assertTrue("Expected symbol, got " + value, value instanceof Symbol);
    return ((Symbol) value).getName();
  }

  private static Keyword keyword(String name) {
    return Keyword.create(name);
  }

  private static Path specsRegistry() {
    String override = System.getenv("HARA_SPECS_REGISTRY");
    if (override != null && !override.isBlank()) return Path.of(override);
    for (Path candidate : List.of(Path.of("../../hara-specs-registry"), Path.of("../hara-specs-registry"))) {
      if (Files.isDirectory(candidate)) return candidate;
    }
    return Path.of("../../hara-specs-registry");
  }

  private record ProtocolSpec(
      String name,
      Map<String, Integer> methods,
      Set<String> parents,
      HaraAvailability availability,
      String capability) {}

  private record NativeSpec(String name, HaraAvailability availability) {}

  private static final String PROTOCOLS_SPEC =
      "01-lang/001-language/draft/conformance/protocols.edn";
  private static final String NATIVE_SPEC =
      "01-lang/001-language/draft/conformance/native.edn";
}
