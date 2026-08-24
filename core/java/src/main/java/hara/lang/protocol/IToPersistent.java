package hara.lang.protocol;

import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

@HaraProtocolBinding(
    namespace = "std.protocol.itopersistent", name = "IToPersistent", parents = {"IMutable"})
public interface IToPersistent extends IMutable {
  @HaraMethod(value = "to-persistent", arity = 1)
  IPersistent toPersistent();
}
