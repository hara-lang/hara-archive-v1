package hara.lang.base;

import hara.lang.base.primitive.Array;
import hara.lang.data.HaraCharacter;
import hara.lang.protocol.Constant;
import hara.lang.protocol.IDisplay;
import hara.lang.protocol.IHash;

import java.math.BigDecimal;
import java.math.BigInteger;
import java.util.Iterator;
import java.util.Map.Entry;
import java.util.function.Function;
import java.util.regex.Pattern;

public interface G {

  /** Prints a regex without repeatedly escaping its backslashes. */
  public static String displayRegex(Pattern value) {
    String pattern = value.pattern();
    StringBuilder display = new StringBuilder("#\"");
    int backslashes = 0;
    for (int index = 0; index < pattern.length(); index++) {
      char character = pattern.charAt(index);
      if (character == '"' && backslashes % 2 == 0) display.append('\\');
      display.append(character);
      backslashes = character == '\\' ? backslashes + 1 : 0;
    }
    return display.append('"').toString();
  }

  public static class Exception extends RuntimeException {
    public Exception() {}

    public Exception(String msg) {
      super(msg);
    }
  }

  public static final Constant.HashType DEFAULT_HASH = Constant.HashType.RAPID;

  @SuppressWarnings({"rawtypes", "unchecked"})
  public static String displayList(java.util.List l) {
    return "#j" + Iter.display(l.iterator());
  }

  @SuppressWarnings({"rawtypes", "unchecked"})
  public static String displayMap(java.util.Map m) {
    return "#j"
        + Iter.toString(
            m.entrySet().iterator(),
            "{",
            "}",
            ",",
            (entry) ->
                display(((Entry) entry).getKey()) + " " + display(((Entry) entry).getValue()));
  }

  @SuppressWarnings("rawtypes")
  public static String displayMapEntry(Entry e) {
    return "[" + display(e.getKey()) + " " + display(e.getValue()) + "]";
  }

  public static String displayCharacter(Character value) {
    return displayCharacter((int) value.charValue());
  }

  public static String displayCharacter(int value) {
    switch (value) {
      case '\n':
        return "\\newline";
      case ' ':
        return "\\space";
      case '\t':
        return "\\tab";
      case '\b':
        return "\\backspace";
      case '\f':
        return "\\formfeed";
      case '\r':
        return "\\return";
      default:
        return Character.isISOControl(value)
            ? String.format("\\u%04X", value)
            : "\\" + new String(Character.toChars(value));
    }
  }

  public static String displayCharacter(Object value) {
    if (value instanceof HaraCharacter character) return displayCharacter(character.codePoint());
    if (value instanceof Character character) return displayCharacter(character);
    throw new IllegalArgumentException("expected a character");
  }


  public static String displayBytes(byte[] value) {
    StringBuilder display = new StringBuilder("(bytes");
    for (byte element : value) {
      display.append(' ').append(element);
    }
    return display.append(')').toString();
  }

  @SuppressWarnings({"rawtypes", "unchecked"})
  public static String display(Object e) {
    if (e == null) {
      return "nil";
    } else if (e instanceof IDisplay) {
      return ((IDisplay) e).display();
    } else if (e instanceof String) {
      return "\"" + Str.escapeJava((String) e) + "\"";
    } else if (e instanceof Character) {
      return displayCharacter((Character) e);
    } else if (e instanceof byte[]) {
      return displayBytes((byte[]) e);
    } else if (e instanceof Pattern) {
      return displayRegex((Pattern) e);
    } else if (e instanceof Double || e instanceof Float) {
      double value = ((Number) e).doubleValue();
      if (!Double.isFinite(value)) throw new IllegalArgumentException("non-finite number");
      return "(double " + Double.toString(value) + ")";
    } else if (e instanceof BigInteger) {
      return e.toString();
    } else if (e instanceof Class) {
      return ((Class) e).getName();
    } else if (e instanceof java.util.List) {
      return displayList((java.util.List) e);
    } else if (e instanceof java.util.Map) {
      return displayMap((java.util.Map) e);
    } else if (e instanceof Entry) {
      return displayMapEntry((Entry) e);
    } else if (e instanceof Iterator) {
      return "#i" + Iter.display((Iterator) e);
    } else if (e.getClass().isArray()) {
      return "#arr" + Iter.display(Array.toIter(e));
    } else {
      return e.toString();
    }
  }

  public static String getLineNumber() {
    var curr = Thread.currentThread().getStackTrace()[3];
    return curr.getClassName() + "/" + curr.getMethodName() + " - L" + curr.getLineNumber();
  }

  public static void prn(Object... arr) {
    System.out.println(getLineNumber() + ":\n" + Array.display(arr));
  }

  public static long hashRapid(Object o) {
    if (o instanceof IHash) {
      return ((IHash) o).hashGet(Constant.HashType.RAPID);
    } else if (o == null) {
      return 0;
    } else {
      return hashValue(o);
    }
  }

  public static long hashMurmur(Object o) {
    if (o instanceof IHash) {
      return ((IHash) o).hashGet(Constant.HashType.MURMUR3);
    } else if (o == null) {
      return 0;
    } else {
      return hashValue(o);
    }
  }

  public static long hashSip(Object o) {
    if (o == null) return 0;
    return hashValue(o);
  }

  public static Function<Object, Long> hashFn(Constant.HashType t) {

    switch (t) {
      case RAPID:
        return item -> Long.valueOf(hashRapid(item));
      case MURMUR3:
        return item -> Long.valueOf(hashMurmur(item));
      case SIP:
        return item -> Long.valueOf(hashSip(item));
      case SYSTEM:
        return item -> Long.valueOf(hashValue(item));
      default:
        throw new UnsupportedOperationException("Not Supported");
    }
  }

  public static long hashCalc(Constant.HashType t, Object o) {
    return hashFn(t).apply(o);
  }

  private static long hashValue(Object value) {
    if (value instanceof byte[]) return java.util.Arrays.hashCode((byte[]) value);
    if (!(value instanceof Number)) return value.hashCode();
    if (value instanceof Double || value instanceof Float) {
      double number = ((Number) value).doubleValue();
      if (number == 0.0d) return 0;
      if (!Double.isFinite(number)) throw new IllegalArgumentException("non-finite number");
      return BigDecimal.valueOf(number).stripTrailingZeros().hashCode();
    }
    if (value instanceof BigInteger) {
      return new BigDecimal((BigInteger) value).stripTrailingZeros().hashCode();
    }
    return BigDecimal.valueOf(((Number) value).longValue())
        .stripTrailingZeros()
        .hashCode();
  }
}
