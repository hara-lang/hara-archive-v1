package hara.lang.protocol;

import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

@HaraProtocolBinding(namespace = "std.protocol.icons", name = "ICons")
public interface ICons<E> {
  @HaraMethod(value = "cons", arity = 2)
  ICons<E> cons(E e);
}
