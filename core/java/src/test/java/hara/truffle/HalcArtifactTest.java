package hara.truffle;

import static org.junit.Assert.assertArrayEquals;
import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertThrows;
import static org.junit.Assert.assertTrue;

import hara.lang.base.G;
import java.io.ByteArrayOutputStream;
import java.io.PrintStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import org.graalvm.polyglot.Context;
import org.junit.Test;

public class HalcArtifactTest {
  @Test
  public void schemaVarReferencesAreCheckedAndNamespaceCanonicalized() {
    String source =
        "(ns demo.schema) "
            + "(def Customer [:map [:id :int]]) "
            + "(defn ^{:schema #'-/Customer} customer-id [customer] (get customer :id))";
    Object[] forms = HaraLanguage.readAll(source, "demo/schema.hal");
    HalcArtifact.Module module =
        HalcArtifact.decode(
            HalcArtifact.encode(
                "demo.schema",
                "demo/schema.hal",
                source.getBytes(StandardCharsets.UTF_8),
                forms));
    hara.lang.data.List<?> definition = (hara.lang.data.List<?>) module.forms[2];
    hara.lang.data.Symbol definitionName = (hara.lang.data.Symbol) definition.nth(1);
    @SuppressWarnings("unchecked")
    hara.lang.protocol.IMapType<Object, Object> metadata =
        (hara.lang.protocol.IMapType<Object, Object>) definitionName.meta();
    hara.lang.data.List<?> reference =
        (hara.lang.data.List<?>) metadata.lookup(hara.lang.data.Keyword.create("schema"));
    hara.lang.data.Symbol target = (hara.lang.data.Symbol) reference.nth(1);
    assertEquals("demo.schema/Customer", target.display());

    String missing =
        "(ns demo.schema) (defn ^{:schema #'MissingSchema} invalid [value] value)";
    Object[] missingForms = HaraLanguage.readAll(missing, "demo/schema.hal");
    HaraException error =
        assertThrows(
            HaraException.class,
            () ->
                HalcArtifact.encode(
                    "demo.schema",
                    "demo/schema.hal",
                    missing.getBytes(StandardCharsets.UTF_8),
                    missingForms));
    assertTrue(error.getMessage().contains("schema Var does not exist: MissingSchema"));
  }

  @Test
  public void nestedSchemaVarReferencesAreCanonicalizedAndChecked() {
    String source =
        "(ns demo.schema) "
            + "(def Address [:map [:street :str]]) "
            + "(def Customer [:map [:address #'-/Address]]) "
            + "(defn ^{:schema #'Customer} save [customer] customer)";
    HalcArtifact.Module module =
        HalcArtifact.decode(
            HalcArtifact.encode(
                "demo.schema",
                "demo/schema.hal",
                source.getBytes(StandardCharsets.UTF_8),
                HaraLanguage.readAll(source, "demo/schema.hal")));
    assertTrue(G.display(module.forms[2]).contains("(var demo.schema/Address)"));
    assertEquals(1, module.schemas.functions.size());
    assertTrue(module.schemas.functions.containsKey("demo.schema/save"));
    assertEquals(2, module.schemas.definitions.size());
    assertTrue(module.schemas.definitions.containsKey("demo.schema/Address"));
    assertTrue(module.schemas.definitions.containsKey("demo.schema/Customer"));
    assertTrue(
        module.schemas.resolvedFunctionType("demo.schema/save") instanceof HalcSchema.MapType);

    String missing =
        "(ns demo.schema) "
            + "(def Customer [:map [:address #'MissingAddress]]) "
            + "(defn ^{:schema #'Customer} save [customer] customer)";
    HaraException error =
        assertThrows(
            HaraException.class,
            () ->
                HalcArtifact.encode(
                    "demo.schema",
                    "demo/schema.hal",
                    missing.getBytes(StandardCharsets.UTF_8),
                    HaraLanguage.readAll(missing, "demo/schema.hal")));
    assertTrue(error.getMessage().contains("schema Var does not exist: MissingAddress"));

    String recursive =
        "(ns demo.schema) "
            + "(def Node [:map [:children [:vector #'Node]]]) "
            + "(defn ^{:schema #'Node} walk [node] node)";
    HalcArtifact.encode(
        "demo.schema",
        "demo/schema.hal",
        recursive.getBytes(StandardCharsets.UTF_8),
        HaraLanguage.readAll(recursive, "demo/schema.hal"));

    String malformed =
        "(ns demo.schema) "
            + "(def Customer [:map [:name]]) "
            + "(defn ^{:schema #'Customer} save [customer] customer)";
    HaraException malformedError =
        assertThrows(
            HaraException.class,
            () ->
                HalcArtifact.encode(
                    "demo.schema",
                    "demo/schema.hal",
                    malformed.getBytes(StandardCharsets.UTF_8),
                    HaraLanguage.readAll(malformed, "demo/schema.hal")));
    assertTrue(
        malformedError
            .getMessage()
            .contains(
                "invalid schema demo.schema/Customer: :map schema fields must be [name type] or [name properties type]"));
  }

