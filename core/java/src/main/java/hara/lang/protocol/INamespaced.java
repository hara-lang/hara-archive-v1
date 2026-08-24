package hara.lang.protocol;

import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

@HaraProtocolBinding(namespace = "std.protocol.inamespaced", name = "INamespaced")
public interface INamespaced {
  @HaraMethod(value = "name", arity = 1)
  String getName();

  @HaraMethod(value = "namespace", arity = 1)
  String getNamespace();
}
