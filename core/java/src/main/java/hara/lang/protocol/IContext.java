package hara.lang.protocol;

import hara.lang.declaration.HaraMethod;
import hara.lang.declaration.HaraProtocolBinding;

/** Callable root context with optional context-runtime operations. */
@HaraProtocolBinding(namespace = "std.protocol.icontext", name = "IContext")
public interface IContext {
  @HaraMethod(value = "call", arity = -1, variadic = true)
  Object call(Object... args);

  default Object rawEval(String source) {
    return source;
  }

  default Object initPtr(IPointer pointer) {
    return null;
  }

  default Object tagsPtr(IPointer pointer) {
    return null;
  }

  default Object derefPtr(IPointer pointer) {
    return pointer;
  }

  default Object displayPtr(IPointer pointer) {
    return pointer;
  }

  default Object invokePtr(IPointer pointer, Object[] args) {
    return call(args == null ? new Object[0] : args);
  }

  default Object transformInPtr(IPointer pointer, Object[] args) {
    return args;
  }

  default Object transformOutPtr(IPointer pointer, Object value) {
    return value;
  }
}
