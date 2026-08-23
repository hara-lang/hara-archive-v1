package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertThrows;
import static org.junit.Assert.assertTrue;

import hara.kernel.base.RT;
import org.graalvm.polyglot.Context;
import org.graalvm.polyglot.HostAccess;
import org.graalvm.polyglot.PolyglotException;
import org.graalvm.polyglot.Value;
import org.junit.Test;

public class HaraJvmFlavorParityTest {
  @Test
  public void reflectionAndStaticInteropMatchTheInterpreter() {
    RT.Instance<Object> interpreter = new RT.Instance<>(null, "jvm-parity-test");
    interpreter.eval(interpreter.readString(namespaceForm()));

    try (Context truffle = context()) {
      truffle.eval(HaraLanguage.ID, namespaceForm());
      String[] expressions = {
        "(hara.native.jvm.reflect/name String)",
        "(hara.native.jvm.reflect/instance? String (new String \"value\"))",
        "(String/valueOf 42)",
        "(. (new Point 3 4) x)"
      };
      for (String expression : expressions) {
        Object expected = interpreter.eval(interpreter.readString(expression));
        Value actual = truffle.eval(HaraLanguage.ID, expression);
        assertEquals(String.valueOf(expected), actual.toString());
      }

      Value fields = truffle.eval(HaraLanguage.ID, "(hara.native.jvm.reflect/fields Point)");
      assertTrue(arrayContains(fields, "x"));

      Value symbols = truffle.eval(HaraLanguage.ID, "(current-symbols)");
      assertTrue(arrayContains(symbols, "String/valueOf"));
      assertTrue(arrayContains(symbols, "hara.native.jvm.reflect/instance?"));
    }
  }

  @Test
  public void embeddedTruffleDoesNotInferDynamicJvmCapabilitiesFromHostAccess() {
    try (Context truffle = context()) {
      truffle.eval(HaraLanguage.ID, namespaceForm());
      PolyglotException classpath =
          assertThrows(
              PolyglotException.class,
              () -> truffle.eval(HaraLanguage.ID, "(hara.native.jvm.classpath/paths)"));
      assertTrue(classpath.getMessage().contains("JVM classpath capability is not granted"));

    }
  }

  private static Context context() {
    return Context.newBuilder(HaraLanguage.ID)
        .allowHostAccess(HostAccess.ALL)
        .allowHostClassLookup(name -> true)
        .build();
  }

  private static String namespaceForm() {
    return "(ns jvm-parity-test (:flavor :jvm [java.lang String] [java.awt Point]))";
  }

  private static boolean arrayContains(Value values, String expected) {
    for (long i = 0; i < values.getArraySize(); i++) {
      if (expected.equals(values.getArrayElement(i).asString())) return true;
    }
    return false;
  }
}
