package hara.lang.protocol;

import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

/** A resumable computation with an explicit lifecycle. */
@HaraProtocolBinding(
    namespace = "std.protocol.icoroutine", name = "ICoroutine", parents = {"IClose"})
public interface ICoroutine extends IClose {
  @HaraMethod(value = "status", arity = 1)
  Object status();

  @HaraMethod(value = "resume", arity = -1, variadic = true)
  Object resume(Object... arguments);
}
