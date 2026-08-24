package hara.lang.protocol;

import hara.lang.declaration.HaraAvailability;
import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

/** Process-owned host that admits work and resolves live run handles. */
@HaraProtocolBinding(
    namespace = "std.protocol.iworkhost",
    name = "IWorkHost",
    parents = {"IComponent"},
    availability = HaraAvailability.CAPABILITY_GATED,
    capability = "native-runtime-protocols")
public interface IWorkHost extends IComponent {
  @HaraMethod(value = "work-submit", arity = 4)
  IWorkRun workSubmit(Object work, Object input, Object options);

  @HaraMethod(value = "work-resolve", arity = 2)
  IWorkRun workResolve(Object reference);
}
