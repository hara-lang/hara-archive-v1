package hara.lang.protocol;

import hara.lang.declaration.HaraAvailability;
import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

/** Live work run with asynchronous result, events, and cancellation. */
@HaraProtocolBinding(
    namespace = "std.protocol.iworkrun",
    name = "IWorkRun",
    parents = {"IWorkRef", "IClosed"},
    availability = HaraAvailability.CAPABILITY_GATED,
    capability = "native-runtime-protocols")
public interface IWorkRun extends IWorkRef, IClosed {
  @HaraMethod(value = "work-status", arity = 1)
  Object workStatus();

  @HaraMethod(value = "work-result", arity = 1)
  IPromise workResult();

  @HaraMethod(value = "work-events", arity = 2)
  IStream workEvents(Object options);

  @HaraMethod(value = "work-cancel", arity = 2)
  IPromise workCancel(Object reason);
}
