package hara.lang.protocol;

import hara.lang.declaration.HaraAvailability;
import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

@HaraProtocolBinding(
    namespace = "std.protocol.imatch",
    name = "IMatch",
    availability = HaraAvailability.CAPABILITY_GATED,
    capability = "native-runtime-protocols")
public interface IMatch {
  @HaraMethod(value = "match-value", arity = 2)
  Object matchValue(Object value);
}
