package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertTrue;

import hara.lang.data.Atom;
import hara.lang.protocol.IDeps;
import hara.lang.protocol.ISetType;
import hara.lang.data.List;
import hara.lang.data.Keyword;
import hara.lang.data.MapEntry;
import hara.lang.data.Queue;
import hara.lang.data.Set;
import hara.lang.data.SortedMap;
import hara.lang.data.Tuple;
import hara.lang.data.Vector;
import hara.lang.base.primitive.Counter;
import hara.lang.base.primitive.Delay;
import java.util.Iterator;
import java.util.Map;
import org.junit.Test;

public class HaraProtocolRuntimeTest {
  @Test
  public void adaptsPersistentCollectionsWithoutChangingTheirInterfaces() {
    HaraProtocol lookup = new HaraProtocol("ILookup", Map.of("lookup", -1));
    HaraProtocolRuntime.installForTest(lookup);
    Vector.Standard<String> vector = Vector.Standard.from(null, "zero", "one");

    assertEquals("one", lookup.invoke("lookup", vector, new Object[] {1L}));

    HaraProtocol assoc = new HaraProtocol("IAssoc", Map.of("assoc", 3));
    HaraProtocolRuntime.installForTest(assoc);
    assertEquals(
        "value",
        lookup.invoke(
            "lookup", assoc.invoke("assoc", vector, new Object[] {0, "value"}), new Object[] {0L}));
  }

  @Test
  public void adaptsMapsFunctionsAndStatefulValues() {
    HaraProtocol lookup = new HaraProtocol("ILookup", Map.of("lookup", -1));
    HaraProtocolRuntime.installForTest(lookup);
    hara.lang.data.Map.Standard<String, String> map =
        hara.lang.data.Map.Standard.from(null, "key", "value");
    assertEquals("value", lookup.invoke("lookup", map, new Object[] {"key"}));

    HaraProtocol ifn = new HaraProtocol("IFn", Map.of("invoke", -1));
    HaraProtocolRuntime.installForTest(ifn);
    assertEquals("value", ifn.invoke("invoke", map, new Object[] {"key"}));

    HaraProtocol deref = new HaraProtocol("IDeref", Map.of("deref", 1));
    HaraProtocolRuntime.installForTest(deref);
    Atom.Basic<Long> atom = new Atom.Basic<>(41L);
    assertEquals(41L, deref.invoke("deref", atom, new Object[0]));
  }

@Test
public void bridgesTheDependencyContractThroughTheGenericAdapter() {
  class DependencyFixture implements IDeps<String, String> {
    @Override
    public String depGet(String id) {
      return "entry:" + id;
    }

    @Override
    public ISetType<String> depEntries(String id) {
      return Set.Standard.from(null, "base:" + id);
    }

    @Override
    public Iterator<String> depKeys() {
      return java.util.List.of("a", "b").iterator();
    }
  }

  DependencyFixture fixture = new DependencyFixture();
  HaraProtocol deps =
      new HaraProtocol(
          "IDeps", Map.of("dep-get", 2, "dep-entries", 2, "dep-keys", 1));
  HaraProtocolRuntime.installForTest(deps);

  assertEquals("entry:a", deps.invoke("dep-get", fixture, new Object[] {"a"}));
  @SuppressWarnings("unchecked")
  ISetType<String> entries =
      (ISetType<String>) deps.invoke("dep-entries", fixture, new Object[] {"a"});
  assertTrue(entries.has("base:a"));
  Iterator<?> ids = (Iterator<?>) deps.invoke("dep-keys", fixture, new Object[0]);
  assertEquals("a", ids.next());
  assertEquals("b", ids.next());
}
  @Test
  public void adaptsCollectionNavigationAndConversion() {
    List.Standard<String> list = List.Standard.from(null, "one", "two");
    HaraProtocol peekFirst = new HaraProtocol("IPeekFirst", Map.of("peek-first", 1));
    HaraProtocolRuntime.installForTest(peekFirst);
    HaraProtocol peekLast = new HaraProtocol("IPeekLast", Map.of("peek-last", 1));
    HaraProtocolRuntime.installForTest(peekLast);
    HaraProtocol pushFirst = new HaraProtocol("IPushFirst", Map.of("push-first", 2));
    HaraProtocolRuntime.installForTest(pushFirst);
    assertEquals("one", peekFirst.invoke("peek-first", list, new Object[0]));
    assertEquals("two", peekLast.invoke("peek-last", list, new Object[0]));
    assertEquals(
        "zero",
        peekFirst.invoke(
            "peek-first",
            pushFirst.invoke("push-first", list, new Object[] {"zero"}),
            new Object[0]));

    HaraProtocol cons = new HaraProtocol("ICons", Map.of("cons", 2));
    HaraProtocolRuntime.installForTest(cons);
    assertEquals(
        "zero",
        peekFirst.invoke(
            "peek-first", cons.invoke("cons", list, new Object[] {"zero"}), new Object[0]));

    HaraProtocol toMutable = new HaraProtocol("IToMutable", Map.of("to-mutable", 1));
    HaraProtocolRuntime.installForTest(toMutable);
    HaraProtocol persistent = new HaraProtocol("IToPersistent", Map.of("to-persistent", 1));
    HaraProtocolRuntime.installForTest(persistent);
    assertEquals(
        "hara.lang.data.List$Mutable",
        toMutable.invoke("to-mutable", list, new Object[0]).getClass().getName());
  }

