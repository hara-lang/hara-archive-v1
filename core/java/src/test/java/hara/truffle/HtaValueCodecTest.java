package hara.truffle;

import static org.junit.Assert.assertArrayEquals;
import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertThrows;
import static org.junit.Assert.assertTrue;

import hara.kernel.base.Parser;
import hara.lang.data.Keyword;
import hara.lang.protocol.ILinearType;
import hara.lang.protocol.IMapType;
import hara.spec.SpecRegistry;
import java.math.BigInteger;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.regex.Pattern;
import org.junit.Test;

public class HtaValueCodecTest {
  @Test
  public void rejectsNonFiniteFloatsAtTheHtaBoundary() {
    for (double value :
        new double[] {Double.NaN, Double.POSITIVE_INFINITY, Double.NEGATIVE_INFINITY}) {
      assertThrows(HaraException.class, () -> HtaValueCodec.encode(value));
    }
    for (double value :
        new double[] {Double.NaN, Double.POSITIVE_INFINITY, Double.NEGATIVE_INFINITY}) {
      byte[] frame = {'H', 'T', 'A', '0', 15, 0, 0, 0, 0, 0, 0, 0, 0};
      long bits = Double.doubleToRawLongBits(value);
      for (int index = 0; index < 8; index++) {
        frame[5 + index] = (byte) (bits >>> (56 - (index * 8)));
      }
      assertThrows(HaraException.class, () -> HtaValueCodec.decode(frame));
    }
  }

  @Test
  public void canonicalizesBigIntegerWireWidthsAndRejectsNoncanonicalText() {
    for (BigInteger value :
        List.of(
            BigInteger.valueOf(Long.MIN_VALUE),
            BigInteger.valueOf(42),
            BigInteger.valueOf(Long.MAX_VALUE))) {
      byte[] encoded = HtaValueCodec.encode(value);
      assertEquals(3, Byte.toUnsignedInt(encoded[4]));
    }
    byte[] large = HtaValueCodec.encode(BigInteger.ONE.shiftLeft(63));
    assertEquals(20, Byte.toUnsignedInt(large[4]));

    byte[] noncanonical = {'H', 'T', 'A', '0', 20, 0, 0, 0, 2, '4', '2'};
    HaraException error =
        assertThrows(HaraException.class, () -> HtaValueCodec.decodeCanonical(noncanonical));
    assertTrue(error.getMessage().startsWith("hta/value-noncanonical:"));
  }

  @Test
  public void encodesTheAlphaHtaGoldenVector() {
    byte[] encoded = HtaValueCodec.encode(List.of("x", 42L, true));
    assertArrayEquals(
        new byte[] {
          'H', 'T', 'A', '0', 9, 0, 0, 0, 3, 4, 0, 0, 0, 1, 'x', 3, 0, 0, 0, 0, 0, 0, 0, 42, 2
        },
        encoded);
    assertEquals(List.of("x", 42L, true), HtaValueCodec.decode(encoded));
  }

  @Test
  public void scalarRegexAndPointerTagsMatchThePortableGoldenVectors() {
    assertArrayEquals(
        new byte[] {'H', 'T', 'A', '0', 19, 0, 0, 3, (byte) 0xbb},
        HtaValueCodec.encode('λ'));
    assertEquals(
        hara.lang.data.HaraCharacter.of('λ'),
        HtaValueCodec.decode(HtaValueCodec.encode(hara.lang.data.HaraCharacter.of('λ'))));

    assertArrayEquals(
        new byte[] {'H', 'T', 'A', '0', 22, 0, 0, 0, 2, 'a', '+'},
        HtaValueCodec.encode(Pattern.compile("a+")));
    assertEquals("a+", ((Pattern) HtaValueCodec.decode(HtaValueCodec.encode(Pattern.compile("a+")))).pattern());

    Object pointer =
        new hara.lang.data.Pointer(
            Keyword.create("kernel"), Map.of(Keyword.create("id"), "ROOT"));
    assertArrayEquals(
        new byte[] {
          'H', 'T', 'A', '0', 34, 6, 0, 0, 0, 6, 'k', 'e', 'r', 'n', 'e', 'l',
          11, 0, 0, 0, 1, 6, 0, 0, 0, 2, 'i', 'd', 4, 0, 0, 0, 4, 'R', 'O', 'O', 'T'
        },
        HtaValueCodec.encode(pointer));
    assertEquals(pointer, HtaValueCodec.decodeCanonical(HtaValueCodec.encode(pointer)));
  }

  @Test
  @org.junit.experimental.categories.Category(hara.spec.RegistryConformance.class)
  public void registryGoldenVectorMatchesJavaEncoding() throws Exception {
    IMapType testCase = registryCase("golden-vector");
    ILinearType input = linear(testCase.lookup(Keyword.create("case", "input")));
    ILinearType expected = linear(testCase.lookup(Keyword.create("case", "expect")));
    List<Object> values = new ArrayList<>();
    for (int index = 0; index < input.count(); index++) values.add(input.nth(index));
    byte[] expectedBytes = new byte[Math.toIntExact(expected.count())];
    for (int index = 0; index < expected.count(); index++) {
      expectedBytes[index] = ((Number) expected.nth(index)).byteValue();
    }
    assertArrayEquals(expectedBytes, HtaValueCodec.encode(values));
  }

