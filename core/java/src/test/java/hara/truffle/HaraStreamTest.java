package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertThrows;
import static org.junit.Assert.assertTrue;

import org.graalvm.polyglot.Context;
import org.graalvm.polyglot.PolyglotException;
import org.junit.Test;

public class HaraStreamTest {
  @Test
  public void generatorYieldsStructuredValuesAndUsesNilForEof() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      assertEquals(
          "[true true :std.native.Stream {:index 0} {:index 1} nil]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(let [s (Stream/generate "
                      + "(fn [n] (loop [i 0] (if (< i n) "
                      + "(do (Coroutine/yield {:index i}) (recur (inc i))) :done))) 2)] "
                      + "[(= (type s) :std.native.Stream) (satisfies? IStream s) (type s) "
                      + "(deref (Stream/next s)) (deref (IStream/next s)) "
                      + "(deref (Stream/next s))])")
              .toString());
    }
  }

  @Test
  public void yieldedNilRejectsAndCloses() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      context.eval(
          HaraLanguage.ID,
          "(def nil-stream (Stream/generate (fn [] (Coroutine/yield nil) :done)))");
      PolyglotException error =
          assertThrows(
              PolyglotException.class,
              () -> context.eval(HaraLanguage.ID, "(deref (Stream/next nil-stream))"));
      assertTrue(error.getMessage().contains("stream/nil-item"));
      assertTrue(context.eval(HaraLanguage.ID, "(nil? (deref (Stream/next nil-stream)))").asBoolean());
    }
  }

  @Test
  public void closeIsIdempotent() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      assertTrue(
          context
              .eval(
                  HaraLanguage.ID,
                  "(let [s (Stream/generate (fn [] (Coroutine/yield :never)))] "
                      + "(IClose/close s) (IClose/close s) (nil? (deref (Stream/next s))))")
              .asBoolean());
    }
  }

  @Test
  public void callbackStreamAcceptsGuestHaraFunctions() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      assertEquals(
          "[42 true nil]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(let [closed (atom false) "
                      + "s (Stream/create (fn [] (promise/from 42)) "
                      + "                 (fn [] (reset! closed true)))] "
                      + "  (let [value (deref (Stream/next s))] "
                      + "    (IClose/close s) "
                      + "    [value (deref closed) (deref (Stream/next s))]))")
              .toString());
    }
  }

  @Test
  public void duplexIsPortableAndNotExposedAsANativeBox() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      for (String source :
          new String[] {
            "Duplex", "std.native.Duplex", "(Process/duplex nil)", "(Socket/duplex nil)"
          }) {
        assertThrows(PolyglotException.class, () -> context.eval(HaraLanguage.ID, source));
      }
    }
  }
}
