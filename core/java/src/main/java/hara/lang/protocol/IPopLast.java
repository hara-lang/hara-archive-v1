package hara.lang.protocol;

import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

@HaraProtocolBinding(namespace = "std.protocol.ipoplast", name = "IPopLast")
public interface IPopLast {
  @HaraMethod(value = "pop-last", arity = 1)
  IPopLast popLast();
}