  @Test
  @org.junit.experimental.categories.Category(hara.spec.RegistryConformance.class)
  public void registryMapEntryMatchesJavaEncodingAndRejectsWrongArity() throws Exception {
    IMapType testCase = registryCase("map-entry");
    ILinearType input = linear(testCase.lookup(Keyword.create("case", "input")));
    IMapType expectation = (IMapType) testCase.lookup(Keyword.create("case", "expect"));
    byte[] expectedBytes = bytes(linear(expectation.lookup(Keyword.create("bytes"))));
    Object entry = new hara.lang.data.MapEntry<>(null, input.nth(0), input.nth(1));

    assertArrayEquals(expectedBytes, HtaValueCodec.encode(entry));
    Object decoded = HtaValueCodec.decodeCanonical(expectedBytes);
    assertTrue(decoded instanceof hara.lang.data.MapEntry<?, ?>);
    assertEquals(entry, decoded);

    byte[] zero = {'H', 'T', 'A', '0', 38, 0, 0, 0, 0};
    byte[] one = Arrays.copyOf(expectedBytes, 17);
    one[8] = 1;
    byte[] three = Arrays.copyOf(expectedBytes, expectedBytes.length + 1);
    three[8] = 3;
    for (byte[] malformed : List.of(zero, one, three)) {
      HaraException error = assertThrows(HaraException.class, () -> HtaValueCodec.decode(malformed));
      assertTrue(error.getMessage().startsWith("hta/value-malformed: map entry"));
    }
  }

  @Test
  public void mapEncodingIsCanonical() {
    Map<Object, Object> left = new LinkedHashMap<>();
    left.put(Keyword.create("b"), 2L);
    left.put(Keyword.create("a"), 1L);
    Map<Object, Object> right = new LinkedHashMap<>();
    right.put(Keyword.create("a"), 1L);
    right.put(Keyword.create("b"), 2L);
    assertArrayEquals(HtaValueCodec.encode(left), HtaValueCodec.encode(right));
  }

  @Test
  public void rejectsTrailingAndTruncatedFrames() {
    byte[] valid = HtaValueCodec.encode("ok");
    assertThrows(
        HaraException.class, () -> HtaValueCodec.decode(Arrays.copyOf(valid, valid.length - 1)));
    assertThrows(
        HaraException.class, () -> HtaValueCodec.decode(Arrays.copyOf(valid, valid.length + 1)));
  }

  @Test
  public void rejectsImpossibleContainerLengthsAndExcessiveNesting() {
    byte[] impossible = {'H', 'T', 'A', '0', 9, 127, -1, -1, -1};
    assertThrows(HaraException.class, () -> HtaValueCodec.decode(impossible));

    Object nested = "leaf";
    for (int i = 0; i <= 256; i++) nested = List.of(nested);
    Object tooDeep = nested;
    assertThrows(HaraException.class, () -> HtaValueCodec.encode(tooDeep));
  }

  @Test
  public void opaqueHandlesRoundTripAndCannotBeReencodedAfterRelease() {
    HtaHandle handle = new HtaHandle("runtime", "cursor", 42L);
    HtaHandle decoded = (HtaHandle) HtaValueCodec.decode(HtaValueCodec.encode(handle));
    assertEquals("runtime", decoded.owner());
    assertEquals("cursor", decoded.type());
    assertEquals(42L, decoded.id());
    assertEquals("#ht[:handle 42]", decoded.toString());
    decoded.displayAs("math", "tensor");
    assertEquals("#math[:tensor 42]", decoded.toString());
    decoded.close();
    assertThrows(HaraException.class, () -> HtaValueCodec.encode(decoded));
  }
  @Test
  public void structsRoundTripAndMutableValuesAreRejected() throws Exception {
    HaraType type = new HaraType("demo/Point", new String[] {"x", "y"});
    HaraStruct struct = new HaraStruct(type, new Object[] {1L, 2L});
    HaraStruct decoded = (HaraStruct) HtaValueCodec.decodeCanonical(HtaValueCodec.encode(struct));
    assertEquals("demo/Point", decoded.type().name());
    assertEquals(1L, decoded.read("x"));
    assertEquals(2L, decoded.read("y"));
    assertEquals(1L, Keyword.create("x").getArg1().apply(decoded));
    assertEquals(7L, Keyword.create("missing").getArg2().apply(decoded, 7L));

    HaraMutable mutable =
        new HaraMutable(new HaraMutableType("demo/Cursor", new String[] {"x"}), new Object[] {1L});
    HaraException error =
        assertThrows(HaraException.class, () -> HtaValueCodec.encode(mutable));
    assertEquals(
        "hta/value-unsupported: mutable values are not serializable; use (into {} value)",
        error.getMessage());
  }

