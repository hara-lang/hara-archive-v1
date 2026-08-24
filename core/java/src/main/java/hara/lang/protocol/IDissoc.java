package hara.lang.protocol;

import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

@HaraProtocolBinding(namespace = "std.protocol.idissoc", name = "IDissoc")
public interface IDissoc<K> {
  @HaraMethod(value = "dissoc", arity = 2)
  IDissoc<K> dissoc(K k);
}
