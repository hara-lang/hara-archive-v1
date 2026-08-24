package hara.lang.protocol;

import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

@HaraProtocolBinding(namespace = "std.protocol.iencode", name = "IEncode")
public interface IEncode {
  @HaraMethod(value = "encode", arity = 2)
  Object encode(Object options);
}
