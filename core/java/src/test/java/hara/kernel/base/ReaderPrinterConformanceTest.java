package hara.kernel.base;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertThrows;
import static org.junit.Assert.assertTrue;

import hara.lang.base.G;
import hara.lang.data.HaraCharacter;
import hara.lang.data.Keyword;
import hara.lang.protocol.IMapType;
import hara.lang.protocol.IObjType;
import org.junit.Test;

public class ReaderPrinterConformanceTest {
  @Test
  public void canonicalCharacterPrintingRoundTripsNamedAndUnicodeValues() {
    HaraCharacter[] values = {
      HaraCharacter.of('\n'),
      HaraCharacter.of(' '),
      HaraCharacter.of('\t'),
      HaraCharacter.of('\b'),
      HaraCharacter.of('\f'),
      HaraCharacter.of('\r'),
      HaraCharacter.of('\0'),
      HaraCharacter.of('a'),
      HaraCharacter.of('\u03bb'),
      HaraCharacter.of(0x1F600)
    };
    String[] displays = {
      "\\newline",
      "\\space",
      "\\tab",
      "\\backspace",
      "\\formfeed",
      "\\return",
      "\\u0000",
      "\\a",
      "\\λ",
      "\\😀"
    };

    for (int i = 0; i < values.length; i++) {
      assertEquals(displays[i], G.display(values[i]));
      assertEquals(values[i], Parser.LispReader.readString(displays[i], null));
    }
  }

  @Test
  public void nestedImmutableDataHasStableReadableRoundTrip() {
    String source = "{:message \"line\\nvalue\" :values [1 2 3] :flags #{:a :b}}";
    Object value = Parser.LispReader.readString(source, null);
    String readable = G.display(value);
    assertEquals(source, readable);
    Object roundTripped = Parser.LispReader.readString(readable, null);
    assertEquals(readable, G.display(roundTripped));
  }

  @Test
  public void malformedEscapesAndCollectionFormsReportSpecificFailures() {
    assertReaderFailure("\"\\u12X4\"", "Invalid digit");
    assertReaderFailure("\"\\q\"", "Unsupported escape character");
    assertReaderFailure("\\u12X4", "Invalid digit");
    assertReaderFailure("{:a 1 :b}", "even number of forms");
    assertReaderFailure("#{1 1}", "Duplicate item");
  }

  @Test
  public void bytesUseTheirOrdinaryReadableConstructor() {
    byte[] bytes = {1, 2, -3};
    assertEquals("(bytes 1 2 -3)", G.display(bytes));
  }

  @Test
  public void immutableFormsCarrySourceSpanMetadataWithoutChangingValueOrPrinting() {
    Object form = Parser.LispReader.readString("(+ 1 2)", null);
    @SuppressWarnings("rawtypes")
    IMapType metadata = (IMapType) ((IObjType) form).meta();
    assertEquals(1L, ((Number) metadata.lookup(Keyword.create("line"))).longValue());
    assertEquals(1L, ((Number) metadata.lookup(Keyword.create("column"))).longValue());
    assertTrue(((Number) metadata.lookup(Keyword.create("end-line"))).longValue() >= 1);
    assertTrue(((Number) metadata.lookup(Keyword.create("end-column"))).longValue() > 1);
    assertEquals("(+ 1 2)", G.display(form));
    assertEquals(G.display(form), G.display(Parser.LispReader.readString(G.display(form), null)));
  }

  private static void assertReaderFailure(String source, String expected) {
    RuntimeException error =
        assertThrows(RuntimeException.class, () -> Parser.LispReader.readString(source, null));
    Throwable cause = error.getCause() == null ? error : error.getCause();
    assertTrue(
        "Expected <" + expected + "> in <" + cause.getMessage() + ">",
        cause.getMessage().contains(expected));
  }
}
