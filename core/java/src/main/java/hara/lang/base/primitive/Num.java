package hara.lang.base.primitive;

import hara.lang.base.NumOps;
import hara.lang.base.NumUtils;

import java.math.BigInteger;

public interface Num {

  static double asDouble(Object value) {
    return NumUtils.requireFinite(((Number) value).doubleValue());
  }

  public static double add(double x, double y) {
    return x + y;
  }

  public static double add(double x, long y) {
    return x + y;
  }

  public static double add(double x, Object y) {
    return add(x, asDouble(y));
  }

  public static double add(long x, double y) {
    return x + y;
  }

  public static long add(long x, long y) {
    return NumUtils.add(x, y);
  }

  public static Number add(long x, Object y) {
    return add((Object) x, y);
  }

  public static double add(Object x, double y) {
    return add(asDouble(x), y);
  }

  public static Number add(Object x, long y) {
    return add(x, (Object) y);
  }

  public static Number add(Object x, Object y) {
    return ops(x).combine(ops(y)).add((Number) x, (Number) y);
  }

  public static double addP(double x, double y) {
    return x + y;
  }

  public static double addP(double x, long y) {
    return x + y;
  }

  public static double addP(double x, Object y) {
    return addP(x, asDouble(y));
  }

  public static double addP(long x, double y) {
    return x + y;
  }

  public static Number addP(long x, long y) {
    long ret = x + y;
    if ((ret ^ x) < 0 && (ret ^ y) < 0) return addP((Number) x, (Number) y);
    return num(ret);
  }

  public static Number addP(long x, Object y) {
    return addP((Object) x, y);
  }

  public static double addP(Object x, double y) {
    return addP(asDouble(x), y);
  }

  public static Number addP(Object x, long y) {
    return addP(x, (Object) y);
  }

  public static Number addP(Object x, Object y) {
    return ops(x).combine(ops(y)).addP((Number) x, (Number) y);
  }

  public static Number and(long x, long y) {
    return num(x & y);
  }

  public static Number and(long x, Object y) {
    return and((Object) x, y);
  }

  public static Number and(Object x, long y) {
    return and(x, (Object) y);
  }

  public static Number and(Object x, Object y) {
    return NumUtils.normalizeInteger(NumUtils.bitOpsCast(x).and(NumUtils.bitOpsCast(y)));
  }

  public static Number andNot(long x, long y) {
    return num(x & ~y);
  }

  public static Number andNot(long x, Object y) {
    return andNot((Object) x, y);
  }

  public static Number andNot(Object x, long y) {
    return andNot(x, (Object) y);
  }

  public static Number andNot(Object x, Object y) {
    return NumUtils.normalizeInteger(NumUtils.bitOpsCast(x).andNot(NumUtils.bitOpsCast(y)));
  }

  // @WarnBoxedMath(false)
  public static boolean[] booleans(Object array) {
    return (boolean[]) array;
  }

  // @WarnBoxedMath(false)
  public static byte[] bytes(Object array) {
    return (byte[]) array;
  }

  static NumUtils.Category category(Object x) {
    Class<? extends Object> xc = x.getClass();

    if (xc == Integer.class) return NumUtils.Category.INTEGER;
    else if (xc == Double.class) return NumUtils.Category.FLOATING;
    else if (xc == Long.class) return NumUtils.Category.INTEGER;
    else if (xc == Float.class) return NumUtils.Category.FLOATING;
    else return NumUtils.Category.INTEGER;
  }

  // @WarnBoxedMath(false)
  public static char[] chars(Object array) {
    return (char[]) array;
  }

  public static Number clearBit(long x, long n) {
    return clearBit((Object) x, (Object) n);
  }

  public static Number clearBit(long x, Object y) {
    return clearBit((Object) x, y);
  }

  public static Number clearBit(Object x, long y) {
    return clearBit(x, (Object) y);
  }

  public static Number clearBit(Object x, Object y) {
    return NumUtils.normalizeInteger(NumUtils.bitOpsCast(x).clearBit(NumUtils.bitIndex(y)));
  }

