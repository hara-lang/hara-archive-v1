package hara.lang.protocol;

import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

@HaraProtocolBinding(namespace = "std.protocol.idereftimeout", name = "IDerefTimeout")
public interface IDerefTimeout<V> {
  @HaraMethod(value = "deref-timeout", arity = 3)
  V derefTimeout(long ms, V timeoutVal);
}
