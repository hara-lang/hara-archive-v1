package hara.lang.base;

import hara.lang.base.primitive.Num;

import java.math.BigDecimal;
import java.math.BigInteger;

public interface NumOps {
  public Number add(Number x, Number y);

  public Number addP(Number x, Number y);

  NumOps combine(NumOps y);

  public Number dec(Number x);

  public Number decP(Number x);

  public Number divide(Number x, Number y);

  public boolean eq(Number x, Number y);

  public boolean gte(Number x, Number y);

  public Number inc(Number x);

  public Number incP(Number x);

  public boolean isNeg(Number x);

  public boolean isPos(Number x);

  public boolean isZero(Number x);

  public boolean lt(Number x, Number y);

  public boolean lte(Number x, Number y);

  public Number multiply(Number x, Number y);

  public Number multiplyP(Number x, Number y);

  public Number negate(Number x);

  public Number negateP(Number x);

  NumOps opsWith(BigIntegerOps x);

  NumOps opsWith(DoubleOps x);

  NumOps opsWith(LongOps x);

  public Number quotient(Number x, Number y);

  public Number remainder(Number x, Number y);

  public Number unchecked_add(Number x, Number y);

  public Number unchecked_dec(Number x);

  public Number unchecked_inc(Number x);

  public Number unchecked_multiply(Number x, Number y);

  public Number unchecked_negate(Number x);

  public abstract static class BaseOps implements NumOps {
    @Override
    public Number addP(Number x, Number y) {
      return add(x, y);
    }

    @Override
    public Number decP(Number x) {
      return dec(x);
    }

    @Override
    public Number incP(Number x) {
      return inc(x);
    }

    @Override
    public Number multiplyP(Number x, Number y) {
      return multiply(x, y);
    }

    @Override
    public Number negateP(Number x) {
      return negate(x);
    }

    @Override
    public Number unchecked_add(Number x, Number y) {
      return add(x, y);
    }

    @Override
    public Number unchecked_dec(Number x) {
      return dec(x);
    }

    @Override
    public Number unchecked_inc(Number x) {
      return inc(x);
    }

    @Override
    public Number unchecked_multiply(Number x, Number y) {
      return multiply(x, y);
    }

    @Override
    public Number unchecked_negate(Number x) {
      return negate(x);
    }
  }

  public static class BigIntegerOps extends BaseOps {
    @Override
    public final Number add(Number x, Number y) {
      return NumUtils.normalizeInteger(
          NumUtils.toBigInteger(x).add(NumUtils.toBigInteger(y)));
    }

    @Override
    public NumOps combine(NumOps y) {
      return y.opsWith(this);
    }

    @Override
    public Number dec(Number x) {
      BigInteger bx = NumUtils.toBigInteger(x);
      return NumUtils.normalizeInteger(bx.subtract(BigInteger.ONE));
    }

    @Override
    public Number divide(Number x, Number y) {
      return Num.divide(NumUtils.toBigInteger(x), NumUtils.toBigInteger(y));
    }

    @Override
    public boolean eq(Number x, Number y) {
      return NumUtils.toBigInteger(x).equals(NumUtils.toBigInteger(y));
    }

    @Override
    public boolean gte(Number x, Number y) {
      return NumUtils.toBigInteger(x).compareTo(NumUtils.toBigInteger(y)) >= 0;
    }

    @Override
    public Number inc(Number x) {
      BigInteger bx = NumUtils.toBigInteger(x);
      return NumUtils.normalizeInteger(bx.add(BigInteger.ONE));
    }

    @Override
    public boolean isNeg(Number x) {
      BigInteger bx = NumUtils.toBigInteger(x);
      return bx.signum() < 0;
    }

    @Override
    public boolean isPos(Number x) {
      BigInteger bx = NumUtils.toBigInteger(x);
      return bx.signum() > 0;
    }

    @Override
    public boolean isZero(Number x) {
      BigInteger bx = NumUtils.toBigInteger(x);
      return bx.signum() == 0;
    }

    @Override
    public boolean lt(Number x, Number y) {
      return NumUtils.toBigInteger(x).compareTo(NumUtils.toBigInteger(y)) == -1;
    }

    @Override
    public boolean lte(Number x, Number y) {
      return NumUtils.toBigInteger(x).compareTo(NumUtils.toBigInteger(y)) <= 0;
    }

    @Override
    public final Number multiply(Number x, Number y) {
      return NumUtils.normalizeInteger(
          NumUtils.toBigInteger(x).multiply(NumUtils.toBigInteger(y)));
    }

    @Override
    public final Number negate(Number x) {
      return NumUtils.normalizeInteger(NumUtils.toBigInteger(x).negate());
    }

