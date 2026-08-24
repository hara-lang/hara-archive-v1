package hara.lang.protocol;

import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

@HaraProtocolBinding(namespace = "std.protocol.ipushlast", name = "IPushLast")
public interface IPushLast<E> {
  @HaraMethod(value = "push-last", arity = 2)
  IPushLast<E> pushLast(E e);
}
