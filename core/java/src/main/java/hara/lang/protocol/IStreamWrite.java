package hara.lang.protocol;

import hara.lang.declaration.HaraAvailability;
import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

/** Asynchronous writable stream capability. */
@HaraProtocolBinding(
    namespace = "std.protocol.istreamwrite",
    name = "IStreamWrite",
    availability = HaraAvailability.CAPABILITY_GATED,
    capability = "native-runtime-protocols")
public interface IStreamWrite {
  @HaraMethod(value = "write", arity = 2)
  Object write(Object value);
}
