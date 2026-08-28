package hara.lang.base;

import java.math.BigDecimal;
import java.math.BigInteger;

public class NumUtils {

  public static final NumOps.BigIntegerOps BIGINT_OPS = new NumOps.BigIntegerOps();
  public static final NumOps.DoubleOps DOUBLE_OPS = new NumOps.DoubleOps();
  public static final NumOps.LongOps LONG_OPS = new NumOps.LongOps();

  public static enum Category {
    FLOATING,
    INTEGER
  }

  public static Number normalizeInteger(BigInteger value) {
    if (value.bitLength() < Long.SIZE) return Long.valueOf(value.longValue());
    return value;
  }

  /** Returns true when value has the language's canonical signed 64-bit width. */
  public static boolean isLongValue(Object value) {
    if (value instanceof Byte
        || value instanceof Short
        || value instanceof Integer
        || value instanceof Long) {
      return true;
    }
    if (value instanceof BigInteger integer) {
      return integer.bitLength() < Long.SIZE;
    }
    return false;
  }

  /** Returns true only for arbitrary-precision integers outside signed 64-bit range. */
  public static boolean isBigIntegerValue(Object value) {
    return value instanceof BigInteger && !isLongValue(value);
  }

  public static BigInteger toBigInteger(Object x) {
    if (x instanceof BigInteger) return (BigInteger) x;
    if (x instanceof Double || x instanceof Float) {
      double value = ((Number) x).doubleValue();
      if (!Double.isFinite(value)) {
        throw new ArithmeticException("non-finite floating-point value is not an integer");
      }
      return BigDecimal.valueOf(value).toBigIntegerExact();
    }
    return BigInteger.valueOf(((Number) x).longValue());
  }

  public static double requireFinite(double value) {
    if (!Double.isFinite(value)) throw new ArithmeticException("non-finite number");
    return value;
  }

  public static int throwIntOverflow() {
    throw new ArithmeticException("integer overflow");
  }

  public static long gcd(long u, long v) {
    while (v != 0) {
      long r = u % v;
      u = v;
      v = r;
    }
    return u;
  }

  public static long add(long x, long y) {
    long ret = x + y;
    if ((ret ^ x) < 0 && (ret ^ y) < 0) return throwIntOverflow();
    return ret;
  }

  public static long dec(long x) {
    if (x == Long.MIN_VALUE) return throwIntOverflow();
    return x - 1;
  }

  public static long inc(long x) {
    if (x == Long.MAX_VALUE) return throwIntOverflow();
    return x + 1;
  }

  public static long multiply(long x, long y) {
    if (x == Long.MIN_VALUE && y < 0) return throwIntOverflow();
    long ret = x * y;
    if (y != 0 && ret / y != x) return throwIntOverflow();
    return ret;
  }

  public static long minus(long x) {
    if (x == Long.MIN_VALUE) return throwIntOverflow();
    return -x;
  }

  public static long minus(long x, long y) {
    long ret = x - y;
    if (((ret ^ x) < 0 && (ret ^ ~y) < 0)) return throwIntOverflow();
    return ret;
  }

  public static long unchecked_add(long x, long y) {
    return x + y;
  }

  public static long unchecked_dec(long x) {
    return x - 1;
  }

  public static long unchecked_inc(long x) {
    return x + 1;
  }

  public static long unchecked_multiply(long x, long y) {
    return x * y;
  }

  public static long unchecked_minus(long x) {
    return -x;
  }

  public static BigInteger bitOpsCast(Object x) {
    Class<? extends Object> xc = x.getClass();

    if (xc == Long.class || xc == Integer.class || xc == Short.class || xc == Byte.class)
      return BigInteger.valueOf(((Number) x).longValue());
    else if (x instanceof BigInteger) return (BigInteger) x;
    throw new IllegalArgumentException("bit operation requires an integer: " + xc.getName());
  }

  public static int bitIndex(Object value) {
    BigInteger index = bitOpsCast(value);
    if (index.signum() < 0 || index.bitLength() > 31) {
      throw new IllegalArgumentException("bit index is out of range: " + index);
    }
    return index.intValue();
  }
}