  public static int compare(Number x, Number y) {
    NumOps ops = ops(x).combine(ops(y));
    if (ops.lt(x, y)) return -1;
    else if (ops.lt(y, x)) return 1;
    return 0;
  }

  public static double dec(double x) {
    return x - 1;
  }

  public static long dec(long x) {
    return NumUtils.dec(x);
  }

  public static Number dec(Object x) {
    return ops(x).dec((Number) x);
  }

  public static double decP(double x) {
    return x - 1;
  }

  public static Number decP(long x) {
    if (x == Long.MIN_VALUE) return NumUtils.BIGINT_OPS.dec(x);
    return num(x - 1);
  }

  public static Number decP(Object x) {
    return ops(x).decP((Number) x);
  }

  public static Number divide(BigInteger n, BigInteger d) {
    if (d.equals(BigInteger.ZERO)) throw new ArithmeticException("Divide by zero");
    return NumUtils.normalizeInteger(n.divide(d));
  }

  public static BigInteger toBigInteger(Number value) {
    return NumUtils.toBigInteger(value);
  }

  public static double divide(double x, double y) {
    if (y == 0.0d) throw new ArithmeticException("Divide by zero");
    return NumUtils.requireFinite(x / y);
  }

  public static double divide(double x, long y) {
    return divide(x, (double) y);
  }

  public static double divide(double x, Object y) {
    return x / asDouble(y);
  };

  public static double divide(long x, double y) {
    return divide((double) x, y);
  }

  public static Number divide(long x, long y) {
    return divide((Number) x, (Number) y);
  }

  public static Number divide(long x, Object y) {
    return divide((Object) x, y);
  }

  public static double divide(Object x, double y) {
    return asDouble(x) / y;
  }

  public static Number divide(Object x, long y) {
    return divide(x, (Object) y);
  }

  public static Number divide(Object x, Object y) {
    NumOps yops = ops(y);
    if (yops.isZero((Number) y)) throw new ArithmeticException("Divide by zero");
    return ops(x).combine(yops).divide((Number) x, (Number) y);
  }

  // @WarnBoxedMath(false)
  public static double[] doubles(Object array) {
    return (double[]) array;
  }

  public static boolean eq(double x, double y) {
    return x == y;
  }

  public static boolean eq(double x, long y) {
    return x == y;
  }

  public static boolean eq(double x, Object y) {
    return x == asDouble(y);
  }

  public static boolean eq(long x, double y) {
    return x == y;
  }

  public static boolean eq(long x, long y) {
    return x == y;
  }

  public static boolean eq(long x, Object y) {
    return eq((Object) x, y);
  }

  public static boolean eq(Number x, Number y) {
    return ops(x).combine(ops(y)).eq(x, y);
  }

  public static boolean eq(Object x, double y) {
    return asDouble(x) == y;
  }

  public static boolean eq(Object x, long y) {
    return eq(x, (Object) y);
  }

  public static boolean eq(Object x, Object y) {
    return eq((Number) x, (Number) y);
  }

  public static boolean equal(Number x, Number y) {
    return ops(x).combine(ops(y)).eq(x, y);
  }

  public static Number flipBit(long x, long n) {
    return flipBit((Object) x, (Object) n);
  }

  public static Number flipBit(long x, Object y) {
    return flipBit((Object) x, y);
  }

  public static Number flipBit(Object x, long y) {
    return flipBit(x, (Object) y);
  }

  public static Number flipBit(Object x, Object y) {
    return NumUtils.normalizeInteger(NumUtils.bitOpsCast(x).flipBit(NumUtils.bitIndex(y)));
  }

  // @WarnBoxedMath(false)
  public static float[] floats(Object array) {
    return (float[]) array;
  }

  public static boolean gt(double x, double y) {
    return x > y;
  }

  public static boolean gt(double x, long y) {
    return x > y;
  }

  public static boolean gt(double x, Object y) {
    return x > asDouble(y);
  }

  public static boolean gt(long x, double y) {
    return x > y;
  }

  public static boolean gt(long x, long y) {
    return x > y;
  }

