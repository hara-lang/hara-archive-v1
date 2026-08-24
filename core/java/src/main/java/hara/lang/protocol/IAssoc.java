package hara.lang.protocol;

import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

@HaraProtocolBinding(namespace = "std.protocol.iassoc", name = "IAssoc")
public interface IAssoc<K, V> {
  @HaraMethod(value = "assoc", arity = 3)
  IAssoc<K, V> assoc(K k, V v);
}
