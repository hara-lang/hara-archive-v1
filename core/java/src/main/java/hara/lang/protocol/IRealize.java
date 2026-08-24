package hara.lang.protocol;

import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

@HaraProtocolBinding(namespace = "std.protocol.irealize", name = "IRealize")
public interface IRealize<V> {
  @HaraMethod(value = "realized?", arity = 1)
  boolean isRealized();

  @HaraMethod(value = "realize", arity = 1)
  V realize();
}