  @Test
  public void adaptsStateNamespaceAndIndexedProtocols() {
    Counter counter = new Counter(1);
    HaraProtocol reset = new HaraProtocol("IReset", Map.of("reset", 2));
    HaraProtocolRuntime.installForTest(reset);
    assertEquals(42, reset.invoke("reset", counter, new Object[] {42}));

    Delay<String> delay = new Delay<>(() -> "ready");
    HaraProtocol realize = new HaraProtocol("IRealize", Map.of("realized?", 1, "realize", 1));
    HaraProtocolRuntime.installForTest(realize);
    assertEquals(false, realize.invoke("realized?", delay, new Object[0]));
    assertEquals("ready", realize.invoke("realize", delay, new Object[0]));
    assertEquals(true, realize.invoke("realized?", delay, new Object[0]));

    HaraProtocol namespaced = new HaraProtocol("INamespaced", Map.of("name", 1, "namespace", 1));
    HaraProtocolRuntime.installForTest(namespaced);
    Keyword keyword = Keyword.create("core/value");
    assertEquals("value", namespaced.invoke("name", keyword, new Object[0]));
    assertEquals("core", namespaced.invoke("namespace", keyword, new Object[0]));

    HaraProtocol indexed =
        new HaraProtocol("IIndexedKV", Map.of("index-of-key", 2, "index-of-val", 2));
    HaraProtocolRuntime.installForTest(indexed);
    SortedMap.Standard<Integer, String> sorted = SortedMap.Standard.from(null, 1, "one", 2, "two");
    assertEquals(1L, indexed.invoke("index-of-key", sorted, new Object[] {2}));
  }

  @Test
  public void exposesIteratorTraversalThroughTheCollectionProtocol() {
    HaraProtocol collection =
        new HaraProtocol(
            "IColl", Map.of("start-string", 1, "end-string", 1, "sep-string", 1, "iterator", 1));
    HaraProtocolRuntime.installForTest(collection);
    Vector.Standard<String> vector = Vector.Standard.from(null, "first", "second");

    Iterator<?> iterator = (Iterator<?>) collection.invoke("iterator", vector, new Object[0]);
    assertEquals("first", iterator.next());
    assertEquals("second", iterator.next());
  }

  @Test
  public void adaptsJavaIterablesAndIteratorsThroughIIter() {
    HaraProtocol iter = new HaraProtocol("IIter", Map.of("iter", 1));
    HaraProtocolRuntime.installForTest(iter);
    HaraProtocol iterator =
        new HaraProtocol("IIterator", Map.of("iter-next?", 1, "iter-next", 1));
    HaraProtocolRuntime.installForTest(iterator);
    HaraProtocol close = new HaraProtocol("IClose", Map.of("close", 1));
    HaraProtocolRuntime.installForTest(close);

    Object cursor =
        iter.invoke(
            "iter", Vector.Standard.from(null, "first", "second"), new Object[0]);
    assertEquals(true, iterator.invoke("iter-next?", cursor, new Object[0]));
    assertEquals("first", iterator.invoke("iter-next", cursor, new Object[0]));
    assertEquals("second", iterator.invoke("iter-next", cursor, new Object[0]));
    assertEquals(false, iterator.invoke("iter-next?", cursor, new Object[0]));
    assertEquals(null, close.invoke("close", cursor, new Object[0]));
  }

  @Test
  public void coversTheCommonCollectionProtocolAcrossAllCoreShapes() {
    HaraProtocol collection =
        new HaraProtocol(
            "IColl", Map.of("start-string", 1, "end-string", 1, "sep-string", 1, "iterator", 1));
    HaraProtocolRuntime.installForTest(collection);

    HaraProtocol count = new HaraProtocol("ICount", Map.of("count", 1));
    HaraProtocolRuntime.installForTest(count);

    Object[][] values = {
      {Vector.Standard.from(null, "one", "two"), 2L, "one"},
      {List.Standard.from(null, "one", "two"), 2L, "one"},
      {Queue.Standard.from(null, "one", "two"), 2L, "one"},
      {new Tuple.Tup3.L<>(null, "one", "two", "three"), 3L, "one"},
      {Set.Standard.from(null, "one", "two"), 2L, null}
    };

    for (Object[] value : values) {
      Object receiver = value[0];
      assertEquals(value[1], count.invoke("count", receiver, new Object[0]));
      Iterator<?> iterator = (Iterator<?>) collection.invoke("iterator", receiver, new Object[0]);
      if (value[2] != null) {
        assertEquals(value[2], iterator.next());
      } else {
        assertTrue(iterator.hasNext());
      }
    }
  }

  @Test
  public void adaptsIndexedSetAndPairInvocationSemantics() {
    HaraProtocol nth = new HaraProtocol("INth", Map.of("nth", 2));
    HaraProtocolRuntime.installForTest(nth);
    Queue.Standard<String> queue = Queue.Standard.from(null, "zero", "one");
    assertEquals("one", nth.invoke("nth", queue, new Object[] {1L}));

    HaraProtocol ifn = new HaraProtocol("IFn", Map.of("invoke", -1));
    HaraProtocolRuntime.installForTest(ifn);
    Set.Standard<String> set = Set.Standard.from(null, "present");
    assertEquals("present", ifn.invoke("invoke", set, new Object[] {"present"}));
    assertEquals("missing", ifn.invoke("invoke", set, new Object[] {"missing", "missing"}));

    HaraProtocol pair = new HaraProtocol("IPair", Map.of("key", 1, "value", 1));
    HaraProtocolRuntime.installForTest(pair);
    MapEntry<String, Integer> entry = new MapEntry<>(null, "key", 42);
    assertEquals("key", pair.invoke("key", entry, new Object[0]));
    assertEquals(42, pair.invoke("value", entry, new Object[0]));
  }
}
