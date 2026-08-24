package hara.truffle;

import hara.kernel.builtin.BuiltinStruct;
import hara.lang.data.Keyword;
import hara.lang.protocol.ILinearType;
import hara.lang.protocol.IMapType;

import java.math.BigInteger;
import java.util.ArrayList;
import java.util.Collections;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Objects;

/** The deliberately small semantic model used by the strict JSON runtime library. */
abstract class JsonValue {
  private JsonValue() {}

  public static final class Null extends JsonValue {
    public static final Null INSTANCE = new Null();

    private Null() {}

    @Override
    public boolean equals(java.lang.Object other) {
      return other instanceof Null;
    }

    @Override
    public int hashCode() {
      return 0;
    }
  }

  public static final class Bool extends JsonValue {
    private final boolean value;

    public Bool(boolean value) {
      this.value = value;
    }

    public boolean value() {
      return value;
    }

    @Override
    public boolean equals(java.lang.Object other) {
      return other instanceof Bool && value == ((Bool) other).value;
    }

    @Override
    public int hashCode() {
      return Boolean.hashCode(value);
    }
  }

  public static final class Integer extends JsonValue {
    private final long value;

    public Integer(long value) {
      this.value = value;
    }

    public long value() {
      return value;
    }

    @Override
    public boolean equals(java.lang.Object other) {
      return other instanceof Integer && value == ((Integer) other).value;
    }

    @Override
    public int hashCode() {
      return Long.hashCode(value);
    }
  }

  public static final class BigIntegerValue extends JsonValue {
    private final BigInteger value;

    public BigIntegerValue(BigInteger value) {
      this.value = value;
    }

    public BigInteger value() {
      return value;
    }

    @Override
    public boolean equals(java.lang.Object other) {
      return other instanceof BigIntegerValue && value.equals(((BigIntegerValue) other).value);
    }

    @Override
    public int hashCode() {
      return value.hashCode();
    }
  }

  public static final class String extends JsonValue {
    private final java.lang.String value;

    public String(java.lang.String value) {
      requireValidUnicode(value);
      this.value = value;
    }

    public java.lang.String value() {
      return value;
    }

    @Override
    public boolean equals(java.lang.Object other) {
      return other instanceof String && value.equals(((String) other).value);
    }

    @Override
    public int hashCode() {
      return value.hashCode();
    }
  }

  public static final class Array extends JsonValue {
    private final List<JsonValue> values;

    public Array(List<JsonValue> values) {
      this.values = Collections.unmodifiableList(new ArrayList<>(values));
    }

    public List<JsonValue> values() {
      return values;
    }

    @Override
    public boolean equals(java.lang.Object other) {
      return other instanceof Array && values.equals(((Array) other).values);
    }

    @Override
    public int hashCode() {
      return values.hashCode();
    }
  }

  public static final class Object extends JsonValue {
    private final Map<java.lang.String, JsonValue> values;

    public Object(Map<java.lang.String, JsonValue> values) {
      this.values = Collections.unmodifiableMap(new LinkedHashMap<>(values));
    }

    public Map<java.lang.String, JsonValue> values() {
      return values;
    }

    @Override
    public boolean equals(java.lang.Object other) {
      return other instanceof Object && values.equals(((Object) other).values);
    }

    @Override
    public int hashCode() {
      return values.hashCode();
    }
  }

  public static JsonValue fromHara(java.lang.Object value) {
    return fromHara(value, false);
  }

