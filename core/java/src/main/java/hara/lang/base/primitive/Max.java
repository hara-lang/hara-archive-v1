package hara.lang.base.primitive;

import hara.lang.base.NumUtils;

public interface Max {

  public static double max(double x, double y) {
    return NumUtils.requireFinite(Math.max(x, y));
  }

  public static Object max(double x, long y) {
    if (x > y) {
      return x;
    } else {
      return y;
    }
  }

  public static Object max(double x, Object y) {
    if (x > ((Number) y).doubleValue()) {
      return x;
    } else {
      return y;
    }
  }

  public static Object max(long x, double y) {
    if (x > y) {
      return x;
    } else {
      return y;
    }
  }

  public static long max(long x, long y) {
    if (x > y) {
      return x;
    } else {
      return y;
    }
  }

  public static Object max(long x, Object y) {
    if (Num.gt(x, y)) {
      return x;
    } else {
      return y;
    }
  }

  public static Object max(Object x, double y) {
    if (((Number) x).doubleValue() > y) {
      return x;
    } else {
      return y;
    }
  }

  public static Object max(Object x, long y) {
    if (Num.gt(x, y)) {
      return x;
    } else {
      return y;
    }
  }

  public static Object max(Object x, Object y) {
    if (Num.gt(x, y)) {
      return x;
    } else {
      return y;
    }
  }
}
