package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertSame;
import static org.junit.Assert.assertTrue;

import hara.lang.protocol.IMapType;
import hara.lang.protocol.ISequential;
import hara.lang.protocol.ISetType;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.LinkedHashSet;
import java.util.Map;
import org.junit.Test;

public class HaraPersistentValuesTest {
  @Test
  public void recursivelyCopiesMutableHostCollectionsIntoPersistentHaraValues() {
    ArrayList<Object> nested = new ArrayList<>();
    nested.add("first");
    LinkedHashSet<Object> tags = new LinkedHashSet<>();
    tags.add("stable");
    Map<String, Object> source = new LinkedHashMap<>();
    source.put("items", nested);
    source.put("tags", tags);
    source.put("array", new Object[] {1L, 2L});

    Object normalized = HaraPersistentValues.normalize(source);
    assertTrue(normalized instanceof IMapType<?, ?>);
    @SuppressWarnings("unchecked")
    IMapType<Object, Object> result = (IMapType<Object, Object>) normalized;
    assertTrue(result.lookup("items") instanceof ISequential<?>);
    assertTrue(result.lookup("tags") instanceof ISetType<?>);
    assertTrue(result.lookup("array") instanceof ISequential<?>);

    nested.add("mutable-after-copy");
    tags.add("changed");
    assertEquals(1L, ((hara.lang.protocol.ICount) result.lookup("items")).count());
    assertEquals(1L, ((ISetType<?>) result.lookup("tags")).count());
  }

  @Test
  public void preservesBinaryArraysAsTheHaraBytesRepresentation() {
    byte[] bytes = new byte[] {1, 2, 3};
    assertSame(bytes, HaraPersistentValues.normalize(bytes));
  }
}
