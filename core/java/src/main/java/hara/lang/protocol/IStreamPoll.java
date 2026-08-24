package hara.lang.protocol;

import hara.lang.declaration.HaraAvailability;
import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

/** Non-blocking readable stream capability. */
@HaraProtocolBinding(
    namespace = "std.protocol.istreampoll",
    name = "IStreamPoll",
    availability = HaraAvailability.CAPABILITY_GATED,
    capability = "native-runtime-protocols")
public interface IStreamPoll {
  @HaraMethod(value = "poll", arity = 1)
  Object poll();
}
