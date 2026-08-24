package hara.lang.protocol;

import hara.lang.declaration.HaraAvailability;
import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

/** Immutable, transportable identity for one work run. */
@HaraProtocolBinding(
    namespace = "std.protocol.iworkref",
    name = "IWorkRef",
    availability = HaraAvailability.CAPABILITY_GATED,
    capability = "native-runtime-protocols")
public interface IWorkRef {
  @HaraMethod(value = "work-id", arity = 1)
  Object workId();
}