  public static boolean gt(long x, Object y) {
    return gt((Object) x, y);
  }

  public static boolean gt(Object x, double y) {
    return asDouble(x) > y;
  }

  public static boolean gt(Object x, long y) {
    return gt(x, (Object) y);
  }

  public static boolean gt(Object x, Object y) {
    return ops(x).combine(ops(y)).lt((Number) y, (Number) x);
  }

  public static boolean gte(double x, double y) {
    return x >= y;
  }

  public static boolean gte(double x, long y) {
    return x >= y;
  }

  public static boolean gte(double x, Object y) {
    return x >= asDouble(y);
  }

  public static boolean gte(long x, double y) {
    return x >= y;
  }

  public static boolean gte(long x, long y) {
    return x >= y;
  }

  public static boolean gte(long x, Object y) {
    return gte((Object) x, y);
  }

  public static boolean gte(Object x, double y) {
    return asDouble(x) >= y;
  }

  public static boolean gte(Object x, long y) {
    return gte(x, (Object) y);
  }

  public static boolean gte(Object x, Object y) {
    return ops(x).combine(ops(y)).gte((Number) x, (Number) y);
  }

  public static double inc(double x) {
    return x + 1;
  }

  public static long inc(long x) {
    return NumUtils.inc(x);
  }

  public static Number inc(Object x) {
    return ops(x).inc((Number) x);
  }

  public static double incP(double x) {
    return x + 1;
  }

  public static Number incP(long x) {
    if (x == Long.MAX_VALUE) return NumUtils.BIGINT_OPS.inc(x);
    return num(x + 1);
  }

  public static Number incP(Object x) {
    return ops(x).incP((Number) x);
  }

  // @WarnBoxedMath(false)
  public static int[] ints(Object array) {
    return (int[]) array;
  }

  static boolean isNaN(Object x) {
    return (x instanceof Double) && ((Double) x).isNaN()
        || (x instanceof Float) && ((Float) x).isNaN();
  }

  public static boolean isNeg(double x) {
    return x < 0;
  }

  public static boolean isNeg(long x) {
    return x < 0;
  }

  public static boolean isNeg(Object x) {
    return ops(x).isNeg((Number) x);
  }

  public static boolean isPos(double x) {
    return x > 0;
  }

  public static boolean isPos(long x) {
    return x > 0;
  }

  public static boolean isPos(Object x) {
    return ops(x).isPos((Number) x);
  }

  public static boolean isZero(double x) {
    return x == 0;
  }

  public static boolean isZero(long x) {
    return x == 0;
  }

  public static boolean isZero(Object x) {
    return ops(x).isZero((Number) x);
  }

  // @WarnBoxedMath(false)
  public static long[] longs(Object array) {
    return (long[]) array;
  }

  public static boolean lt(double x, double y) {
    return x < y;
  }

  public static boolean lt(double x, long y) {
    return x < y;
  }

  public static boolean lt(double x, Object y) {
    return x < asDouble(y);
  }

  public static boolean lt(long x, double y) {
    return x < y;
  }

  public static boolean lt(long x, long y) {
    return x < y;
  }

  public static boolean lt(long x, Object y) {
    return lt((Object) x, y);
  }

  public static boolean lt(Object x, double y) {
    return asDouble(x) < y;
  }

  public static boolean lt(Object x, long y) {
    return lt(x, (Object) y);
  }

  public static boolean lt(Object x, Object y) {
    return ops(x).combine(ops(y)).lt((Number) x, (Number) y);
  }

  public static boolean lte(double x, double y) {
    return x <= y;
  }

  public static boolean lte(double x, long y) {
    return x <= y;
  }

  public static boolean lte(double x, Object y) {
    return x <= asDouble(y);
  }

  public static boolean lte(long x, double y) {
    return x <= y;
  }

  public static boolean lte(long x, long y) {
    return x <= y;
  }

  public static boolean lte(long x, Object y) {
    return lte((Object) x, y);
  }

  public static boolean lte(Object x, double y) {
    return asDouble(x) <= y;
  }

