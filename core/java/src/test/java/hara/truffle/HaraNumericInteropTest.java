package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertThrows;
import static org.junit.Assert.assertTrue;

import com.oracle.truffle.api.interop.InteropLibrary;
import com.oracle.truffle.api.interop.UnsupportedMessageException;
import java.math.BigDecimal;
import java.math.BigInteger;
import org.graalvm.polyglot.Context;
import org.graalvm.polyglot.PolyglotException;
import org.graalvm.polyglot.Value;
import org.junit.Test;

public class HaraNumericInteropTest {
  private static final BigInteger LARGE_INTEGER = new BigInteger("123456789012345678901234567890");

  @Test
  public void exportsBigIntegersAsExactPolyglotNumbers() throws Exception {
    InteropLibrary interop = InteropLibrary.getUncached();
    Object exported = HaraBox.export(LARGE_INTEGER);

    assertTrue(interop.isNumber(exported));
    assertTrue(interop.fitsInBigInteger(exported));
    assertFalse(interop.fitsInLong(exported));
    assertFalse(interop.fitsInDouble(exported));
    assertEquals(LARGE_INTEGER, interop.asBigInteger(exported));
    assertThrows(UnsupportedMessageException.class, () -> interop.asLong(exported));
  }

  @Test
  public void exportsHostBigDecimalsAsFiniteFloats() throws Exception {
    InteropLibrary interop = InteropLibrary.getUncached();
    Object exported = HaraBox.export(new BigDecimal("1.2300"));

    assertTrue(exported instanceof Double);
    assertTrue(interop.isNumber(exported));
    assertTrue(interop.fitsInDouble(exported));
    assertEquals(1.23d, interop.asDouble(exported), 0.0d);
  }

  @Test
  public void rejectsHostBigDecimalsOutsideFiniteFloatRange() {
    IllegalArgumentException error =
        assertThrows(
            IllegalArgumentException.class,
            () -> HaraBox.export(new BigDecimal("1e1000000")));
    assertTrue(error.getMessage().contains("finite Hara Float range"));
  }

  @Test
  public void rejectsHostNonFiniteFloatsAtTheLanguageBoundary() {
    for (Number value :
        new Number[] {
          Double.NaN,
          Double.POSITIVE_INFINITY,
          Double.NEGATIVE_INFINITY,
          Float.NaN,
          Float.POSITIVE_INFINITY,
          Float.NEGATIVE_INFINITY
        }) {
      assertThrows(HaraException.class, () -> HaraBox.export(value));
      assertThrows(HaraException.class, () -> new HaraBox(value));
    }
  }

  @Test
  public void exposesArbitraryIntegersAndFloatsAtTheLanguageBoundary() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      Value integer = context.eval(HaraLanguage.ID, Long.toString(Long.MAX_VALUE));
      assertTrue(integer.isNumber());
      assertEquals(Long.MAX_VALUE, integer.asLong());

      Value large = context.eval(HaraLanguage.ID, LARGE_INTEGER.toString());
      assertTrue(large.isNumber());
      assertTrue(large.fitsInBigInteger());
      assertFalse(large.fitsInLong());
      assertEquals(LARGE_INTEGER, large.as(BigInteger.class));

      Value belowLong = context.eval(HaraLanguage.ID, BigInteger.valueOf(Long.MIN_VALUE).subtract(BigInteger.ONE).toString());
      assertEquals(BigInteger.valueOf(Long.MIN_VALUE).subtract(BigInteger.ONE), belowLong.as(BigInteger.class));

      Value floating = context.eval(HaraLanguage.ID, "1.2300");
      assertTrue(floating.isNumber());
      assertTrue(floating.fitsInDouble());
      assertEquals(1.23, floating.asDouble(), 0.0);

      PolyglotException suffix =
          assertThrows(PolyglotException.class, () -> context.eval(HaraLanguage.ID, "1.2300M"));
      assertTrue(suffix.getMessage().contains("legacy numeric suffixes N and M"));
    }
  }
}
