package hara.lang.protocol;

import hara.lang.declaration.HaraAvailability;
import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

/** Non-blocking writable stream capability. */
@HaraProtocolBinding(
    namespace = "std.protocol.istreamoffer",
    name = "IStreamOffer",
    availability = HaraAvailability.CAPABILITY_GATED,
    capability = "native-runtime-protocols")
public interface IStreamOffer {
  @HaraMethod(value = "offer", arity = 2)
  boolean offer(Object value);
}
