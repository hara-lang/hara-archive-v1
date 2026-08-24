package hara.lang.protocol;

import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

@HaraProtocolBinding(namespace = "std.protocol.iempty", name = "IEmpty")
public interface IEmpty {
  @HaraMethod(value = "empty", arity = 1)
  IEmpty empty();
}
