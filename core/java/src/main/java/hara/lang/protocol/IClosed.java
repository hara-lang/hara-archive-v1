package hara.lang.protocol;

import hara.lang.declaration.HaraAvailability;
import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

/** Observable terminal-state capability. */
@HaraProtocolBinding(
    namespace = "std.protocol.iclosed",
    name = "IClosed",
    availability = HaraAvailability.CAPABILITY_GATED,
    capability = "native-runtime-protocols")
public interface IClosed {
  @HaraMethod(value = "closed?", arity = 1)
  boolean closed();
}
