package hara.lang.protocol;

import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

@HaraProtocolBinding(namespace = "std.protocol.ipointer", name = "IPointer")
public interface IPointer {
  @HaraMethod(value = "ptr-context", arity = 1)
  Object ptrContext();
}