  public static JsonValue fromHara(java.lang.Object value, boolean normalizeKeywords) {
    value = HaraBox.unwrap(value);
    if (value == null) return Null.INSTANCE;
    if (value instanceof Boolean) return new Bool((Boolean) value);
    if (value instanceof Byte
        || value instanceof Short
        || value instanceof java.lang.Integer
        || value instanceof Long) {
      return new Integer(((Number) value).longValue());
    }
    if (value instanceof BigInteger) {
      BigInteger integer = (BigInteger) value;
      try {
        return new Integer(integer.longValueExact());
      } catch (ArithmeticException e) {
        return new BigIntegerValue(integer);
      }
    }
    if (value instanceof java.lang.String) return new String((java.lang.String) value);
    if (normalizeKeywords && value instanceof Keyword) {
      return new String(keywordName((Keyword) value));
    }
    if (value instanceof ILinearType<?>) {
      var values = new ArrayList<JsonValue>();
      for (var item : (ILinearType<?>) value) values.add(fromHara(item, normalizeKeywords));
      return new Array(values);
    }
    if (value instanceof java.util.List<?>) {
      var values = new ArrayList<JsonValue>();
      for (var item : (java.util.List<?>) value) values.add(fromHara(item, normalizeKeywords));
      return new Array(values);
    }
    if (value instanceof IMapType<?, ?>) {
      var values = new LinkedHashMap<java.lang.String, JsonValue>();
      for (var entry : (IMapType<?, ?>) value) {
        put(values, key(entry.getKey(), normalizeKeywords), entry.getValue(), normalizeKeywords);
      }
      return new Object(values);
    }
    if (value instanceof Map<?, ?>) {
      var values = new LinkedHashMap<java.lang.String, JsonValue>();
      for (var entry : ((Map<?, ?>) value).entrySet()) {
        put(values, key(entry.getKey(), normalizeKeywords), entry.getValue(), normalizeKeywords);
      }
      return new Object(values);
    }
    if (value instanceof Float || value instanceof Double) {
      throw new IllegalArgumentException(
          "Provable JSON v1 supports signed 64-bit integers, not fractional numbers.");
    }
    throw new IllegalArgumentException(
        "Value is not representable as provable JSON v1: " + value.getClass().getName());
  }

  public java.lang.Object toHara() {
    if (this instanceof Null) return null;
    if (this instanceof Bool) return ((Bool) this).value();
    if (this instanceof Integer) return ((Integer) this).value();
    if (this instanceof BigIntegerValue) return ((BigIntegerValue) this).value();
    if (this instanceof String) return ((String) this).value();
    if (this instanceof Array) {
      var values = new ArrayList<java.lang.Object>();
      for (var value : ((Array) this).values()) values.add(value.toHara());
      return BuiltinStruct.vector(values);
    }
    var values = new ArrayList<java.lang.Object>();
    for (var entry : ((Object) this).values().entrySet()) {
      values.add(entry.getKey());
      values.add(entry.getValue().toHara());
    }
    return BuiltinStruct.orderedMap(values);
  }

  private static void put(
      Map<java.lang.String, JsonValue> values,
      java.lang.String key,
      java.lang.Object value,
      boolean normalizeKeywords) {
    if (values.containsKey(key)) {
      throw new IllegalArgumentException("Duplicate JSON object key: " + key);
    }
    values.put(key, fromHara(value, normalizeKeywords));
  }

  private static java.lang.String key(java.lang.Object key, boolean normalizeKeywords) {
    if (key instanceof java.lang.String) {
      requireValidUnicode((java.lang.String) key);
      return (java.lang.String) key;
    }
    if (normalizeKeywords && key instanceof Keyword) return keywordName((Keyword) key);
    throw new IllegalArgumentException("JSON object keys must be strings.");
  }

  private static java.lang.String keywordName(Keyword keyword) {
    return keyword.getNamespace() == null
        ? keyword.getName()
        : keyword.getNamespace() + "/" + keyword.getName();
  }

  static void requireValidUnicode(java.lang.String value) {
    Objects.requireNonNull(value, "JSON strings cannot be null.");
    for (int index = 0; index < value.length(); index++) {
      char current = value.charAt(index);
      if (Character.isHighSurrogate(current)) {
        if (++index >= value.length() || !Character.isLowSurrogate(value.charAt(index))) {
          throw new IllegalArgumentException("JSON strings cannot contain unpaired surrogates.");
        }
      } else if (Character.isLowSurrogate(current)) {
        throw new IllegalArgumentException("JSON strings cannot contain unpaired surrogates.");
      }
    }
  }
}
