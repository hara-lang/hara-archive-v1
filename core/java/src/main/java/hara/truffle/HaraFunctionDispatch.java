package hara.truffle;

import hara.lang.data.types.ISequentialLookupType;
import hara.lang.protocol.ISetType;
import hara.lang.protocol.IFn;
import hara.lang.protocol.ILookup;
import java.util.Arrays;

/** Shared callable dispatch for protocol invocation and Truffle interop execution. */
final class HaraFunctionDispatch {
  private HaraFunctionDispatch() {}

  static Object invoke(Object receiver, Object[] arguments) {
    IFn<?, ?, ?> function = (IFn<?, ?, ?>) receiver;
    Object[] values =
        Arrays.stream(arguments)
            .map(HaraProtocolExtensions::unwrapArgument)
            .toArray(Object[]::new);
    if (function instanceof ILookup) {
      return HaraProtocolExtensions.lookupValue((ILookup<?, ?>) function, values);
    }
    if (function instanceof ISequentialLookupType && values.length == 1) {
      return ((ISequentialLookupType<?>) function)
          .nth(HaraNumericConversions.toLong(values[0], "IFn sequential lookup"));
    }
    if (function instanceof ISetType) {
      return HaraProtocolExtensions.setValue((ISetType<?>) function, values);
    }
    return HaraProtocolExtensions.applyFunction(function, values);
  }
}
