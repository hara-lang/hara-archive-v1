package hara.lang.protocol;

import hara.lang.data.Tuple;
import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

import java.util.Iterator;
import java.util.Map;
import java.util.function.Consumer;

@HaraProtocolBinding(namespace = "std.protocol.iwatch", name = "IWatch")
public interface IWatch<R, V> {
  @HaraMethod(value = "watch-add", arity = 3)
  default void addWatch(Object key, Consumer<WatchEntry<R, V>> f) {
    throw new UnsupportedOperationException("Not Supported");
  }

  @HaraMethod(value = "watch-list", arity = 1)
  default Iterator<Map.Entry<Object, Consumer<WatchEntry<R, V>>>> getWatches() {
    return null;
  }

  default void notifyWatches(V oldVal, V newVal) {
    Iterator<Map.Entry<Object, Consumer<WatchEntry<R, V>>>> ws = getWatches();
    if (ws != null) {
      ws.forEachRemaining(
          e -> e.getValue().accept(new WatchEntry<R, V>(e.getKey(), this, oldVal, newVal)));
    }
  }

  @HaraMethod(value = "watch-remove", arity = 2)
  default void removeWatch(Object key) {
    throw new UnsupportedOperationException("Not Supported");
  }

  @SuppressWarnings("unchecked")
  public class WatchEntry<R, V> extends Tuple.Tup5.L<Object, R, Object, V, V> {

    WatchEntry(Object key, IWatch<R, V> ref, V oldVal, V newVal) {
      super(null, key, (R) ref, null, oldVal, newVal);
    }

    public V oldVal() {
      return this.D();
    }

    public V newVal() {
      return this.E();
    }
  }
}
