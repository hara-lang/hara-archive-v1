package hara.lang.protocol;

import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

@HaraProtocolBinding(namespace = "std.protocol.ireset", name = "IReset")
public interface IReset<V> {
  @HaraMethod(value = "reset", arity = 2)
  V reset(V v);
}
