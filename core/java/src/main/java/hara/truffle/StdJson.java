package hara.truffle;

import hara.lang.protocol.IMapType;
import hara.lang.protocol.ILinearType;

/** Compact and indented encoders for the strict JSON v1 value model. */
final class StdJson {
  private StdJson() {}

  static Object read(String source) {
    return StrictJson.parse(source);
  }

  static String write(Object value) {
    requireStrictValue(value);
    StringBuilder out = new StringBuilder();
    append(out, JsonValue.fromHara(value), 0, false);
    return out.toString();
  }

  static String writePretty(Object value) {
    requireStrictValue(value);
    StringBuilder out = new StringBuilder();
    append(out, JsonValue.fromHara(value), 0, true);
    return out.toString();
  }

  private static void append(StringBuilder out, JsonValue value, int depth, boolean pretty) {
    if (value instanceof JsonValue.Null) out.append("null");
    else if (value instanceof JsonValue.Bool bool) out.append(bool.value());
    else if (value instanceof JsonValue.Integer integer) out.append(integer.value());
    else if (value instanceof JsonValue.BigIntegerValue integer) out.append(integer.value());
    else if (value instanceof JsonValue.String string) appendString(out, string.value());
    else if (value instanceof JsonValue.Array array) {
      out.append('[');
      for (int index = 0; index < array.values().size(); index++) {
        if (index > 0) out.append(',');
        if (pretty) newline(out, depth + 1);
        append(out, array.values().get(index), depth + 1, pretty);
      }
      if (pretty && !array.values().isEmpty()) newline(out, depth);
      out.append(']');
    } else if (value instanceof JsonValue.Object object) {
      out.append('{');
      int index = 0;
      for (var entry : object.values().entrySet()) {
        if (index++ > 0) out.append(',');
        if (pretty) newline(out, depth + 1);
        appendString(out, entry.getKey());
        out.append(pretty ? ": " : ":");
        append(out, entry.getValue(), depth + 1, pretty);
      }
      if (pretty && !object.values().isEmpty()) newline(out, depth);
      out.append('}');
    } else throw new IllegalArgumentException("Unsupported strict JSON value");
  }

  private static void newline(StringBuilder out, int depth) {
    out.append('\n');
    out.append("  ".repeat(depth));
  }

  private static void appendString(StringBuilder out, String value) {
    out.append('"');
    for (int index = 0; index < value.length(); index++) {
      char c = value.charAt(index);
      switch (c) {
        case '"' -> out.append("\\\"");
        case '\\' -> out.append("\\\\");
        case '\b' -> out.append("\\b");
        case '\f' -> out.append("\\f");
        case '\n' -> out.append("\\n");
        case '\r' -> out.append("\\r");
        case '\t' -> out.append("\\t");
        default -> {
          if (c < 0x20) out.append(String.format("\\u%04x", (int) c));
          else out.append(c);
        }
      }
    }
    out.append('"');
  }

  private static void requireStrictValue(Object value) {
    value = HaraBox.unwrap(value);
    if (value == null || value instanceof Boolean || value instanceof String) return;
    if (value instanceof Byte || value instanceof Short || value instanceof Integer || value instanceof Long) return;
    if (value instanceof java.math.BigInteger) return;
    if (value instanceof ILinearType<?> values) {
      for (Object item : values) requireStrictValue(item);
      return;
    }
    if (value instanceof IMapType<?, ?> map) {
      for (var entry : map) {
        if (!(entry.getKey() instanceof String)) {
          throw new IllegalArgumentException("JSON object keys must be strings.");
        }
        requireStrictValue(entry.getValue());
      }
      return;
    }
    throw new IllegalArgumentException(
        "JSON values must be nil, booleans, integers, strings, vectors, or string-key maps.");
  }
}
