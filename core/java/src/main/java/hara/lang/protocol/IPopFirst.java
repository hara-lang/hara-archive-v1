package hara.lang.protocol;

import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

@HaraProtocolBinding(namespace = "std.protocol.ipopfirst", name = "IPopFirst")
public interface IPopFirst {
  @HaraMethod(value = "pop-first", arity = 1)
  IPopFirst popFirst();
}
