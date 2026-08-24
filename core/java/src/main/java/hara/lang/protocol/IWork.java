package hara.lang.protocol;

import hara.lang.declaration.HaraAvailability;
import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

/** A stable, side-effect-free description of executable work. */
@HaraProtocolBinding(
    namespace = "std.protocol.iwork",
    name = "IWork",
    availability = HaraAvailability.CAPABILITY_GATED,
    capability = "native-runtime-protocols")
public interface IWork {
  @HaraMethod(value = "work-spec", arity = 1)
  Object workSpec();
}
