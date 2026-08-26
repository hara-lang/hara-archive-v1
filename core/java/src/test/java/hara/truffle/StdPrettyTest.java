package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertThrows;

import org.graalvm.polyglot.Context;
import org.junit.Test;

public class StdPrettyTest {
  @Test
  public void portableRendererGroupsAndBreaksDocuments() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      context.eval(HaraLanguage.ID, "(require 'std.foundation.pretty)");
      assertEquals(
          "[:document/annotate :pretty/string \"x\"]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(pr-str (std.native.Document/annotate :pretty/string \"x\"))")
              .asString());
      assertEquals(
          "abc",
          context.eval(HaraLanguage.ID, "(std.foundation.pretty/render \"abc\")").asString());
      assertEquals(
          "a😀",
          context.eval(HaraLanguage.ID, "(std.native.Document/text \"a\" \\😀)").asString());
      assertEquals(
          "a b",
          context
              .eval(
                  HaraLanguage.ID,
                  "(std.foundation.pretty/render [:group \"a\" [:line] \"b\"] {:width 80})")
              .asString());
      assertEquals(
          "\n  a",
          context
              .eval(
                  HaraLanguage.ID,
                  "(std.foundation.pretty/render [:nest 2 [:line] \"a\"] {:width 80})")
              .asString());
      String document =
          "[:group \"(\" [:nest 2 [:line] \"alpha\" [:line] \"beta\"] \")\"]";
      assertEquals(
          "( alpha beta)",
          context
              .eval(
                  HaraLanguage.ID,
                  "(std.foundation.pretty/render " + document + " {:width 80})")
              .asString());
      assertEquals(
          "(\n  alpha\n  beta)",
          context
              .eval(
                  HaraLanguage.ID,
                  "(std.foundation.pretty/render " + document + " {:width 8})")
              .asString());
      assertEquals(
          "{:a 1, :b 2}",
          context
              .eval(HaraLanguage.ID, "(pretty/pprint-str {:b 2 :a 1})")
              .asString());
      assertThrows(
          RuntimeException.class,
          () ->
              context.eval(
                  HaraLanguage.ID, "(std.foundation.pretty/render \"abc\" nil)"));
      assertThrows(
          RuntimeException.class,
          () -> context.eval(HaraLanguage.ID, "(require 'std.pretty)"));
      assertThrows(
          RuntimeException.class,
          () -> context.eval(HaraLanguage.ID, "(require 'std.foundation.pretty.engine)"));
    }
  }
}
