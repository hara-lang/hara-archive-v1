package hara.lang.protocol;

import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

/** Explicitly closes a resource or traversal cursor. */
@HaraProtocolBinding(namespace = "std.protocol.iclose", name = "IClose")
public interface IClose extends AutoCloseable {
  @Override
  @HaraMethod(value = "close", arity = 1)
  void close() throws Exception;
}