    @Override
    public final NumOps opsWith(BigIntegerOps x) {
      return this;
    }

    @Override
    public final NumOps opsWith(DoubleOps x) {
      return NumUtils.DOUBLE_OPS;
    }

    @Override
    public final NumOps opsWith(LongOps x) {
      return this;
    }

    @Override
    public Number quotient(Number x, Number y) {
      return NumUtils.normalizeInteger(
          NumUtils.toBigInteger(x).divide(NumUtils.toBigInteger(y)));
    }

    @Override
    public Number remainder(Number x, Number y) {
      return NumUtils.normalizeInteger(
          NumUtils.toBigInteger(x).remainder(NumUtils.toBigInteger(y)));
    }
  }

  public static class DoubleOps extends BaseOps {
    private static int compareIntegerToDouble(BigInteger integer, double floating) {
    NumUtils.requireFinite(floating);
    return new BigDecimal(integer).compareTo(BigDecimal.valueOf(floating));
  }

  private static boolean isFloating(Number value) {
    return value instanceof Double || value instanceof Float;
  }

  private static int compareExactly(Number x, Number y) {
    boolean xFloat = isFloating(x);
    boolean yFloat = isFloating(y);
    if (xFloat && yFloat) {
      double left = x.doubleValue();
      double right = y.doubleValue();
      if (left == 0.0d && right == 0.0d) return 0;
      return Double.compare(left, right);
    }
    if (xFloat) {
      return -compareIntegerToDouble(NumUtils.toBigInteger(y), x.doubleValue());
    }
    if (yFloat) {
      return compareIntegerToDouble(NumUtils.toBigInteger(x), y.doubleValue());
    }
    return NumUtils.toBigInteger(x).compareTo(NumUtils.toBigInteger(y));
  }

  private static boolean ordered(Number x, Number y) {
      NumUtils.requireFinite(x.doubleValue());
      NumUtils.requireFinite(y.doubleValue());
      return true;
    }

    @Override
    public final Number add(Number x, Number y) {
      return Double.valueOf(NumUtils.requireFinite(x.doubleValue() + y.doubleValue()));
    }

    @Override
    public NumOps combine(NumOps y) {
      return y.opsWith(this);
    }

    @Override
    public Number dec(Number x) {
      return Double.valueOf(NumUtils.requireFinite(x.doubleValue() - 1));
    }

    @Override
    public Number divide(Number x, Number y) {
      return Double.valueOf(NumUtils.requireFinite(x.doubleValue() / y.doubleValue()));
    }

    @Override
    public boolean eq(Number x, Number y) {
      double left = x.doubleValue();
      double right = y.doubleValue();
      return !Double.isNaN(left) && !Double.isNaN(right) && compareExactly(x, y) == 0;
    }

    @Override
    public boolean gte(Number x, Number y) {
      return ordered(x, y) && compareExactly(x, y) >= 0;
    }

    @Override
    public Number inc(Number x) {
      return Double.valueOf(NumUtils.requireFinite(x.doubleValue() + 1));
    }

    @Override
    public boolean isNeg(Number x) {
      return x.doubleValue() < 0;
    }

    @Override
    public boolean isPos(Number x) {
      return x.doubleValue() > 0;
    }

    @Override
    public boolean isZero(Number x) {
      return x.doubleValue() == 0;
    }

    @Override
    public boolean lt(Number x, Number y) {
      return ordered(x, y) && compareExactly(x, y) < 0;
    }

    @Override
    public boolean lte(Number x, Number y) {
      return ordered(x, y) && compareExactly(x, y) <= 0;
    }

    @Override
    public final Number multiply(Number x, Number y) {
      return Double.valueOf(NumUtils.requireFinite(x.doubleValue() * y.doubleValue()));
    }

    @Override
    public final Number negate(Number x) {
      return Double.valueOf(NumUtils.requireFinite(-x.doubleValue()));
    }

    @Override
    public final NumOps opsWith(BigIntegerOps x) {
      return this;
    }

    @Override
    public final NumOps opsWith(DoubleOps x) {
      return this;
    }

    @Override
    public final NumOps opsWith(LongOps x) {
      return this;
    }

    @Override
    public Number quotient(Number x, Number y) {
      return Num.quotient(x.doubleValue(), y.doubleValue());
    }

    @Override
    public Number remainder(Number x, Number y) {
      return Num.remainder(x.doubleValue(), y.doubleValue());
    }
  }

  public static class LongOps implements NumOps {
    @Override
    public final Number add(Number x, Number y) {
      return Num.num(NumUtils.add(x.longValue(), y.longValue()));
    }

