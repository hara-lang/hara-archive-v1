package hara.lang.protocol;

import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

@HaraProtocolBinding(namespace = "std.protocol.ipushfirst", name = "IPushFirst")
public interface IPushFirst<E> {
  @HaraMethod(value = "push-first", arity = 2)
  IPushFirst<E> pushFirst(E e);
}