  @Test
  public void immutableRuntimeCollectionsAndPointersRoundTrip() {
    Object tuple = new hara.lang.data.Tuple.Tup2.L<>(null, Keyword.create("x"), 42L);
    Object decodedTuple = HtaValueCodec.decodeCanonical(HtaValueCodec.encode(tuple));
    assertTrue(decodedTuple instanceof hara.lang.data.Vector<?>);
    assertEquals(42L, ((hara.lang.data.Vector<?>) decodedTuple).nth(1));

    Object entry = new hara.lang.data.MapEntry<>(null, Keyword.create("x"), 42L);
    byte[] encodedEntry = HtaValueCodec.encode(entry);
    assertEquals(38, Byte.toUnsignedInt(encodedEntry[4]));
    Object decodedEntry = HtaValueCodec.decodeCanonical(encodedEntry);
    assertTrue(decodedEntry instanceof hara.lang.data.MapEntry<?, ?>);
    assertEquals(entry, decodedEntry);

    Object orderedSet = hara.lang.data.OrderedSet.Standard.from(null, "b", "a");
    Object decodedSet = HtaValueCodec.decodeCanonical(HtaValueCodec.encode(orderedSet));
    assertTrue(decodedSet instanceof hara.lang.data.OrderedSet<?>);

    Object deque = hara.lang.data.Deque.Standard.from(null, 1L, 2L);
    Object decodedDeque = HtaValueCodec.decodeCanonical(HtaValueCodec.encode(deque));
    assertTrue(decodedDeque instanceof hara.lang.data.Deque<?>);

    Object priorityMap = hara.lang.data.PriorityMap.Standard.from(null, "a", 2L, "b", 1L);
    Object decodedPriorityMap = HtaValueCodec.decodeCanonical(HtaValueCodec.encode(priorityMap));
    assertTrue(decodedPriorityMap instanceof hara.lang.data.PriorityMap<?, ?>);
    assertEquals("b", ((hara.lang.data.PriorityMap<?, ?>) decodedPriorityMap).peekFirst().getKey());

    java.util.Map<Object, Object> fields = new LinkedHashMap<>();
    fields.put(Keyword.create("id"), "ROOT");
    Object pointer = new hara.lang.data.Pointer(Keyword.create("kernel"), fields);
    Object decodedPointer = HtaValueCodec.decodeCanonical(HtaValueCodec.encode(pointer));
    assertTrue(decodedPointer instanceof hara.lang.data.Pointer);
    assertEquals(pointer, decodedPointer);
  }

  @Test
  public void qualifiedVarsRoundTripAsImmutableReferences() {
    HaraVar variable = new HaraVar("example.lib", "answer", 42L);
    byte[] encoded = HtaValueCodec.encode(variable);
    assertEquals(35, Byte.toUnsignedInt(encoded[4]));
    assertArrayEquals(
        new byte[] {
          'H', 'T', 'A', '0', 35, 7, 0, 0, 0, 18,
          'e', 'x', 'a', 'm', 'p', 'l', 'e', '.', 'l', 'i', 'b', '/',
          'a', 'n', 's', 'w', 'e', 'r'
        },
        encoded);

    Object decoded = HtaValueCodec.decodeCanonical(encoded);
    assertTrue(decoded instanceof HaraVar);
    HaraVar reference = (HaraVar) decoded;
    assertEquals("example.lib", reference.namespaceName());
    assertEquals("answer", reference.symbolName());
    assertEquals(null, reference.deref());
  }

  private static IMapType registryCase(String caseName) throws Exception {
    Path path =
        SpecRegistry.resolve("02-platform/000050-transport-hta/draft/conformance/transport-hta.edn");
    Object value = Parser.LispReader.readString(Files.readString(path), null);
    IMapType suite = (IMapType) value;
    ILinearType cases = linear(suite.lookup(Keyword.create("suite", "cases")));
    for (int index = 0; index < cases.count(); index++) {
      IMapType candidate = (IMapType) cases.nth(index);
      Keyword id = (Keyword) candidate.lookup(Keyword.create("case", "id"));
      if ("hta.case".equals(id.getNamespace()) && caseName.equals(id.getName())) {
        return candidate;
      }
    }
    throw new AssertionError("Missing registry HTA case: " + caseName);
  }

  private static ILinearType linear(Object value) {
    if (!(value instanceof ILinearType values)) {
      throw new AssertionError("Expected registry vector: " + value);
    }
    return values;
  }

  private static byte[] bytes(ILinearType values) {
    byte[] output = new byte[Math.toIntExact(values.count())];
    for (int index = 0; index < values.count(); index++) {
      output[index] = ((Number) values.nth(index)).byteValue();
    }
    return output;
  }

}