  public static boolean lte(Object x, long y) {
    return lte(x, (Object) y);
  }

  public static boolean lte(Object x, Object y) {
    return ops(x).combine(ops(y)).lte((Number) x, (Number) y);
  }

  public static Number mod(Object x, Object y) {
    // `mod` is the canonical named remainder operator in the Hara
    // bytecode contract; preserve the dividend sign at this boundary.
    return remainder(x, y);
  }

  public static double minus(double x) {
    return -x;
  }

  public static double minus(double x, double y) {
    return x - y;
  }

  public static double minus(double x, long y) {
    return x - y;
  }

  public static double minus(double x, Object y) {
    return minus(x, asDouble(y));
  }

  public static long minus(long x) {
    return NumUtils.minus(x);
  }

  public static double minus(long x, double y) {
    return x - y;
  }

  public static long minus(long x, long y) {
    return NumUtils.minus(x, y);
  }

  public static Number minus(long x, Object y) {
    return minus((Object) x, y);
  }

  public static Number minus(Object x) {
    return ops(x).negate((Number) x);
  }

  public static double minus(Object x, double y) {
    return minus(asDouble(x), y);
  }

  public static Number minus(Object x, long y) {
    return minus(x, (Object) y);
  }

  public static Number minus(Object x, Object y) {
    NumOps yops = ops(y);
    return ops(x).combine(yops).add((Number) x, yops.negate((Number) y));
  }

  public static double minusP(double x) {
    return -x;
  }

  public static double minusP(double x, double y) {
    return x - y;
  }

  public static double minusP(double x, long y) {
    return x - y;
  }

  public static double minusP(double x, Object y) {
    return minus(x, asDouble(y));
  }

  public static Number minusP(long x) {
    if (x == Long.MIN_VALUE) return BigInteger.valueOf(x).negate();
    return num(-x);
  }

  public static double minusP(long x, double y) {
    return x - y;
  }

  public static Number minusP(long x, long y) {
    long ret = x - y;
    if (((ret ^ x) < 0 && (ret ^ ~y) < 0)) return minusP((Number) x, (Number) y);
    return num(ret);
  }

  public static Number minusP(long x, Object y) {
    return minusP((Object) x, y);
  }

  public static Number minusP(Object x) {
    return ops(x).negateP((Number) x);
  }

  public static double minusP(Object x, double y) {
    return minus(asDouble(x), y);
  }

  public static Number minusP(Object x, long y) {
    return minusP(x, (Object) y);
  }

  public static Number minusP(Object x, Object y) {
    NumOps yops = ops(y);
    Number negativeY = yops.negateP((Number) y);
    NumOps negativeYOps = ops(negativeY);
    return ops(x).combine(negativeYOps).addP((Number) x, negativeY);
  }

  public static double multiply(double x, double y) {
    return x * y;
  }

  public static double multiply(double x, long y) {
    return x * y;
  }

  public static double multiply(double x, Object y) {
    return multiply(x, asDouble(y));
  }

  public static double multiply(long x, double y) {
    return x * y;
  }

  public static long multiply(long x, long y) {
    return NumUtils.multiply(x, y);
  }

  public static Number multiply(long x, Object y) {
    return multiply((Object) x, y);
  }

  public static double multiply(Object x, double y) {
    return multiply(asDouble(x), y);
  }

  public static Number multiply(Object x, long y) {
    return multiply(x, (Object) y);
  }

  public static Number multiply(Object x, Object y) {
    return ops(x).combine(ops(y)).multiply((Number) x, (Number) y);
  }

  public static double multiplyP(double x, double y) {
    return x * y;
  }

  public static double multiplyP(double x, long y) {
    return x * y;
  }

  public static double multiplyP(double x, Object y) {
    return multiplyP(x, asDouble(y));
  }

  public static double multiplyP(long x, double y) {
    return x * y;
  }

  public static Number multiplyP(long x, long y) {
    if (x == Long.MIN_VALUE && y < 0) return multiplyP((Number) x, (Number) y);
    long ret = x * y;
    if (y != 0 && ret / y != x) return multiplyP((Number) x, (Number) y);
    return num(ret);
  }

