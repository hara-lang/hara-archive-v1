package hara.truffle;

import hara.lang.protocol.IFn;

/**
 * Marker for the runtime's own builtin function objects. Specialized invoke paths may call
 * {@link #apply(Object[])} directly instead of dispatching through the IFn protocol. The
 * result is identical: builtins never implement {@code ILookup}, {@code ISequentialLookupType},
 * or {@code ISetType}, so the protocol invoker always degrades to {@link IFn#applyAsArray}.
 */
public interface HaraBuiltinFunction {
  Object apply(Object[] arguments);

  default boolean recordsExceptionCreation() {
    return false;
  }
}
