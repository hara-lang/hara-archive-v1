package hara.lang.protocol;

import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

@HaraProtocolBinding(namespace = "std.protocol.iequality", name = "IEquality")
public interface IEquality {
  @HaraMethod(value = "equality", arity = 2)
  boolean equality(Object other);
}