    @Override
    public final Number addP(Number x, Number y) {
      long lx = x.longValue(), ly = y.longValue();
      long ret = lx + ly;
      if ((ret ^ lx) < 0 && (ret ^ ly) < 0) return NumUtils.BIGINT_OPS.add(x, y);
      return Num.num(ret);
    }

    @Override
    public NumOps combine(NumOps y) {
      return y.opsWith(this);
    }

    @Override
    public Number dec(Number x) {
      long val = x.longValue();
      return Num.num(NumUtils.dec(val));
    }

    @Override
    public Number decP(Number x) {
      long val = x.longValue();
      if (val > Long.MIN_VALUE) return Num.num(val - 1);
      return NumUtils.BIGINT_OPS.dec(x);
    }

    @Override
    public Number divide(Number x, Number y) {
      long n = x.longValue();
      long d = y.longValue();
      if (d == 0) throw new ArithmeticException("Divide by zero");
      if (n == Long.MIN_VALUE && d == -1) {
        return BigInteger.valueOf(n).negate();
      }
      return Num.num(n / d);
    }

    @Override
    public boolean eq(Number x, Number y) {
      return x.longValue() == y.longValue();
    }

    @Override
    public boolean gte(Number x, Number y) {
      return x.longValue() >= y.longValue();
    }

    @Override
    public Number inc(Number x) {
      long val = x.longValue();
      return Num.num(NumUtils.inc(val));
    }

    @Override
    public Number incP(Number x) {
      long val = x.longValue();
      if (val < Long.MAX_VALUE) return Num.num(val + 1);
      return NumUtils.BIGINT_OPS.inc(x);
    }

    @Override
    public boolean isNeg(Number x) {
      return x.longValue() < 0;
    }

    @Override
    public boolean isPos(Number x) {
      return x.longValue() > 0;
    }

    @Override
    public boolean isZero(Number x) {
      return x.longValue() == 0;
    }

    @Override
    public boolean lt(Number x, Number y) {
      return x.longValue() < y.longValue();
    }

    @Override
    public boolean lte(Number x, Number y) {
      return x.longValue() <= y.longValue();
    }

    @Override
    public final Number multiply(Number x, Number y) {
      return Num.num(NumUtils.multiply(x.longValue(), y.longValue()));
    }

    @Override
    public final Number multiplyP(Number x, Number y) {
      long lx = x.longValue(), ly = y.longValue();
      if (lx == Long.MIN_VALUE && ly < 0) return NumUtils.BIGINT_OPS.multiply(x, y);
      long ret = lx * ly;
      if (ly != 0 && ret / ly != lx) return NumUtils.BIGINT_OPS.multiply(x, y);
      return Num.num(ret);
    }

    @Override
    public final Number negate(Number x) {
      long val = x.longValue();
      return Num.num(NumUtils.minus(val));
    }

    @Override
    public final Number negateP(Number x) {
      long val = x.longValue();
      if (val > Long.MIN_VALUE) return Num.num(-val);
      return BigInteger.valueOf(val).negate();
    }

    @Override
    public final NumOps opsWith(BigIntegerOps x) {
      return NumUtils.BIGINT_OPS;
    }

    @Override
    public final NumOps opsWith(DoubleOps x) {
      return NumUtils.DOUBLE_OPS;
    }

    @Override
    public final NumOps opsWith(LongOps x) {
      return this;
    }

    @Override
    public Number quotient(Number x, Number y) {
      long numerator = x.longValue();
      long denominator = y.longValue();
      if (denominator == 0) throw new ArithmeticException("Divide by zero");
      if (numerator == Long.MIN_VALUE && denominator == -1) {
        return BigInteger.valueOf(numerator).negate();
      }
      return Num.num(numerator / denominator);
    }

    @Override
    public Number remainder(Number x, Number y) {
      if (x.longValue() == Long.MIN_VALUE && y.longValue() == -1) return Num.num(0);
      return Num.num(x.longValue() % y.longValue());
    }

    @Override
    public final Number unchecked_add(Number x, Number y) {
      return Num.num(NumUtils.unchecked_add(x.longValue(), y.longValue()));
    }

    @Override
    public Number unchecked_dec(Number x) {
      long val = x.longValue();
      return Num.num(NumUtils.unchecked_dec(val));
    }

    @Override
    public Number unchecked_inc(Number x) {
      long val = x.longValue();
      return Num.num(NumUtils.unchecked_inc(val));
    }

    @Override
    public final Number unchecked_multiply(Number x, Number y) {
      return Num.num(NumUtils.unchecked_multiply(x.longValue(), y.longValue()));
    }

    @Override
    public final Number unchecked_negate(Number x) {
      long val = x.longValue();
      return Num.num(NumUtils.unchecked_minus(val));
    }
  }
}
