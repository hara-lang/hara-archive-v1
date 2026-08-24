package hara.lang.protocol;

import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

@HaraProtocolBinding(namespace = "std.protocol.inth", name = "INth")
public interface INth<E> {
  @HaraMethod(value = "nth", arity = 2)
  E nth(long i);
}
