package hara.lang.protocol;

import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

@HaraProtocolBinding(namespace = "std.protocol.ideref", name = "IDeref")
public interface IDeref<V> {
  @HaraMethod(value = "deref", arity = 1)
  V deref();
}
