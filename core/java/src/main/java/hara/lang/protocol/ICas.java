package hara.lang.protocol;

import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

/** Atomically replaces an expected value. */
@HaraProtocolBinding(namespace = "std.protocol.icas", name = "ICas")
public interface ICas<V> {
  @HaraMethod(value = "cas", arity = 3)
  boolean cas(V oldValue, V newValue);
}
