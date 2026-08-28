package hara.kernel.base;

import hara.spec.SpecRegistry;
import hara.lang.data.*;
import org.junit.Test;

import java.nio.file.Files;
import java.nio.file.Path;

import static org.junit.Assert.*;

public class ParserTest {

  @Test
  public void testReadStringNumber() {
    assertEquals(123L, Parser.LispReader.readString("123", null));
    assertEquals(Double.valueOf(123.45), Parser.LispReader.readString("123.45", null));
    assertEquals(0xFFL, Parser.LispReader.readString("0xFF", null));
    RuntimeException integerSuffix =
        assertThrows(RuntimeException.class, () -> Parser.LispReader.readString("123N", null));
    assertTrue(integerSuffix.getCause().getMessage().contains("legacy numeric suffixes N and M"));
    RuntimeException decimalSuffix =
        assertThrows(RuntimeException.class, () -> Parser.LispReader.readString("123.45M", null));
    assertTrue(decimalSuffix.getCause().getMessage().contains("legacy numeric suffixes N and M"));
    assertThrows(
        RuntimeException.class,
        () -> Parser.LispReader.readString("9223372036854775808", null));
    assertThrows(
        RuntimeException.class,
        () -> Parser.LispReader.readString("-9223372036854775809", null));
  }

  @Test
  public void readableNumericPrintingPreservesLongAndDoubleCategories() {
    assertEquals("123", hara.lang.base.G.display(123L));
    assertEquals("(double 123.45)", hara.lang.base.G.display(123.45));
    assertEquals(123L, Parser.LispReader.readString("123", null));
    assertEquals(Double.valueOf(123.45), Parser.LispReader.readString("123.45", null));
  }

  @Test
  @org.junit.experimental.categories.Category(hara.spec.RegistryConformance.class)
  public void l0ConformanceCorpusIsReadableEdn() throws Exception {
    Object corpus =
        Parser.LispReader.readString(
            Files.readString(
                SpecRegistry.require("01-lang/001-language/draft/conformance/core.edn")),
            null);
    assertTrue(corpus instanceof hara.lang.protocol.IMapType);
    hara.lang.protocol.IMapType map = (hara.lang.protocol.IMapType) corpus;
    assertEquals("0.0.0-alpha", map.lookup(Keyword.create("spec/version")));
    assertTrue(map.lookup(Keyword.create("cases")) instanceof hara.lang.protocol.ILinearType);
  }

  @Test
  public void testReadStringSymbol() {
    assertEquals(Symbol.create("a"), Parser.LispReader.readString("a", null));
    assertEquals(Keyword.create("a"), Parser.LispReader.readString(":a", null));
    assertEquals(Keyword.create("ns", "a"), Parser.LispReader.readString(":ns/a", null));
    assertEquals(Symbol.create("ns", "a"), Parser.LispReader.readString("ns/a", null));
  }

  @Test
  public void testReadStringString() {
    assertEquals("hello", Parser.LispReader.readString("\"hello\"", null));
    assertEquals("hello \"world\"", Parser.LispReader.readString("\"hello \\\"world\\\"\"", null));
  }

  @Test
  public void testReadStringChar() {
    assertEquals(HaraCharacter.of('a'), Parser.LispReader.readString("\\a", null));
    assertEquals(HaraCharacter.of('\n'), Parser.LispReader.readString("\\newline", null));
    assertEquals(HaraCharacter.of(' '), Parser.LispReader.readString("\\space", null));
  }

  @Test
  public void testReadStringList() {
    Object result = Parser.LispReader.readString("(+ 1 2)", null);
    assertTrue(result instanceof List);
    List list = (List) result;
    assertEquals(3, list.count());
    assertEquals(Symbol.create("+"), list.nth(0));
    assertEquals(1L, list.nth(1));
    assertEquals(2L, list.nth(2));
  }

  @Test
  public void expandsAnonymousFunctionArgumentsDeterministically() {
    assertEquals(
        "(fn [__reader_fn_0_1 __reader_fn_0_2 & __reader_fn_0_rest] (+ __reader_fn_0_1 __reader_fn_0_2 (count __reader_fn_0_rest)))",
        hara.lang.base.G.display(
            Parser.LispReader.readString("#(+ % %2 (count %&))", null)));
    RuntimeException error =
        assertThrows(
            RuntimeException.class,
            () -> Parser.LispReader.readString("#(+ %0 1)", null));
    assertTrue(error.getCause().getMessage().contains("arguments begin at %1"));
  }

  @Test
  public void testReadStringVector() {
    Object result = Parser.LispReader.readString("[1 2 3]", null);
    assertTrue(result instanceof hara.lang.data.Tuple.Tup1);
    hara.lang.protocol.ILinearType v = (hara.lang.protocol.ILinearType) result;
    assertEquals(3, v.count());
  }

