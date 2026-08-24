package hara.lang.protocol;

import hara.lang.declaration.HaraAvailability;
import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

/** Executes one leaf request from the Work algebra. */
@HaraProtocolBinding(
    namespace = "std.protocol.iworkexecutor",
    name = "IWorkExecutor",
    availability = HaraAvailability.CAPABILITY_GATED,
    capability = "native-runtime-protocols")
public interface IWorkExecutor {
  @HaraMethod(value = "work-execute", arity = 2)
  Object workExecute(Object request);
}
