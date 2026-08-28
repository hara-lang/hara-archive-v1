package hara.kernel.builtin;

import hara.kernel.base.Module;
import hara.lang.base.NumUtils;

@SuppressWarnings({"unchecked", "rawtypes"})
@Module.Ns(name = "global", tag = "check")
public interface BuiltinCheck {

  @Module.Fn(name = "class?", complete = true)
  public static <TYPE> boolean isClass(TYPE x) {
    return (x instanceof Class);
  }

  @Module.Fn(name = "falsey?", complete = true)
  public static <TYPE> boolean isFalsey(TYPE x) {
    if (x == null) {
      return true;
    } else if (x instanceof Boolean) {
      return !((Boolean) x).booleanValue();
    } else {
      return false;
    }
  }

  @Module.Fn(name = "not", complete = true)
  public static boolean not(Object value) {
    return !isTruthy(value);
  }

  @Module.Fn(name = "long?", complete = true)
  public static <TYPE> boolean isLong(TYPE x) {
    return NumUtils.isLongValue(x);
  }

  @Module.Fn(name = "double?", complete = true)
  public static <TYPE> boolean isDouble(TYPE x) {
    return x instanceof Float || x instanceof Double;
  }

  @Module.Fn(name = "truthy?", complete = true)
  public static <TYPE> boolean isTruthy(TYPE x) {
    if (x == null) {
      return false;
    } else if (x instanceof Boolean) {
      return ((Boolean) x).booleanValue();
    } else {
      return true;
    }
  }

  //
  // Checks
  //

}