  public static Number multiplyP(long x, Object y) {
    return multiplyP((Object) x, y);
  }

  public static double multiplyP(Object x, double y) {
    return multiplyP(asDouble(x), y);
  }

  public static Number multiplyP(Object x, long y) {
    return multiplyP(x, (Object) y);
  }

  public static Number multiplyP(Object x, Object y) {
    return ops(x).combine(ops(y)).multiplyP((Number) x, (Number) y);
  }

  public static Number not(long x) {
    return num(~x);
  }

  public static Number not(Object x) {
    return NumUtils.normalizeInteger(NumUtils.bitOpsCast(x).not());
  }

  public static Number num(double x) {
    return Double.valueOf(NumUtils.requireFinite(x));
  }

  public static Number num(float x) {
    NumUtils.requireFinite(x);
    return Float.valueOf(x);
  }

  public static Number num(long x) {
    return Long.valueOf(x);
  }

  public static Number num(Object x) {
    if (x instanceof Byte
        || x instanceof Short
        || x instanceof Integer
        || x instanceof Long
        || x instanceof BigInteger) {
      return NumUtils.normalizeInteger(NumUtils.toBigInteger(x));
    }
    return (Number) x;
  }

  static NumOps ops(Object x) {
    Class<? extends Object> xc = x.getClass();

    if (xc == Long.class || xc == Integer.class || xc == Short.class || xc == Byte.class)
      return NumUtils.LONG_OPS;
    else if (xc == Double.class || xc == Float.class) return NumUtils.DOUBLE_OPS;
    else if (xc == BigInteger.class) return NumUtils.BIGINT_OPS;
    else return NumUtils.BIGINT_OPS;
  }

  public static Number or(long x, long y) {
    return num(x | y);
  }

  public static Number or(long x, Object y) {
    return or((Object) x, y);
  }

  public static Number or(Object x, long y) {
    return or(x, (Object) y);
  }

  public static Number or(Object x, Object y) {
    return NumUtils.normalizeInteger(NumUtils.bitOpsCast(x).or(NumUtils.bitOpsCast(y)));
  }

  public static double quotient(double n, double d) {
    if (d == 0) throw new ArithmeticException("Divide by zero");

    double q = n / d;
    if (q <= Long.MAX_VALUE && q >= Long.MIN_VALUE) {
      return (long) q;
    } else {
      return NumUtils.requireFinite(q);
    }
  }

  public static double quotient(double x, long y) {
    return quotient(x, (double) y);
  }

  public static Number quotient(double x, Object y) {
    return quotient((Object) x, y);
  }

  public static double quotient(long x, double y) {
    return quotient((double) x, y);
  }

  public static Number quotient(long x, long y) {
    if (y == 0) throw new ArithmeticException("Divide by zero");
    if (x == Long.MIN_VALUE && y == -1) return BigInteger.valueOf(x).negate();
    return num(x / y);
  }

  public static Number quotient(long x, Object y) {
    return quotient((Object) x, y);
  }

  public static Number quotient(Object x, double y) {
    return quotient(x, (Object) y);
  }

  public static Number quotient(Object x, long y) {
    return quotient(x, (Object) y);
  }

  public static Number quotient(Object x, Object y) {
    NumOps yops = ops(y);
    if (yops.isZero((Number) y)) throw new ArithmeticException("Divide by zero");
    return ops(x).combine(yops).quotient((Number) x, (Number) y);
  }

  // @WarnBoxedMath(false)
  public static double remainder(double n, double d) {
    if (d == 0) throw new ArithmeticException("Divide by zero");

    double q = n / d;
    if (q <= Long.MAX_VALUE && q >= Long.MIN_VALUE) {
      return NumUtils.requireFinite(n - ((long) q) * d);
    } else {
      return NumUtils.requireFinite(n - ((long) q) * d);
    }
  }

  public static double remainder(double x, long y) {
    return remainder(x, (double) y);
  }

