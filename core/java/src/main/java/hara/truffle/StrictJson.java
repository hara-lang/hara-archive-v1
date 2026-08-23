package hara.truffle;

import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.math.BigInteger;

/** A dependency-free parser for the strict, proof-friendly JSON v1 profile. */
final class StrictJson {
  public static final int MAX_DEPTH = 256;

  private StrictJson() {}

  static JsonValue parseValue(String source) {
    if (source == null) throw new IllegalArgumentException("JSON source cannot be null.");
    var parser = new Parser(source);
    var value = parser.value(0);
    parser.whitespace();
    if (!parser.end()) parser.fail("Trailing content after the JSON value");
    return value;
  }

  static Object parse(String source) {
    return parseValue(source).toHara();
  }

  private static final class Parser {
    private final String source;
    private int offset;

    private Parser(String source) {
      this.source = source;
    }

    private JsonValue value(int depth) {
      if (depth > MAX_DEPTH) fail("JSON nesting exceeds " + MAX_DEPTH);
      whitespace();
      if (end()) fail("Expected a JSON value");
      switch (source.charAt(offset)) {
        case 'n':
          return literal("null", JsonValue.Null.INSTANCE);
        case 't':
          return literal("true", new JsonValue.Bool(true));
        case 'f':
          return literal("false", new JsonValue.Bool(false));
        case '"':
          return new JsonValue.String(string());
        case '[':
          return array(depth + 1);
        case '{':
          return object(depth + 1);
        default:
          return number();
      }
    }

    private JsonValue literal(String literal, JsonValue value) {
      if (!source.startsWith(literal, offset)) fail("Invalid JSON token");
      offset += literal.length();
      return value;
    }

    private JsonValue number() {
      int start = offset;
      if (peek('-')) offset++;
      if (end()) fail("Incomplete JSON number");
      if (peek('0')) {
        offset++;
        if (!end() && isDigit(source.charAt(offset))) fail("Leading zero in JSON number");
      } else {
        if (!isDigit19(source.charAt(offset))) fail("Expected a JSON value");
        while (!end() && isDigit(source.charAt(offset))) offset++;
      }
      if (!end() && (peek('.') || peek('e') || peek('E'))) {
        fail("Provable JSON v1 only supports integers");
      }
      try {
        BigInteger value = new BigInteger(source.substring(start, offset));
        try {
          return new JsonValue.Integer(value.longValueExact());
        } catch (ArithmeticException ignored) {
          return new JsonValue.BigIntegerValue(value);
        }
      } catch (NumberFormatException e) {
        fail("Invalid JSON integer");
        throw new AssertionError();
      }
    }

    private JsonValue array(int depth) {
      offset++;
      whitespace();
      var values = new ArrayList<JsonValue>();
      if (take(']')) return new JsonValue.Array(values);
      while (true) {
        values.add(value(depth));
        whitespace();
        if (take(']')) return new JsonValue.Array(values);
        expect(',');
        whitespace();
        if (peek(']')) fail("Trailing commas are not valid JSON");
      }
    }

    private JsonValue object(int depth) {
      offset++;
      whitespace();
      var values = new LinkedHashMap<String, JsonValue>();
      if (take('}')) return new JsonValue.Object(values);
      while (true) {
        if (!peek('"')) fail("JSON object keys must be strings");
        var key = string();
        whitespace();
        expect(':');
        var value = value(depth);
        if (values.putIfAbsent(key, value) != null) {
          fail("Duplicate JSON object key: " + key);
        }
        whitespace();
        if (take('}')) return new JsonValue.Object(values);
        expect(',');
        whitespace();
        if (peek('}')) fail("Trailing commas are not valid JSON");
      }
    }

    private String string() {
      expect('"');
      var value = new StringBuilder();
      while (!end()) {
        char current = source.charAt(offset++);
        if (current == '"') {
          var result = value.toString();
          JsonValue.requireValidUnicode(result);
          return result;
        }
        if (current < 0x20) fail("Unescaped control character in JSON string");
        if (current != '\\') {
          value.append(current);
          continue;
        }
        if (end()) fail("Incomplete JSON escape");
        switch (source.charAt(offset++)) {
          case '"':
            value.append('"');
            break;
          case '\\':
            value.append('\\');
            break;
          case '/':
            value.append('/');
            break;
          case 'b':
            value.append('\b');
            break;
          case 'f':
            value.append('\f');
            break;
          case 'n':
            value.append('\n');
            break;
          case 'r':
            value.append('\r');
            break;
          case 't':
            value.append('\t');
            break;
          case 'u':
            value.append(unicodeEscape());
            break;
          default:
            fail("Invalid JSON escape");
        }
      }
      fail("Unterminated JSON string");
      throw new AssertionError();
    }

    private char unicodeEscape() {
      if (offset + 4 > source.length()) fail("Incomplete Unicode escape");
      int value = 0;
      for (int index = 0; index < 4; index++) {
        int digit = Character.digit(source.charAt(offset++), 16);
        if (digit < 0) fail("Invalid Unicode escape");
        value = (value << 4) | digit;
      }
      return (char) value;
    }

    private void whitespace() {
      while (!end()) {
        char current = source.charAt(offset);
        if (current == ' ' || current == '\t' || current == '\n' || current == '\r') offset++;
        else break;
      }
    }

    private void expect(char expected) {
      if (!take(expected)) fail("Expected '" + expected + "'");
    }

    private boolean take(char expected) {
      if (peek(expected)) {
        offset++;
        return true;
      }
      return false;
    }

    private boolean peek(char expected) {
      return !end() && source.charAt(offset) == expected;
    }

    private boolean end() {
      return offset >= source.length();
    }

    private void fail(String message) {
      throw new IllegalArgumentException(message + " at character " + offset + ".");
    }

    private static boolean isDigit(char value) {
      return value >= '0' && value <= '9';
    }

    private static boolean isDigit19(char value) {
      return value >= '1' && value <= '9';
    }
  }
}