  @Test
  public void testReadStringVectorLarge() {
    Object compact = Parser.LispReader.readString("[1 2 3 4 5 6 7 8]", null);
    assertTrue(compact instanceof hara.lang.data.Tuple.Tup8);
    Object result = Parser.LispReader.readString("[1 2 3 4 5 6 7 8 9]", null);
    assertTrue(result instanceof Vector);
    Vector v = (Vector) result;
    assertEquals(9, v.count());
  }

  @Test
  public void testReadStringMap() {
    Object result = Parser.LispReader.readString("{:a 1 :b 2}", null);
    assertTrue(result instanceof hara.lang.data.Map);
    hara.lang.data.Map map = (hara.lang.data.Map) result;
    assertEquals(2, map.count());
    assertEquals(1L, map.lookup(Keyword.create("a")));
    assertEquals(2L, map.lookup(Keyword.create("b")));
  }

  @Test
  public void testReadQuote() {
    Object result = Parser.LispReader.readString("'a", null);
    assertTrue(result instanceof List);
    List l = (List) result;
    assertEquals(Symbol.create("quote"), l.nth(0));
    assertEquals(Symbol.create("a"), l.nth(1));
  }

  @Test
  public void testReadComment() {
    // Comments return the reader, which loop in read() ignores.
    // readString(";", null) -> might throw EOF if nothing else.
    try {
      Parser.LispReader.readString("; comment", null);
      fail("Should throw EOF");
    } catch (Exception e) {
      // Expected EOF
    }

    assertEquals(1L, Parser.LispReader.readString("; comment\n1", null));
  }

  @Test
  public void testReadMetadata() {
    Object result = Parser.LispReader.readString("^:foo [1]", null);
    assertTrue(result instanceof hara.lang.protocol.IObjType);
    hara.lang.protocol.IObjType obj = (hara.lang.protocol.IObjType) result;
    hara.lang.protocol.IMapType meta = (hara.lang.protocol.IMapType) obj.meta();
    assertEquals(Boolean.TRUE, meta.lookup(Keyword.create("foo")));
  }

  @Test
  public void testReadMapMetadata() {
    Object result = Parser.LispReader.readString("^{:tag \"fast\"} [1]", null);
    assertTrue(result instanceof hara.lang.protocol.IObjType);
    hara.lang.protocol.IObjType obj = (hara.lang.protocol.IObjType) result;
    hara.lang.protocol.IMapType meta = (hara.lang.protocol.IMapType) obj.meta();
    assertEquals("fast", meta.lookup(Keyword.create("tag")));
  }

  @Test
  public void testSyntaxQuote() {
    Object result = Parser.LispReader.readString("`a", null);
    assertTrue(result instanceof List);
    assertEquals(Symbol.create("syntax-quote"), ((List) result).nth(0));
  }

  @Test
  public void testReadUnmatchedDelimiter() {
    try {
      Parser.LispReader.readString(")", null);
      fail("Should throw RuntimeException for unmatched delimiter");
    } catch (Exception e) {
      assertTrue(e.getMessage().contains("Unmatched delimiter"));
    }
  }

  @Test
  public void testReadUnfinishedString() {
    try {
      Parser.LispReader.readString("\"", null);
      fail("Should throw RuntimeException for EOF while reading string");
    } catch (Exception e) {
      // ReaderException wraps the actual exception
      assertTrue(e.getCause().getMessage().contains("EOF while reading string"));
    }
  }

  @Test
  public void testReadInvalidNumber() {
    try {
      // "123a" - Parser splits at macro/whitespace. 'a' is not macro/whitespace.
      // Wait, 123a is read as a token? No, readNumber reads until macro or
      // whitespace.
      // If 1 starts, it calls readNumber.
      // readNumber loops until macro or whitespace. 'a' is neither.
      // So it reads "123a" and tries to matchNumber("123a").
      // matchNumber will return null.
      // Then it throws NumberFormatException.
      Parser.LispReader.readString("123a", null);
      fail("Should throw NumberFormatException");
    } catch (Exception e) {
      assertTrue(e.getCause() instanceof NumberFormatException);
    }
  }

  @Test
  public void testReadNilTrueFalse() {
    assertNull(Parser.LispReader.readString("nil", null));
    assertEquals(Boolean.TRUE, Parser.LispReader.readString("true", null));
    assertEquals(Boolean.FALSE, Parser.LispReader.readString("false", null));
  }

  @Test
  public void testDeref() {
    Object result = Parser.LispReader.readString("@a", null);
    assertTrue(result instanceof List);
    List l = (List) result;
    assertEquals(Symbol.create("deref"), l.nth(0));
    assertEquals(Symbol.create("a"), l.nth(1));
  }

