package hara.truffle;

import hara.lang.data.Symbol;
import java.math.BigInteger;

/**
 * Canonical synchronous target dispatcher used by Java bytecode/Truffle seams.
 *
 * <p>Protocol targets use the annotation-backed protocol registry. Native targets resolve the
 * already-installed native Var, so this class does not maintain a second protocol- or
 * native-specific installer table.
 */
final class HaraTargetRuntime {
  enum ResultMode {
    HANDLE,
    I64,
    BOOL
  }

  private HaraTargetRuntime() {}

  static Object invoke(HaraContext context, String target, Object[] arguments) {
    return invoke(context, target, arguments, ResultMode.HANDLE);
  }

  static Object invoke(
      HaraContext context, String target, Object[] arguments, ResultMode resultMode) {
    if (target == null || target.isBlank()) throw new HaraException("target/invalid: empty target");
    context.publishWholeWasmProtocolCall(target, arguments.length, resultMode, "enter");
    try {
      Object result = dispatch(context, target, arguments);
      Object encoded = encode(result, resultMode);
      context.publishWholeWasmProtocolCall(target, arguments.length, resultMode, "return");
      return encoded;
    } catch (RuntimeException failure) {
      context.publishWholeWasmProtocolCall(target, arguments.length, resultMode, "error");
      throw failure;
    }
  }

  private static Object dispatch(HaraContext context, String target, Object[] arguments) {
    if (target.startsWith("std.protocol.")) {
      int slash = target.lastIndexOf('/');
      int protocolEnd = target.lastIndexOf('.', slash < 0 ? target.length() : slash);
      if (slash < 0 || protocolEnd < "std.protocol.".length()) {
        throw new HaraException("target/invalid: " + target);
      }
      String protocol = target.substring(protocolEnd + 1, slash);
      String method = target.substring(slash + 1);
      return context.invokeProtocol(protocol, method, arguments);
    }
    if (target.startsWith("std.native.")) {
      HaraVar variable = context.resolve(Symbol.create(target));
      if (variable == null) throw new HaraException("target/unbound: " + target);
      return context.invokeCallable(variable.deref(), arguments);
    }
    throw new HaraException("target/unknown: " + target);
  }

  private static Object encode(Object value, ResultMode resultMode) {
    return switch (resultMode) {
      case HANDLE -> value;
      case BOOL -> {
        Object unwrapped = HaraBox.unwrap(value);
        if (!(unwrapped instanceof Boolean result)) {
          throw new HaraException("target/result: expected boolean");
        }
        yield result;
      }
      case I64 -> {
        Object unwrapped = HaraBox.unwrap(value);
        if (unwrapped instanceof Byte || unwrapped instanceof Short || unwrapped instanceof Integer
            || unwrapped instanceof Long) {
          yield ((Number) unwrapped).longValue();
        }
        if (unwrapped instanceof BigInteger integer) {
          try {
            yield integer.longValueExact();
          } catch (ArithmeticException error) {
            throw new HaraException("target/result: integer overflow");
          }
        }
        throw new HaraException("target/result: expected i64");
      }
    };
  }
}
