package hara.lang.protocol;

import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

@HaraProtocolBinding(namespace = "std.protocol.iencodable", name = "IEncodable")
public interface IEncodable {
  @HaraMethod(value = "encode-with", arity = 2)
  Object encodeWith(Object visitor);
}
