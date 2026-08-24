package hara.lang.protocol;

import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

@HaraProtocolBinding(namespace = "std.protocol.ipeeklast", name = "IPeekLast")
public interface IPeekLast<E> {
  @HaraMethod(value = "peek-last", arity = 1)
  E peekLast();
}
