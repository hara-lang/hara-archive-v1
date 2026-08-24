package hara.lang.protocol;

import hara.lang.declaration.HaraAvailability;
import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

/** Exceptional stream termination capability. */
@HaraProtocolBinding(
    namespace = "std.protocol.iabort",
    name = "IAbort",
    availability = HaraAvailability.CAPABILITY_GATED,
    capability = "native-runtime-protocols")
public interface IAbort {
  @HaraMethod(value = "abort", arity = 2)
  Object abort(Object error);
}
