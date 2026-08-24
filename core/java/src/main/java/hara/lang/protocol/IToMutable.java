package hara.lang.protocol;

import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

@HaraProtocolBinding(
    namespace = "std.protocol.itomutable", name = "IToMutable", parents = {"IPersistent"})
public interface IToMutable extends IPersistent {
  @HaraMethod(value = "to-mutable", arity = 1)
  IMutable toMutable();
}
