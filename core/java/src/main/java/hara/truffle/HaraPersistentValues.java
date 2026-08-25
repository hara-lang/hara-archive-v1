package hara.truffle;

import hara.lang.protocol.IMapType;
import hara.lang.protocol.ISequential;
import hara.lang.protocol.ISetType;
import java.lang.reflect.Array;
import java.util.ArrayList;
import java.util.Collection;
import java.util.Map;

/** Converts host collection builders into persistent Hara values at public boundaries. */
final class HaraPersistentValues {
  private HaraPersistentValues() {}

  static Object normalize(Object value) {
    Object unwrapped = HaraBox.unwrap(value);
    if (unwrapped == null
        || unwrapped instanceof byte[]
        || unwrapped instanceof IMapType<?, ?>
        || unwrapped instanceof ISequential<?>
        || unwrapped instanceof ISetType<?>) {
      return unwrapped;
    }
    if (unwrapped instanceof Map<?, ?> map) {
      ArrayList<Object> entries = new ArrayList<>(map.size() * 2);
      map.forEach(
          (key, nested) -> {
            entries.add(normalize(key));
            entries.add(normalize(nested));
          });
      return hara.lang.data.Map.Standard.from(null, entries.toArray());
    }
    if (unwrapped instanceof java.util.Set<?> set) {
      ArrayList<Object> values = new ArrayList<>(set.size());
      for (Object nested : set) values.add(normalize(nested));
      return hara.lang.data.Set.Standard.from(null, values.toArray());
    }
    if (unwrapped instanceof Collection<?> collection) {
      ArrayList<Object> values = new ArrayList<>(collection.size());
      for (Object nested : collection) values.add(normalize(nested));
      return hara.lang.data.Vector.Standard.from(null, values.toArray());
    }
    Class<?> type = unwrapped.getClass();
    if (type.isArray()) {
      int length = Array.getLength(unwrapped);
      Object[] values = new Object[length];
      for (int i = 0; i < length; i++) values[i] = normalize(Array.get(unwrapped, i));
      return hara.lang.data.Vector.Standard.from(null, values);
    }
    return unwrapped;
  }
}
