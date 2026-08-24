package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertSame;
import static org.junit.Assert.assertThrows;
import static org.junit.Assert.assertTrue;

import hara.lang.data.List;
import hara.lang.data.Map;
import hara.lang.data.Queue;
import hara.lang.data.Set;
import hara.lang.data.SortedMap;
import hara.lang.data.Tuple;
import hara.lang.data.Vector;
import hara.lang.protocol.IColl;
import hara.lang.protocol.ICount;
import hara.lang.protocol.ILookup;
import hara.lang.protocol.IPair;
import java.util.ArrayList;
import java.util.Iterator;
import org.junit.Test;

/** Differential checks between protocol calls and the existing Java collection behavior. */
public class CollectionProtocolDifferentialTest {
  @Test
  public void protocolCountAndIterationMatchJavaValues() {
    HaraProtocol count = new HaraProtocol("ICount", java.util.Map.of("count", 1));
    HaraProtocolRuntime.installForTest(count);
    HaraProtocol collection =
        new HaraProtocol(
            "IColl",
            java.util.Map.of("start-string", 1, "end-string", 1, "sep-string", 1, "iterator", 1));
    HaraProtocolRuntime.installForTest(collection);

    Object[] values = {
      Map.Standard.from(null, "a", 1, "b", 2),
      Vector.Standard.from(null, "a", "b"),
      List.Standard.from(null, "a", "b"),
      Set.Standard.from(null, "a", "b"),
      Queue.Standard.from(null, "a", "b"),
      new Tuple.Tup2.L<>(null, "a", "b"),
      SortedMap.Standard.from(null, "b", 2, "a", 1)
    };

    for (Object value : values) {
      ICount javaValue = (ICount) value;
      assertEquals(javaValue.count(), count.invoke("count", value, new Object[0]));

      ArrayList<Object> expected = new ArrayList<>();
      for (Object element : (Iterable<?>) value) {
        expected.add(comparable(element));
      }
      Iterator<?> protocolIterator =
          (Iterator<?>) collection.invoke("iterator", value, new Object[0]);
      ArrayList<Object> actual = new ArrayList<>();
      protocolIterator.forEachRemaining(element -> actual.add(comparable(element)));
      assertEquals(expected, actual);
    }
  }

  @Test
  public void nilAndUnsupportedValuesHaveExplicitProtocolBoundaries() {
    HaraProtocol lookup = new HaraProtocol("ILookup", java.util.Map.of("lookup", -1));
    HaraProtocolRuntime.installForTest(lookup);
    HaraProtocol count = new HaraProtocol("ICount", java.util.Map.of("count", 1));
    HaraProtocolRuntime.installForTest(count);
    HaraProtocol conj = new HaraProtocol("IConj", java.util.Map.of("conj", 2));
    HaraProtocolRuntime.installForTest(conj);
    HaraProtocol empty = new HaraProtocol("IEmpty", java.util.Map.of("empty", 1));
    HaraProtocolRuntime.installForTest(empty);
    HaraProtocol assoc = new HaraProtocol("IAssoc", java.util.Map.of("assoc", 3));
    HaraProtocolRuntime.installForTest(assoc);

    assertEquals(0L, count.invoke("count", null, new Object[0]));
    assertEquals("fallback", lookup.invoke("lookup", null, new Object[] {"key", "fallback"}));
    assertEquals(null, empty.invoke("empty", null, new Object[0]));
    Object singleton = conj.invoke("conj", null, new Object[] {"value"});
    assertTrue(singleton instanceof List.Standard);
    assertEquals("value", ((Iterable<?>) singleton).iterator().next());

    HaraException primitive =
        assertThrows(
            HaraException.class,
            () -> assoc.invoke("assoc", 42L, new Object[] {"key", "value"}));
    assertTrue(primitive.getMessage().contains("IAssoc/assoc"));
    assertTrue(primitive.getMessage().contains("category=number"));
    assertTrue(primitive.getMessage().contains("primitive:number"));
  }

  @Test
  public void invalidIndexesDuplicatesAndMapEntryShapeMatchJavaValues() {
    HaraProtocol nth = new HaraProtocol("INth", java.util.Map.of("nth", 2));
    HaraProtocolRuntime.installForTest(nth);
    Vector.Standard<String> vector = Vector.Standard.from(null, "only");
    assertThrows(RuntimeException.class, () -> nth.invoke("nth", vector, new Object[] {-1L}));
    assertThrows(RuntimeException.class, () -> nth.invoke("nth", vector, new Object[] {1L}));

    Set.Standard<String> set = Set.Standard.from(null, "same", "same");
    assertEquals(1L, set.count());
    assertSame("same", set.find("same"));

    Map.Standard<String, Long> map = Map.Standard.from(null, "key", 1L, "key", 2L);
    assertEquals(1L, map.count());
    assertEquals(2L, map.lookup("key").longValue());
    Object entry = map.iterator().next();
    assertTrue(entry instanceof IPair);
    IPair<?, ?> pair = (IPair<?, ?>) entry;
    assertEquals("key", pair.getKey());
    assertEquals(Long.valueOf(2L), pair.getValue());
  }

  private static Object comparable(Object value) {
    if (value instanceof IPair) {
      IPair<?, ?> pair = (IPair<?, ?>) value;
      return java.util.Arrays.asList(pair.getKey(), pair.getValue());
    }
    return value;
  }

  @Test
  public void lookupAndInvocationPreserveNullAndNotFoundSemantics() {
    HaraProtocol lookup = new HaraProtocol("ILookup", java.util.Map.of("lookup", -1));
    HaraProtocolRuntime.installForTest(lookup);
    HaraProtocol ifn = new HaraProtocol("IFn", java.util.Map.of("invoke", -1));
    HaraProtocolRuntime.installForTest(ifn);

    Map.Standard<String, String> map = Map.Standard.from(null, "present", null);
    ILookup<String, String> javaMap = map;
    assertEquals(javaMap.lookup("present"), lookup.invoke("lookup", map, new Object[] {"present"}));
    assertEquals("missing", lookup.invoke("lookup", map, new Object[] {"missing", "missing"}));
    assertEquals("missing", ifn.invoke("invoke", map, new Object[] {"missing", "missing"}));
  }

  @Test
  public void indexedAndSetInvocationMatchDirectValues() {
    HaraProtocol ifn = new HaraProtocol("IFn", java.util.Map.of("invoke", -1));
    HaraProtocolRuntime.installForTest(ifn);

    Vector.Standard<String> vector = Vector.Standard.from(null, "zero", "one");
    assertEquals(vector.nth(1L), ifn.invoke("invoke", vector, new Object[] {1L}));

    Set.Standard<String> set = Set.Standard.from(null, "present");
    assertEquals("present", ifn.invoke("invoke", set, new Object[] {"present"}));
    assertEquals("missing", ifn.invoke("invoke", set, new Object[] {"missing", "missing"}));
    assertEquals("present", set.find("present"));
  }
}
