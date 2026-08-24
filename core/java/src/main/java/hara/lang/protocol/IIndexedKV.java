package hara.lang.protocol;

import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

@HaraProtocolBinding(namespace = "std.protocol.iindexedkv", name = "IIndexedKV")
public interface IIndexedKV<K, V> {
  @HaraMethod(value = "index-of-key", arity = 2)
  long indexOfKey(K key);

  @HaraMethod(value = "index-of-val", arity = 2)
  long indexOfVal(V val);
}
