package hara.lang.protocol;

import hara.lang.declaration.HaraAvailability;
import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

@HaraProtocolBinding(
    namespace = "std.protocol.istringlike",
    name = "IStringLike",
    availability = HaraAvailability.CAPABILITY_GATED,
    capability = "native-runtime-protocols")
public interface IStringLike {
  @HaraMethod(value = "to-string", arity = 1)
  Object toStringValue();

  @HaraMethod(value = "from-string", arity = 2)
  Object fromString(Object value);
}
