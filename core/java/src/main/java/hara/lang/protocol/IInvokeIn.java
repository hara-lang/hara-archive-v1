package hara.lang.protocol;

import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

@HaraProtocolBinding(namespace = "std.protocol.iinvokein", name = "IInvokeIn")
public interface IInvokeIn {
  @HaraMethod(value = "invoke-in", arity = -1, variadic = true)
  Object invokeIn(IContext context, Object... args);
}
