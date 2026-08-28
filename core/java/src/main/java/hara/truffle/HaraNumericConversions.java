package hara.truffle;

import com.oracle.truffle.api.CompilerDirectives.TruffleBoundary;
import hara.lang.base.NumUtils;
import java.math.BigDecimal;
import java.math.BigInteger;

/** Explicit, checked numeric conversions used at Hara host boundaries. */
final class HaraNumericConversions {
  private HaraNumericConversions() {}

  static boolean isNumeric(Object input) {
    return numericValue(input) instanceof Number;
  }

  static boolean fitsLong(Object input) {
    return NumUtils.isLongValue(numericValue(input));
  }

  @TruffleBoundary
  static long toLong(Object input) {
    try {
      return boundaryInteger(input).longValueExact();
    } catch (ArithmeticException error) {
      throw cannotConvert("long", input);
    }
  }

  @TruffleBoundary
  static long toLong(Object input, String operation) {
    try {
      return boundaryInteger(input).longValueExact();
    } catch (ArithmeticException error) {
      throw new HaraException(operation + " expects an exact signed 64-bit integer");
    }
  }

  /** Converts a numeric value to i64 by truncating its fractional part toward zero. */
  @TruffleBoundary
  static long toLongTruncating(Object input) {
    Object value = numericValue(input);
    try {
      if (value instanceof BigInteger integer) return integer.longValueExact();
      if (value instanceof Byte
          || value instanceof Short
          || value instanceof Integer
          || value instanceof Long) {
        return ((Number) value).longValue();
      }
      if (value instanceof Double || value instanceof Float) {
        double floating = ((Number) value).doubleValue();
        if (Double.isFinite(floating)) {
          return BigDecimal.valueOf(floating).toBigInteger().longValueExact();
        }
      }
    } catch (ArithmeticException error) {
      throw cannotConvert("long", input);
    }
    throw cannotConvert("long", input);
  }

  @TruffleBoundary
  static int toInt(Object input) {
    try {
      return boundaryInteger(input).intValueExact();
    } catch (ArithmeticException error) {
      throw cannotConvert("i32", input);
    }
  }

  @TruffleBoundary
  static int toInt(Object input, String operation) {
    try {
      return boundaryInteger(input).intValueExact();
    } catch (ArithmeticException error) {
      throw new HaraException(operation + " expects an exact signed 32-bit integer");
    }
  }

  @TruffleBoundary
  static double toDouble(Object input) {
    Object value = numericValue(input);
    if (value instanceof Double || value instanceof Float) {
      return requireFinite(((Number) value).doubleValue());
    }
    if (value instanceof Number number) {
      double converted = number.doubleValue();
      if (Double.isFinite(converted)) return converted;
    }
    throw cannotConvert("double", input);
  }

  static double requireFinite(double value) {
    if (!Double.isFinite(value)) throw new HaraException("non-finite number");
    return value;
  }

  @TruffleBoundary
  static Number toNumber(Object input, String operation) {
    Object value = numericValue(input);
    if (value instanceof Number number) return number;
    throw new HaraException(operation + " expects a numeric value");
  }

  @TruffleBoundary
  static BigInteger toInteger(Object input, String operation) {
    Object value = numericValue(input);
    if (value instanceof BigInteger integer) return integer;
    if (value instanceof Byte
        || value instanceof Short
        || value instanceof Integer
        || value instanceof Long) {
      return BigInteger.valueOf(((Number) value).longValue());
    }
    throw new HaraException(operation + " expects integers");
  }

  @TruffleBoundary
  static int toShiftDistance(Object input, String operation) {
    BigInteger distance = toInteger(input, operation);
    if (distance.signum() < 0 || distance.bitLength() > 31) {
      throw new HaraException(operation + " distance is outside the host index range");
    }
    return distance.intValue();
  }

  private static BigInteger boundaryInteger(Object input) {
    Object value = numericValue(input);
    if (value instanceof BigInteger integer) return integer;
    if (value instanceof Byte
        || value instanceof Short
        || value instanceof Integer
        || value instanceof Long) {
      return BigInteger.valueOf(((Number) value).longValue());
    }
    if (value instanceof Double || value instanceof Float) {
      double floating = ((Number) value).doubleValue();
      if (Double.isFinite(floating)) {
        return BigDecimal.valueOf(floating).toBigIntegerExact();
      }
    }
    throw new ArithmeticException("not an exact integer");
  }

  private static Object numericValue(Object input) {
    Object value = HaraBox.unwrap(input);
    if (value instanceof HaraBigInteger integer) return integer.value();
    return value;
  }

  private static HaraException cannotConvert(String target, Object value) {
    return new HaraException(
        target + " expects an exactly representable numeric value, got "
            + (value == null ? "nil" : value));
  }
}
