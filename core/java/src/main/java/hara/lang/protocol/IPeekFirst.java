package hara.lang.protocol;

import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

@HaraProtocolBinding(namespace = "std.protocol.ipeekfirst", name = "IPeekFirst")
public interface IPeekFirst<E> {
  @HaraMethod(value = "peek-first", arity = 1)
  E peekFirst();
}
