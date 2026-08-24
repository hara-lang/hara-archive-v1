package hara.lang.protocol;

import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

/** Context-aware application and transformation protocol. */
@HaraProtocolBinding(namespace = "std.protocol.iapplicable", name = "IApplicable")
public interface IApplicable {
  @HaraMethod(value = "apply-in", arity = 3)
  Object applyIn(Object runtime, Object[] args);

  @HaraMethod(value = "apply-default", arity = 1)
  default Object applyDefault() {
    return this;
  }

  @HaraMethod(value = "transform-in", arity = 3)
  Object transformIn(Object runtime, Object[] args);

  @HaraMethod(value = "transform-out", arity = 4)
  Object transformOut(Object runtime, Object[] args, Object value);
}