  @Test
  public void mapsAndSetsEncodeInCanonicalOrder() {
    byte[] mapA =
        HalcArtifact.encode(
            "t",
            "t",
            new byte[0],
            new Object[] {
              hara.lang.data.Map.Standard.from(null, new Object[] {"b", 2L, "a", 1L, "c", 3L})
            });
    byte[] mapB =
        HalcArtifact.encode(
            "t",
            "t",
            new byte[0],
            new Object[] {
              hara.lang.data.Map.Standard.from(null, new Object[] {"c", 3L, "a", 1L, "b", 2L})
            });
    assertArrayEquals(mapA, mapB);

    byte[] setA =
        HalcArtifact.encode(
            "t",
            "t",
            new byte[0],
            new Object[] {hara.lang.data.Set.Standard.from(null, new Object[] {3L, 1L, 2L})});
    byte[] setB =
        HalcArtifact.encode(
            "t",
            "t",
            new byte[0],
            new Object[] {hara.lang.data.Set.Standard.from(null, new Object[] {2L, 3L, 1L})});
    assertArrayEquals(setA, setB);

    // Entry order in the payload follows the canonical encoded-byte order, not
    // the host map/set iteration order. For longs 1, 100, -1 the canonical order
    // is 1 < 100 < -1 (unsigned lexicographic on the 8-byte big-endian encoding).
    byte[] one = {3, 0, 0, 0, 0, 0, 0, 0, 1};
    byte[] hundred = {3, 0, 0, 0, 0, 0, 0, 0, 100};
    byte[] minusOne = {3, -1, -1, -1, -1, -1, -1, -1, -1};

    byte[] mapEncoded =
        HalcArtifact.encode(
            "t",
            "t",
            new byte[0],
            new Object[] {
              hara.lang.data.Map.Standard.from(
                  null, new Object[] {1L, "a", -1L, "b", 100L, "c"})
            });
    assertTrue(indexOf(mapEncoded, one) >= 0);
    assertTrue(indexOf(mapEncoded, one) < indexOf(mapEncoded, hundred));
    assertTrue(indexOf(mapEncoded, hundred) < indexOf(mapEncoded, minusOne));

    byte[] setEncoded =
        HalcArtifact.encode(
            "t",
            "t",
            new byte[0],
            new Object[] {hara.lang.data.Set.Standard.from(null, new Object[] {1L, -1L, 100L})});
    assertTrue(indexOf(setEncoded, one) >= 0);
    assertTrue(indexOf(setEncoded, one) < indexOf(setEncoded, hundred));
    assertTrue(indexOf(setEncoded, hundred) < indexOf(setEncoded, minusOne));

    // Ordered collections keep insertion order: it is semantic there.
    byte[] orderedA =
        HalcArtifact.encode(
            "t",
            "t",
            new byte[0],
            new Object[] {
              hara.lang.data.OrderedMap.Standard.from(null, new Object[] {"b", 2L, "a", 1L})
            });
    byte[] orderedB =
        HalcArtifact.encode(
            "t",
            "t",
            new byte[0],
            new Object[] {
              hara.lang.data.OrderedMap.Standard.from(null, new Object[] {"a", 1L, "b", 2L})
            });
    assertTrue(!java.util.Arrays.equals(orderedA, orderedB));
  }

