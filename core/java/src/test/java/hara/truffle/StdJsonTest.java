package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertThrows;

import org.graalvm.polyglot.Context;
import org.junit.Test;

public class StdJsonTest {
  @Test
  public void strictJsonReadsWritesAndPrettyPrints() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      assertEquals(
          "[nil true -2 \"x\" [3] {\"a\" 4}]",
          context
              .eval(HaraLanguage.ID, "(std.native.Json/read \"[null,true,-2,\\\"x\\\",[3],{\\\"a\\\":4}]\")")
              .toString());
      assertEquals(
          "{\"a\":1,\"b\":[true,null]}",
          context.eval(HaraLanguage.ID, "(std.native.Json/write {\"a\" 1 \"b\" [true nil]})").asString());
      assertEquals(
          "{\"a\":1}",
          context.eval(HaraLanguage.ID, "(Json/write {\"a\" 1})").asString());
      assertEquals(
          "{\n  \"a\": 1\n}",
          context.eval(HaraLanguage.ID, "(std.native.Json/pretty {\"a\" 1} {})").asString());
      assertThrows(
          RuntimeException.class,
          () -> context.eval(HaraLanguage.ID, "(std.native.Json/pretty {\"a\" 1} nil)"));
    }
  }

  @Test
  public void strictJsonRejectsUnsupportedFormsAndPrettyUsesReadablePrinter() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      assertThrows(
          RuntimeException.class,
          () -> context.eval(HaraLanguage.ID, "(std.native.Json/read \"1.5\")"));
      assertThrows(
          RuntimeException.class,
          () -> context.eval(HaraLanguage.ID, "(std.native.Json/write {:a 1})"));
      assertEquals(
          "{:a [1 2]}",
          context.eval(HaraLanguage.ID, "(pretty/pprint-str {:a [1 2]})").asString());
    }
  }

  @Test
  public void strictJsonRejectsDuplicateKeysInvalidNumbersAndExcessiveNesting() {
    assertThrows(
        IllegalArgumentException.class,
        () -> StrictJson.parseValue("{\"a\":1,\"a\":2}"));
    assertThrows(IllegalArgumentException.class, () -> StrictJson.parseValue("1e3"));
    assertEquals(
        new java.math.BigInteger("9223372036854775808"),
        ((JsonValue.BigIntegerValue) StrictJson.parseValue("9223372036854775808")).value());
    String nested = "[".repeat(StrictJson.MAX_DEPTH + 1) + "0" + "]".repeat(StrictJson.MAX_DEPTH + 1);
    assertThrows(IllegalArgumentException.class, () -> StrictJson.parseValue(nested));
  }
}
