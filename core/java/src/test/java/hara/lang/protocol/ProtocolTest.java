package hara.lang.protocol;

import hara.lang.base.Ex;
import org.junit.Test;

import java.util.Iterator;
import java.util.Map;
import java.util.concurrent.atomic.AtomicReference;
import java.util.function.Consumer;

import static org.junit.Assert.*;

public class ProtocolTest {

  @Test
  public void testConstantEnums() {
    assertEquals(Constant.MetaType.OBJECT, Constant.MetaType.valueOf("OBJECT"));
    assertEquals(Constant.HashType.RAPID, Constant.HashType.valueOf("RAPID"));
    assertEquals(Constant.HashType.MURMUR3, Constant.HashType.valueOf("MURMUR3"));
    assertEquals(Constant.ObjType.STRING, Constant.ObjType.valueOf("STRING"));
  }

  @Test
  public void testICollDefaults() {
    IColl<String> coll =
        new IColl<String>() {
          @Override
          public Iterator<String> iterator() {
            return java.util.Arrays.asList("a", "b").iterator();
          }

          @Override
          public boolean equality(Object o) {
            return false;
          }

          @Override
          public IColl<String> conj(String s) {
            return null;
          }

          @Override
          public IColl<String> empty() {
            return null;
          }

          @Override
          public long hashCalc(Constant.HashType type) {
            return 0;
          }

          @Override
          public String startString() {
            return "[";
          }

          @Override
          public String endString() {
            return "]";
          }
        };

    assertEquals(" ", coll.sepString());
    assertEquals("[\"a\" \"b\"]", coll.display());
  }

  @Test
  public void testIComponentDefaults() {
    IComponent comp =
        new IComponent() {
          @Override
          public IMetadata getProps() {
            return null;
          }

          @Override
          public IMetadata getStatus() {
            return null;
          }

          @Override
          public boolean isStarted() {
            return false;
          }

          @Override
          public boolean isStopped() {
            return true;
          }

          @Override
          public IComponent start() {
            return this;
          }

          @Override
          public IComponent stop() {
            return this;
          }
        };

    assertFalse(comp.isRemote());
    assertEquals(comp, comp.kill()); // kill calls stop
  }

  @Test
  public void testIFnDefaults() {
    IFn<String, String, String> fn =
        new IFn<String, String, String>() {
          @Override
          public String invoke(String s) {
            return "invoked " + s;
          }

          public java.util.function.Function<String, String> getArg1() {
            return this::invoke;
          }
        };

    assertEquals("invoked a", fn.invoke("a"));

    // Test apply with List
    assertEquals("invoked a", fn.apply(java.util.Arrays.asList("a")));

    // Test apply with array
    assertEquals("invoked a", fn.apply(new Object[] {"a"}));
  }

  @Test
  public void testILookupDefaults() {
    ILookup<String, String> lookup =
        new ILookup<String, String>() {
          @Override
          public Map.Entry<String, String> find(String key) {
            if ("a".equals(key)) return new java.util.AbstractMap.SimpleEntry<>("a", "1");
            return null;
          }

          @Override
          public Iterator<String> keys() {
            return null;
          }

          @Override
          public Iterator<String> vals() {
            return null;
          }
        };

    assertEquals("1", lookup.lookup("a"));
    assertNull(lookup.lookup("b"));
    assertEquals("default", lookup.lookup("b", "default"));
  }

  @Test(expected = Ex.Unsupported.class)
  public void testIPairDefaults() {
    IPair<String, String> pair =
        new IPair<String, String>() {
          @Override
          public String getKey() {
            return "k";
          }

          @Override
          public String getValue() {
            return "v";
          }
        };
    pair.setValue("new");
  }

  @Test
  public void testIWatchDefaults() {
    AtomicReference<String> notified = new AtomicReference<>();
    IWatch<Object, String> watch =
        new IWatch<Object, String>() {
          @Override
          public Iterator<Map.Entry<Object, Consumer<WatchEntry<Object, String>>>> getWatches() {
            Consumer<WatchEntry<Object, String>> consumer =
                e -> notified.set(e.oldVal() + "->" + e.newVal());
            Map.Entry<Object, Consumer<WatchEntry<Object, String>>> entry =
                new java.util.AbstractMap.SimpleEntry<>("key", consumer);
            return java.util.Collections.singletonList(entry).iterator();
          }
        };

    watch.notifyWatches("old", "new");
    assertEquals("old->new", notified.get());

    try {
      watch.addWatch("k", null);
      fail("Should throw UnsupportedOperationException");
    } catch (UnsupportedOperationException e) {
    }

    try {
      watch.removeWatch("k");
      fail("Should throw UnsupportedOperationException");
    } catch (UnsupportedOperationException e) {
    }
  }
}