  private static int indexOf(byte[] haystack, byte[] needle) {
    outer:
    for (int i = 0; i + needle.length <= haystack.length; i++) {
      for (int j = 0; j < needle.length; j++) {
        if (haystack[i + j] != needle[j]) continue outer;
      }
      return i;
    }
    return -1;
  }

  @Test
  public void regexValuesRoundTripPortably() {
    java.util.regex.Pattern pattern = java.util.regex.Pattern.compile("a+b");
    HalcArtifact.Module module =
        HalcArtifact.decode(HalcArtifact.encode("t", "t", new byte[0], new Object[] {pattern}));
    assertTrue(module.forms[0] instanceof java.util.regex.Pattern);
    assertEquals("a+b", ((java.util.regex.Pattern) module.forms[0]).pattern());

    java.util.regex.Pattern flagged =
        java.util.regex.Pattern.compile("a+b", java.util.regex.Pattern.CASE_INSENSITIVE);
    HaraException error =
        assertThrows(
            HaraException.class,
            () -> HalcArtifact.encode("t", "t", new byte[0], new Object[] {flagged}));
    assertTrue(error.getMessage().contains("regex flags"));
  }

  @Test
  public void goldenBytesLockThePortableFormat() {
    // One form per opcode (0-16, with the historical opcode 7 slot unused). Any change to the byte layout, the opcode
    // numbering, or the canonical collection ordering must update this golden
    // value and the registry's 01-lang/009-halc/draft/halc-format.md together.
    Object[] forms =
        new Object[] {
          null,
          false,
          true,
          42L,
          2.5d,
          new java.math.BigInteger("123456789012345678901234567890"),
          "hárà",
          'x',
          hara.lang.data.Symbol.create("my.ns", "my-sym"),
          hara.lang.data.Keyword.create("kw"),
          hara.lang.data.List.Standard.from(null, new Object[] {1L, "a"}),
          hara.lang.data.Vector.Standard.from(null, new Object[] {1L, "a"}),
          hara.lang.data.Map.Standard.from(null, new Object[] {2L, "b", 1L, "a"}),
          hara.lang.data.Set.Standard.from(null, new Object[] {2L, 1L}),
          hara.lang.data.OrderedMap.Standard.from(null, new Object[] {2L, "b", 1L, "a"}),
          hara.lang.data.OrderedSet.Standard.from(null, new Object[] {2L, 1L}),
          java.util.regex.Pattern.compile("a+b")
        };
    byte[] expected =
        hexBytes(
            "48414c43000100010000013f57211e103028689092d59627fbba64015c289acd1bc5b2e7be27ec53d8bf4c35"
                + "00000001740000000174e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
                + "0000001100010203000000000000002a044004000000000000050000001e313233343536373839303132"
                + "333435363738393031323334353637383930060000000668c3a172c3a008000000780901000000056d792e6e73"
                + "000000066d792d73796d000a00000000026b77000b00000002030000000000000001060000000161000c00000002"
                + "030000000000000001060000000161000d00000002030000000000000001060000000161030000000000000002"
                + "060000000162000e00000002030000000000000001030000000000000002000f00000002030000000000000002"
                + "060000000162030000000000000001060000000161001000000002030000000000000002030000000000000001"
                + "001100000003612b62");
    byte[] encoded = HalcArtifact.encode("t", "t", new byte[0], forms);
    assertArrayEquals(expected, encoded);

    // The golden artifact must also remain decodable.
    HalcArtifact.Module module = HalcArtifact.decode(expected);
    assertEquals(HalcArtifact.Origin.HALC, module.origin);
    assertEquals("t", module.namespace);
    assertEquals(17, module.forms.length);
    assertEquals("a+b", ((java.util.regex.Pattern) module.forms[16]).pattern());
  }

