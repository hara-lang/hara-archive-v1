package hara.lang.protocol;

import hara.lang.declaration.HaraAvailability;
import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

/** Asynchronous buffered-write barrier. */
@HaraProtocolBinding(
    namespace = "std.protocol.iflush",
    name = "IFlush",
    availability = HaraAvailability.CAPABILITY_GATED,
    capability = "native-runtime-protocols")
public interface IFlush {
  @HaraMethod(value = "flush", arity = 1)
  Object flush();
}
