package hara.lang.protocol;

import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

@HaraProtocolBinding(namespace = "std.protocol.ifind", name = "IFind")
public interface IFind<K, V> {
  @HaraMethod(value = "find", arity = 2)
  V find(K key);

  default boolean has(K key) {
    return find(key) != null;
  }
}