  @Test
  public void legacyHirMagicDecodesButEncodingAlwaysUsesHalcMagic() {
    byte[] halc = HalcArtifact.encode("t", "t", new byte[0], new Object[] {42L});
    byte[] legacy = halc.clone();
    legacy[0] = 'H';
    legacy[1] = 'I';
    legacy[2] = 'R';
    legacy[3] = 0;

    assertEquals(HalcArtifact.Origin.LEGACY_HIR, HalcArtifact.decode(legacy).origin);
    assertArrayEquals(new byte[] {'H', 'A', 'L', 'C'}, java.util.Arrays.copyOf(halc, 4));
  }

  private static byte[] hexBytes(String hex) {
    byte[] bytes = new byte[hex.length() / 2];
    for (int i = 0; i < bytes.length; i++) {
      bytes[i] = (byte) Integer.parseInt(hex.substring(i * 2, i * 2 + 2), 16);
    }
    return bytes;
  }

  @Test
  public void foundationArtifactIsDeterministicAndRoundTripsForms() throws Exception {
    Path source = Path.of("lib/src/std/foundation.hal");
    byte[] sourceBytes = Files.readAllBytes(source);
    Object[] forms =
        HaraLanguage.readAll(
            Files.readString(source, StandardCharsets.UTF_8),
            HalcArtifact.FOUNDATION_RESOURCE);

    byte[] first =
        HalcArtifact.encode(
            "std.foundation", HalcArtifact.FOUNDATION_RESOURCE, sourceBytes, forms);
    byte[] second =
        HalcArtifact.encode(
            "std.foundation", HalcArtifact.FOUNDATION_RESOURCE, sourceBytes, forms);
    assertArrayEquals(first, second);

    HalcArtifact.Module decoded = HalcArtifact.decode(first);
    assertEquals("std.foundation", decoded.namespace);
    assertEquals(HalcArtifact.FOUNDATION_RESOURCE, decoded.resource);
    assertEquals(forms.length, decoded.forms.length);
    for (int index = 0; index < forms.length; index++) {
      assertEquals(G.display(forms[index]), G.display(decoded.forms[index]));
    }
  }

  @Test
  public void rejectsCorruptAndTruncatedArtifacts() throws Exception {
    byte[] source = "(ns std.foundation)".getBytes(StandardCharsets.UTF_8);
    Object[] forms =
        HaraLanguage.readAll(new String(source, StandardCharsets.UTF_8), "foundation.hal");
    byte[] artifact =
        HalcArtifact.encode(
            "std.foundation", HalcArtifact.FOUNDATION_RESOURCE, source, forms);

    byte[] corrupt = artifact.clone();
    corrupt[corrupt.length - 1] ^= 1;
    assertTrue(
        assertThrows(HaraException.class, () -> HalcArtifact.decode(corrupt))
            .getMessage()
            .contains("checksum"));

    byte[] truncated = java.util.Arrays.copyOf(artifact, artifact.length - 1);
    assertTrue(
        assertThrows(HaraException.class, () -> HalcArtifact.decode(truncated))
            .getMessage()
            .contains("truncated"));

    byte[] missingExecutableFlag = artifact.clone();
    missingExecutableFlag[6] = 0;
    missingExecutableFlag[7] = 0;
    assertTrue(
        assertThrows(HaraException.class, () -> HalcArtifact.decode(missingExecutableFlag))
            .getMessage()
            .contains("unsupported flags"));
  }