  @Test
  public void testMapDuplicateKey() {
    try {
      Parser.LispReader.readString("{:a 1 :a 2}", null);
      fail("Expected RuntimeException for duplicate key");
    } catch (Parser.LispReader.ReaderException e) {
      assertTrue(e.getCause().getMessage().contains("Duplicate key"));
    } catch (hara.lang.base.Ex.Runtime e) {
      assertTrue(e.getMessage().contains("Duplicate key"));
    }
  }

  @Test
  public void testSetDuplicateItem() {
    try {
      Parser.LispReader.readString("#{1 1}", null);
      fail("Expected RuntimeException for duplicate item");
    } catch (Parser.LispReader.ReaderException e) {
      assertTrue(e.getCause().getMessage().contains("Duplicate item"));
    } catch (hara.lang.base.Ex.Runtime e) {
      assertTrue(e.getMessage().contains("Duplicate item"));
    }
  }

  @Test
  public void testDiscardReader() {
    // #_ ignores the next form
    assertEquals(1L, Parser.LispReader.readString("#_ 2 1", null));
  }

  @Test
  public void taggedHandleSyntaxReadsAsInertData() {
    Object result = Parser.LispReader.readString("#math[:tensor 42]", null);
    assertTrue(result instanceof TaggedLiteral);
    TaggedLiteral tagged = (TaggedLiteral) result;
    assertEquals(Symbol.create("math"), tagged.tag());
    assertEquals("#math[:tensor 42]", tagged.display());
    assertEquals(result, Parser.LispReader.readString(tagged.display(), null));
  }

  @Test
  public void testVarQuoteReader() {
    Object result = Parser.LispReader.readString("#'a", null);
    assertTrue(result instanceof List);
    List l = (List) result;
    assertEquals(Symbol.create("var"), l.nth(0));
    assertEquals(Symbol.create("a"), l.nth(1));
  }

  @Test
  public void testQueueDispatchIsRejected() {
    try {
      Parser.LispReader.readString("#[1 2]", null);
      fail("Expected unknown dispatch macro");
    } catch (Parser.LispReader.ReaderException e) {
      assertTrue(e.getCause().getMessage().contains("No dispatch macro for: ["));
    }
  }

  @Test
  public void testRegexReader() {
    Object result = Parser.LispReader.readString("#\"abc\"", null);
    assertTrue(result instanceof java.util.regex.Pattern);
    java.util.regex.Pattern p = (java.util.regex.Pattern) result;
    assertEquals("abc", p.pattern());
  }

  @Test
  public void testUnquoteReader() {
    // Unquote is usually only valid inside syntax-quote, but the reader just
    // produces a symbol wrapping
    Object result = Parser.LispReader.readString("~a", null);
    assertTrue(result instanceof List);
    List l = (List) result;
    assertEquals(Symbol.create("unquote"), l.nth(0));
    assertEquals(Symbol.create("a"), l.nth(1));

    Object resultSplice = Parser.LispReader.readString("~@a", null);
    assertTrue(resultSplice instanceof List);
    List lSplice = (List) resultSplice;
    assertEquals(Symbol.create("unquote-splicing"), lSplice.nth(0));
    assertEquals(Symbol.create("a"), lSplice.nth(1));
  }

  @Test
  public void testCharacterReaderExtended() {
    assertEquals(HaraCharacter.of('\u0000'), Parser.LispReader.readString("\\u0000", null));
    assertEquals(HaraCharacter.of(0x1F600), Parser.LispReader.readString("\\u01F600", null));
    assertEquals(HaraCharacter.of(0x1F600), Parser.LispReader.readString("\\😀", null));
    assertEquals(HaraCharacter.of('\uFFFF'), Parser.LispReader.readString("\\uFFFF", null));
    assertThrows(RuntimeException.class, () -> Parser.LispReader.readString("\\uD800", null));
    assertEquals(HaraCharacter.of('\t'), Parser.LispReader.readString("\\tab", null));
    assertEquals(HaraCharacter.of('\b'), Parser.LispReader.readString("\\backspace", null));
    assertEquals(HaraCharacter.of('\f'), Parser.LispReader.readString("\\formfeed", null));
    assertEquals(HaraCharacter.of('\r'), Parser.LispReader.readString("\\return", null));

    // Octal
    assertEquals(HaraCharacter.of('\007'), Parser.LispReader.readString("\\o007", null));
    assertEquals(HaraCharacter.of('\377'), Parser.LispReader.readString("\\o377", null));
  }

  @Test
  public void testStringReaderExtended() {
    assertEquals(
        "\t\r\n\b\f\\\"", Parser.LispReader.readString("\"\\t\\r\\n\\b\\f\\\\\\\"\"", null));
    assertEquals("\u0000", Parser.LispReader.readString("\"\\u0000\"", null));
    assertEquals("\7", Parser.LispReader.readString("\"\\7\"", null)); // Octal in string
  }
}
