package hara.lang.protocol;

import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;
import java.util.Iterator;
import java.util.Map;

@HaraProtocolBinding(
    namespace = "std.protocol.ilookup", name = "ILookup", parents = {"IFind"})
public interface ILookup<K, V> extends IFind<K, Map.Entry<K, V>> {

  Iterator<K> keys();

  default V lookup(K key) {
    return lookup(key, null);
  }

  @HaraMethod(value = "lookup", arity = -1, variadic = true)
  default V lookup(K key, V notFound) {
    Map.Entry<K, V> ret = find(key);
    return (ret == null) ? notFound : ret.getValue();
  }

  Iterator<V> vals();
}
