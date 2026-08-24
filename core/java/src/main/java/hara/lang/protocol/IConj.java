package hara.lang.protocol;

import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

@HaraProtocolBinding(namespace = "std.protocol.iconj", name = "IConj")
public interface IConj<E> {
  @HaraMethod(value = "conj", arity = 2)
  IConj<E> conj(E e);
}
