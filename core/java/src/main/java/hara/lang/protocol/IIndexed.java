package hara.lang.protocol;

import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

@HaraProtocolBinding(namespace = "std.protocol.iindexed", name = "IIndexed")
public interface IIndexed<K, V> {
  @HaraMethod(value = "index-of", arity = 2)
  K indexOf(V val);
}
