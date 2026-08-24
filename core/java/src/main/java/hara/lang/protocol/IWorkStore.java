package hara.lang.protocol;

import hara.lang.declaration.HaraAvailability;
import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

/** Queries and atomically journals managed Work execution state. */
@HaraProtocolBinding(
    namespace = "std.protocol.iworkstore",
    name = "IWorkStore",
    availability = HaraAvailability.CAPABILITY_GATED,
    capability = "native-runtime-protocols")
public interface IWorkStore {
  @HaraMethod(value = "work-query", arity = 2)
  Object workQuery(Object query);

  @HaraMethod(value = "work-transact", arity = 2)
  Object workTransact(Object transition);
}