  public static Number remainder(double x, Object y) {
    return remainder((Object) x, y);
  }

  public static double remainder(long x, double y) {
    return remainder((double) x, y);
  }

  public static long remainder(long x, long y) {
    if (y == 0) throw new ArithmeticException("Divide by zero");
    if (x == Long.MIN_VALUE && y == -1) return 0;
    return x % y;
  }

  public static Number remainder(long x, Object y) {
    return remainder((Object) x, y);
  }

  public static Number remainder(Object x, double y) {
    return remainder(x, (Object) y);
  }

  public static Number remainder(Object x, long y) {
    return remainder(x, (Object) y);
  }

  public static Number remainder(Object x, Object y) {
    NumOps yops = ops(y);
    if (yops.isZero((Number) y)) throw new ArithmeticException("Divide by zero");
    return ops(x).combine(yops).remainder((Number) x, (Number) y);
  }

  public static Number setBit(long x, long n) {
    return setBit((Object) x, (Object) n);
  }

  public static Number setBit(long x, Object y) {
    return setBit((Object) x, y);
  }

  public static Number setBit(Object x, long y) {
    return setBit(x, (Object) y);
  }

  public static Number setBit(Object x, Object y) {
    return NumUtils.normalizeInteger(NumUtils.bitOpsCast(x).setBit(NumUtils.bitIndex(y)));
  }

  public static Number shiftLeft(long x, long n) {
    return shiftLeft((Object) x, (Object) n);
  }

  public static Number shiftLeft(long x, Object y) {
    return shiftLeft((Object) x, y);
  }

  public static Number shiftLeft(Object x, long y) {
    return shiftLeft(x, (Object) y);
  }

  public static Number shiftLeft(Object x, Object y) {
    return NumUtils.normalizeInteger(NumUtils.bitOpsCast(x).shiftLeft(NumUtils.bitIndex(y)));
  }

  public static int shiftLeftInt(int x, int n) {
    return x << n;
  }

  public static Number shiftRight(long x, long n) {
    return shiftRight((Object) x, (Object) n);
  }

  public static Number shiftRight(long x, Object y) {
    return shiftRight((Object) x, y);
  }

  public static Number shiftRight(Object x, long y) {
    return shiftRight(x, (Object) y);
  }

  public static Number shiftRight(Object x, Object y) {
    return NumUtils.normalizeInteger(NumUtils.bitOpsCast(x).shiftRight(NumUtils.bitIndex(y)));
  }

  public static int shiftRightInt(int x, int n) {
    return x >> n;
  }

  // @WarnBoxedMath(false)
  public static short[] shorts(Object array) {
    return (short[]) array;
  }

  public static boolean testBit(long x, long n) {
    return testBit((Object) x, (Object) n);
  }

  public static boolean testBit(long x, Object y) {
    return testBit((Object) x, y);
  }

  public static boolean testBit(Object x, long y) {
    return testBit(x, (Object) y);
  }

  public static boolean testBit(Object x, Object y) {
    return NumUtils.bitOpsCast(x).testBit(NumUtils.bitIndex(y));
  }

  public static int throwIntOverflow() {
    throw new ArithmeticException("integer overflow");
  }

  public static double unchecked_add(double x, double y) {
    return add(x, y);
  }

  public static double unchecked_add(double x, long y) {
    return add(x, y);
  }

  public static double unchecked_add(double x, Object y) {
    return add(x, y);
  }

  public static double unchecked_add(long x, double y) {
    return add(x, y);
  }

  public static long unchecked_add(long x, long y) {
    return x + y;
  }

  public static Number unchecked_add(long x, Object y) {
    return unchecked_add((Object) x, y);
  }

  public static double unchecked_add(Object x, double y) {
    return add(x, y);
  }

  public static Number unchecked_add(Object x, long y) {
    return unchecked_add(x, (Object) y);
  }

  public static Number unchecked_add(Object x, Object y) {
    return ops(x).combine(ops(y)).unchecked_add((Number) x, (Number) y);
  }

  public static double unchecked_dec(double x) {
    return dec(x);
  }

