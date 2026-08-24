package hara.lang.protocol;

import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

/** Reduces a value with or without an explicit initial accumulator. */
@HaraProtocolBinding(namespace = "std.protocol.ireduce", name = "IReduce")
public interface IReduce {
  Object reduce(Object function);

  @HaraMethod(value = "reduce", arity = -1, variadic = true)
  Object reduce(Object function, Object initial);
}
