package hara.lang.protocol;

import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

/** Asynchronous pull source. A fulfilled null value denotes end-of-stream. */
@HaraProtocolBinding(
    namespace = "std.protocol.istream", name = "IStream", parents = {"IClose"})
public interface IStream extends IClose {
  @HaraMethod(value = "next", arity = 1)
  Object next();
}