  public static long unchecked_dec(long x) {
    return x - 1;
  }

  public static Number unchecked_dec(Object x) {
    return ops(x).unchecked_dec((Number) x);
  }

  public static double unchecked_inc(double x) {
    return inc(x);
  }

  public static long unchecked_inc(long x) {
    return x + 1;
  }

  public static Number unchecked_inc(Object x) {
    return ops(x).unchecked_inc((Number) x);
  }

  public static int unchecked_int_add(int x, int y) {
    return x + y;
  }

  public static int unchecked_int_dec(int x) {
    return x - 1;
  }

  public static int unchecked_int_divide(int x, int y) {
    return x / y;
  }

  public static int unchecked_int_inc(int x) {
    return x + 1;
  }

  public static int unchecked_int_multiply(int x, int y) {
    return x * y;
  }

  public static int unchecked_int_negate(int x) {
    return -x;
  }

  public static int unchecked_int_remainder(int x, int y) {
    return x % y;
  }

  public static int unchecked_int_subtract(int x, int y) {
    return x - y;
  }

  public static double unchecked_minus(double x) {
    return minus(x);
  }

  public static double unchecked_minus(double x, double y) {
    return minus(x, y);
  }

  public static double unchecked_minus(double x, long y) {
    return minus(x, y);
  }

  public static double unchecked_minus(double x, Object y) {
    return minus(x, y);
  }

  public static long unchecked_minus(long x) {
    return -x;
  }

  public static double unchecked_minus(long x, double y) {
    return minus(x, y);
  }

  public static long unchecked_minus(long x, long y) {
    return x - y;
  }

  public static Number unchecked_minus(long x, Object y) {
    return unchecked_minus((Object) x, y);
  }

  public static Number unchecked_minus(Object x) {
    return ops(x).unchecked_negate((Number) x);
  }

  public static double unchecked_minus(Object x, double y) {
    return minus(x, y);
  }

  public static Number unchecked_minus(Object x, long y) {
    return unchecked_minus(x, (Object) y);
  }

  public static Number unchecked_minus(Object x, Object y) {
    NumOps yops = ops(y);
    return ops(x).combine(yops).unchecked_add((Number) x, yops.unchecked_negate((Number) y));
  }

  public static double unchecked_multiply(double x, double y) {
    return multiply(x, y);
  }

  public static double unchecked_multiply(double x, long y) {
    return multiply(x, y);
  }

  public static double unchecked_multiply(double x, Object y) {
    return multiply(x, y);
  }

  public static double unchecked_multiply(long x, double y) {
    return multiply(x, y);
  }

  public static long unchecked_multiply(long x, long y) {
    return x * y;
  }

  public static Number unchecked_multiply(long x, Object y) {
    return unchecked_multiply((Object) x, y);
  }

  public static double unchecked_multiply(Object x, double y) {
    return multiply(x, y);
  }

  public static Number unchecked_multiply(Object x, long y) {
    return unchecked_multiply(x, (Object) y);
  }

  public static Number unchecked_multiply(Object x, Object y) {
    return ops(x).combine(ops(y)).unchecked_multiply((Number) x, (Number) y);
  }

  public static Number unsignedShiftRight(long x, long n) {
    return shiftRight(x, n);
  }

  public static Number unsignedShiftRight(long x, Object y) {
    return shiftRight(x, y);
  }

  public static Number unsignedShiftRight(Object x, long y) {
    return shiftRight(x, y);
  }

  public static Number unsignedShiftRight(Object x, Object y) {
    return shiftRight(x, y);
  }

  public static int unsignedShiftRightInt(int x, int n) {
    return x >>> n;
  }

  public static Number xor(long x, long y) {
    return num(x ^ y);
  }

  public static Number xor(long x, Object y) {
    return xor((Object) x, y);
  }

  public static Number xor(Object x, long y) {
    return xor(x, (Object) y);
  }

  public static Number xor(Object x, Object y) {
    return NumUtils.normalizeInteger(NumUtils.bitOpsCast(x).xor(NumUtils.bitOpsCast(y)));
  }
}
