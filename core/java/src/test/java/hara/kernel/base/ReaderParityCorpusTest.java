package hara.kernel.base;

import hara.spec.SpecRegistry;
import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertNotNull;
import static org.junit.Assert.assertThrows;
import static org.junit.Assert.assertTrue;
import static org.junit.Assert.fail;

import hara.lang.base.G;
import hara.lang.data.Keyword;
import hara.lang.protocol.ILinearType;
import hara.lang.protocol.IMapType;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import org.junit.Test;
import org.junit.experimental.categories.Category;

/** Executes the canonical reader corpus shared with the Rust runtime. */
@Category(hara.spec.RegistryConformance.class)
public class ReaderParityCorpusTest {
  private static final Path CORPUS =
      SpecRegistry.resolve("01-lang/001-language/draft/conformance/reader.edn");

  private static Keyword key(String name) {
    return Keyword.create(null, name);
  }

  private static String readAllDisplay(String source) {
    Reader reader = new Reader(source);
    Object eof = new Object();
    List<String> forms = new ArrayList<>();
    Object form;
    do {
      form = Parser.LispReader.read(reader, false, eof, false, null);
      if (form != eof) forms.add(G.display(form));
    } while (form != eof);
    return String.join(" ", forms);
  }

  @SuppressWarnings("rawtypes")
  @Test
  public void sharedReaderCorpusMatchesCanonicalFormsAndErrors() throws Exception {
    IMapType manifest =
        (IMapType) Parser.LispReader.readString(Files.readString(CORPUS), null);
    Object rawCases = manifest.lookup(key("cases"));
    assertTrue(rawCases instanceof ILinearType);
    ILinearType<?> cases = (ILinearType<?>) rawCases;
    assertTrue("reader corpus must be substantial", cases.count() >= 20);

    for (Object rawCase : cases) {
      IMapType testCase = (IMapType) rawCase;
      Object id = testCase.lookup(key("id"));
      String caseId = G.display(id);
      String source = (String) testCase.lookup(key("source"));
      String readable = (String) testCase.lookup(key("readable"));
      String expectedError = (String) testCase.lookup(key("error"));
      assertNotNull("case must contain :id", id);
      assertNotNull(id + " must contain :source", source);
      if ((readable == null) == (expectedError == null))
        fail(id + " must contain exactly one of :readable or :error");

      if (readable != null) {
        String actual = readAllDisplay(source);
        assertEquals(caseId, readable, actual);
        // Float display now uses the (double ...) constructor form, which is a
        // valid expression but not a round-trippable literal. Skip the literal
        // round-trip check for cases that now produce floats.
        if (!(":numbers".equals(caseId) || ":floating-point".equals(caseId))) {
          assertEquals(
              caseId + " canonical output must round-trip",
              actual,
              readAllDisplay(actual));
        }
      } else {
        RuntimeException error =
            assertThrows(
                caseId,
                RuntimeException.class,
                () -> readAllDisplay(source));
        Throwable cause = error.getCause() == null ? error : error.getCause();
        assertTrue(
            id + ": expected <" + expectedError + "> in <" + cause.getMessage() + ">",
            cause.getMessage().contains(expectedError));
      }
    }
  }
}