  @Test
  public void compileCommandWritesLoadableFoundationArtifact() throws Exception {
    Path output = Files.createTempFile("foundation-", ".halc");
    try {
      ByteArrayOutputStream stdout = new ByteArrayOutputStream();
      ByteArrayOutputStream stderr = new ByteArrayOutputStream();
      int status =
          Main.run(
              new String[] {
                "compile-halc",
                "lib/src/std/foundation.hal",
                "--output",
                output.toString()
              },
              new PrintStream(stdout, true, StandardCharsets.UTF_8),
              new PrintStream(stderr, true, StandardCharsets.UTF_8));
      assertEquals(stderr.toString(StandardCharsets.UTF_8), 0, status);
      assertEquals("std.foundation", HalcArtifact.decode(Files.readAllBytes(output)).namespace);
    } finally {
      Files.deleteIfExists(output);
    }
  }

  @Test
  @org.junit.experimental.categories.Category(hara.spec.RegistryConformance.class)
  public void sharedCrossRuntimeGoldensDecode() throws Exception {
    Path root =
        hara.spec.SpecRegistry.resolve("01-lang/009-halc/draft/conformance/golden");
    HalcArtifact.Module current = HalcArtifact.decode(Files.readAllBytes(root.resolve("complete.halc")));
    assertEquals(HalcArtifact.Origin.HALC, current.origin);
    assertEquals("halc.conformance.complete", current.namespace);
    assertEquals("conformance/complete.hal", current.resource);
    assertEquals(
        HalcArtifact.Origin.LEGACY_HIR,
        HalcArtifact.decode(Files.readAllBytes(root.resolve("legacy-v1.hir"))).origin);
  }

  @Test
  @org.junit.experimental.categories.Category(hara.spec.RegistryConformance.class)
  public void registryGoldenMatchesJavaEncoding() throws Exception {
    Path root = hara.spec.SpecRegistry.resolve("01-lang/009-halc/draft/conformance");
    Path sourcePath = root.resolve("complete.hal");
    String source = Files.readString(sourcePath, StandardCharsets.UTF_8);
    byte[] encoded =
        HalcArtifact.encode(
            "halc.conformance.complete",
            "conformance/complete.hal",
            source.getBytes(StandardCharsets.UTF_8),
            HaraLanguage.readAll(source, "conformance/complete.hal"));
    assertArrayEquals(
        Files.readAllBytes(root.resolve("golden/complete.halc")), encoded);
  }

  @Test
  public void embeddedHbxSupersedesLegacyFoundationHalcMode() {
    String previous = System.getProperty("hara.HalcMode");
    try {
      System.setProperty("hara.HalcMode", "strict");
      long before = FoundationHalcLowerer.compilationCount();
      assertFoundation();
      assertEquals(before, FoundationHalcLowerer.compilationCount());
      System.setProperty("hara.HalcMode", "off");
      assertFoundation();
      assertEquals(before, FoundationHalcLowerer.compilationCount());
    } finally {
      if (previous == null) System.clearProperty("hara.HalcMode");
      else System.setProperty("hara.HalcMode", previous);
    }
  }

  private static void assertFoundation() {
    try (Context context =
        Context.newBuilder(HaraLanguage.ID)
            .option("engine.WarnInterpreterOnly", "false")
            .build()) {
      assertEquals(
          42,
          context
              .eval(HaraLanguage.ID, "((std.foundation/comp inc inc) 40)")
              .asLong());
      assertEquals(
          "[2 3]",
          context
              .eval(HaraLanguage.ID, "(vec (std.foundation/map inc [1 2]))")
              .toString());
      assertEquals(
          "[42 {:a {:b 42}} [0 1 2 3] [7 7 7] [2 3] (9 7)]",
          context
              .eval(
                  HaraLanguage.ID,
                  "[(get-in {:a {:b 42}} [:a :b])"
                      + " (assoc-in {} [:a :b] 42)"
                      + " (vec (range 4))"
                      + " (vec (repeat 3 7))"
                      + " (vec ((map inc) [1 2]))"
                      + " ((juxt inc dec) 8)]")
              .toString());
      assertEquals(
          "[[f g] [f g h] [f g h l & more]]",
          context.eval(HaraLanguage.ID, "(:arglists (meta (var comp)))").toString());
      assertEquals(
          42,
          context.eval(HaraLanguage.ID, "((comp inc inc inc inc) 38)").asLong());
    }
  }
}
