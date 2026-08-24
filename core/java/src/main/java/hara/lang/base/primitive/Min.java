package hara.lang.base.primitive;

import hara.lang.base.NumUtils;

public interface Min {
  public static double min(double x, double y) {
    return NumUtils.requireFinite(Math.min(x, y));
  }

  public static Object min(double x, long y) {
    if (x < y) {
      return x;
    } else {
      return y;
    }
  }

  public static Object min(double x, Object y) {
    if (x < ((Number) y).doubleValue()) {
      return x;
    } else {
      return y;
    }
  }

  public static Object min(long x, double y) {
    if (x < y) {
      return x;
    } else {
      return y;
    }
  }

  public static long min(long x, long y) {
    if (x < y) {
      return x;
    } else {
      return y;
    }
  }

  public static Object min(long x, Object y) {
    if (Num.lt(x, y)) {
      return x;
    } else {
      return y;
    }
  }

  public static Object min(Object x, double y) {
    if (((Number) x).doubleValue() < y) {
      return x;
    } else {
      return y;
    }
  }

  public static Object min(Object x, long y) {
    if (Num.lt(x, y)) {
      return x;
    } else {
      return y;
    }
  }

  public static Object min(Object x, Object y) {
    if (Num.lt(x, y)) {
      return x;
    } else {
      return y;
    }
  }
}
