package hara.lang.protocol;

import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

@HaraProtocolBinding(namespace = "std.protocol.icount", name = "ICount")
public interface ICount {
  @HaraMethod(value = "count", arity = 1)
  long count();
}
