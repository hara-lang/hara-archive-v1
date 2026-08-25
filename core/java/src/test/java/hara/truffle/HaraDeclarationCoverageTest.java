package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertNotNull;
import static org.junit.Assert.assertTrue;

import hara.kernel.base.Parser;
import hara.lang.data.Keyword;
import hara.lang.data.Symbol;
import hara.lang.protocol.ILinearType;
import hara.lang.protocol.IMapType;
import hara.lang.declaration.HaraAvailability;
import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraNativeBinding;
import hara.lang.declaration.HaraProtocolBinding;
import hara.spec.SpecRegistry;
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
import java.util.regex.Matcher;
import java.util.regex.Pattern;
import org.junit.Test;

/** Checks that the Java declaration surface is closed before runtime publication. */
@org.junit.experimental.categories.Category(hara.spec.RegistryConformance.class)
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
    Map<String, ProtocolSpec> protocols =
        protocolSpecs(readMap(specsRegistry().resolve(PROTOCOLS_SPEC)));
    Map<String, NativeSpec> nativeTypes =
        nativeSpecs(readMap(specsRegistry().resolve(NATIVE_SPEC)));
    int portableProtocols =
        (int)
            protocols.values().stream()
                .filter(spec -> spec.availability == HaraAvailability.PORTABLE)
                .count();
    int capabilityProtocols = protocols.size() - portableProtocols;
    int declaredMethods =
        protocols.values().stream().mapToInt(spec -> spec.methods.size()).sum();
    assertEquals(protocols.size(), jsonNumber(json, "protocolCount"));
    assertEquals(portableProtocols, jsonNumber(json, "portableProtocolCount"));
    assertEquals(capabilityProtocols, jsonNumber(json, "capabilityProtocolCount"));
    assertEquals(declaredMethods, jsonNumber(json, "declaredMethodCount"));
    assertEquals(nativeTypes.size(), jsonNumber(json, "nativeTypeCount"));
    assertTrue(json, json.contains("\"IEncodeVisitor\""));
    assertTrue(json, json.contains("\"IStringLike\""));
  }

  @Test
  public void everySpecsProtocolHasOneAnnotatedJavaInterface() throws Exception {
    IMapType contract = readMap(specsRegistry().resolve(PROTOCOLS_SPEC));
    Map<String, ProtocolSpec> expected = protocolSpecs(contract);

    assertTrue("Java protocol closure must not be empty", !expected.isEmpty());

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
  public void runtimeDiscoveryFindsTheSameClosedProtocolSet() throws Exception {
    Map<String, Class<?>> declarations = HaraProtocolDeclarations.discover();
    Map<String, ProtocolSpec> expected =
        protocolSpecs(readMap(specsRegistry().resolve(PROTOCOLS_SPEC)));
    assertEquals(expected.keySet(), declarations.keySet());
    assertNotNull(declarations.get("IColl").getAnnotation(HaraProtocolBinding.class));
    assertNotNull(declarations.get("IMetadata").getAnnotation(HaraProtocolBinding.class));
  }

  @Test
  public void collectionAndMetadataInterfacesAreOrdinaryAnnotatedProtocols() {
    assertNotNull(hara.lang.protocol.IColl.class.getAnnotation(HaraProtocolBinding.class));
    assertNotNull(hara.lang.protocol.IMetadata.class.getAnnotation(HaraProtocolBinding.class));
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

    assertTrue("Native annotation closure must not be empty", !expected.isEmpty());
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
      assertEquals(
          "Native method surface differs for " + spec.name,
          spec.methods,
          List.of(binding.methods()));
      assertEquals(
          "Native capability differs for " + spec.name,
          spec.capability,
          binding.capability());
    }
    assertEquals(expected.keySet(), HaraNativeDeclarations.METHODS.keySet());
  }

  @Test
  public void deterministicDeclarationManifestsMatchTheSpecsRegistry() throws Exception {
    Map<String, NativeSpec> nativeTypes =
        nativeSpecs(readMap(specsRegistry().resolve(NATIVE_SPEC)));
    List<String> expectedNative =
        nativeTypes.values().stream()
            .map(
                spec ->
                    String.join(
                        "|",
                        "native",
                        "std.native." + spec.name,
                        availability(spec.availability),
                        spec.capability,
                        "annotation",
                        spec.methods.stream()
                            .map(method -> "std.native." + spec.name + "/" + method)
                            .sorted()
                            .reduce((left, right) -> left + "," + right)
                            .orElse("")))
            .sorted()
            .toList();
    assertEquals(expectedNative, HaraDeclarationManifest.nativeManifest());

    Map<String, ProtocolSpec> protocols =
        protocolSpecs(readMap(specsRegistry().resolve(PROTOCOLS_SPEC)));
    List<String> expectedProtocols =
        protocols.values().stream()
            .map(
                spec ->
                    String.join(
                        "|",
                        "protocol",
                        "std.protocol." + spec.name.toLowerCase() + "." + spec.name,
                        spec.name,
                        availability(spec.availability),
                        spec.capability,
                        "annotation",
                        spec.parents.stream().sorted().reduce((left, right) -> left + "," + right).orElse(""),
                        spec.methods.entrySet().stream()
                            .map(
                                method ->
                                    "std.protocol."
                                        + spec.name.toLowerCase()
                                        + "."
                                        + spec.name
                                        + "/"
                                        + method.getKey()
                                        + ":"
                                        + method.getValue())
                            .sorted()
                            .reduce((left, right) -> left + "," + right)
                            .orElse("")))
            .sorted()
            .toList();
    assertEquals(expectedProtocols, HaraDeclarationManifest.protocolManifest());
    assertTrue(
        expectedProtocols.stream()
            .anyMatch(line -> line.startsWith("protocol|std.protocol.icoll.IColl|")));
    assertTrue(
        expectedProtocols.stream()
            .anyMatch(line -> line.startsWith("protocol|std.protocol.imetadata.IMetadata|")));
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

  private static Map<String, NativeSpec> nativeSpecs(IMapType contract) throws Exception {
    Map<String, String> capabilities = nativeCapabilities();
    Map<String, NativeSpec> result = new LinkedHashMap<>();
    ILinearType entries = linear(contract.lookup(keyword("types")), "native :types");
    for (int index = 0; index < entries.count(); index++) {
      IMapType entry = (IMapType) entries.nth(index);
      String name = symbol(entry.lookup(keyword("name")));
      List<String> methods = symbols(entry.lookup(keyword("methods")), name + " :methods");
      String availability = ((Keyword) entry.lookup(keyword("availability"))).getName();
      HaraAvailability mapped =
          availability.equals("capability-gated")
              ? HaraAvailability.CAPABILITY_GATED
              : HaraAvailability.PORTABLE;
      String capability = capabilities.getOrDefault(name, "");
      if (mapped == HaraAvailability.CAPABILITY_GATED) {
        assertFalse("Missing native capability: " + name, capability.isBlank());
      } else {
        assertTrue("Portable native type has a capability: " + name, capability.isBlank());
      }
      assertFalse("Duplicate native type: " + name, result.containsKey(name));
      result.put(
          name,
          new NativeSpec(
              name,
              methods,
              mapped,
              capability));
    }
    return result;
  }

  private static Map<String, String> nativeCapabilities() throws Exception {
    String source = Files.readString(specsRegistry().resolve(NATIVE_RUNTIME_SPEC));
    Map<String, String> result = new LinkedHashMap<>();
    Matcher matcher =
        Pattern.compile(
                "(?s)\\{\\s*:native/id\\s+[^\\s}]+\\s+:native/symbol\\s+([^\\s}]+).*?"
                    + ":native/availability\\s+:([^\\s}]+)"
                    + "(?:\\s+:native/capability\\s+:([^\\s}]+))?.*?\\}")
            .matcher(source);
    while (matcher.find()) {
      String name = matcher.group(1);
      String capability = matcher.group(3) == null ? "" : matcher.group(3);
      assertFalse("Duplicate native runtime type: " + name, result.containsKey(name));
      result.put(name, capability);
    }
    assertTrue("Native runtime type capability inventory is empty", !result.isEmpty());
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

  private static String availability(HaraAvailability availability) {
    return switch (availability) {
      case PORTABLE -> "portable";
      case CAPABILITY_GATED -> "capability-gated";
      case INVENTORY_ONLY -> "inventory-only";
    };
  }

  private static List<String> symbols(Object value, String label) {
    ILinearType values = linear(value, label);
    List<String> output = new ArrayList<>();
    for (int index = 0; index < values.count(); index++) {
      output.add(symbol(values.nth(index)));
    }
    return List.copyOf(output);
  }

  private static Keyword keyword(String name) {
    return Keyword.create(name);
  }

  private static int jsonNumber(String json, String name) {
    Matcher matcher =
        Pattern.compile("\\\"" + Pattern.quote(name) + "\\\"\\s*:\\s*(\\d+)")
            .matcher(json);
    assertTrue("Missing compatibility count: " + name, matcher.find());
    return Integer.parseInt(matcher.group(1));
  }

  private static Path specsRegistry() {
    return SpecRegistry.root();
  }

  private record ProtocolSpec(
      String name,
      Map<String, Integer> methods,
      Set<String> parents,
      HaraAvailability availability,
      String capability) {}

  private record NativeSpec(
      String name, List<String> methods, HaraAvailability availability, String capability) {}

  private static final String PROTOCOLS_SPEC =
      "01-lang/001-language/draft/conformance/protocols.edn";
  private static final String NATIVE_SPEC =
      "01-lang/001-language/draft/conformance/native.edn";
  private static final String NATIVE_RUNTIME_SPEC =
      "01-lang/003-native/draft/native-spec.edn";
}
