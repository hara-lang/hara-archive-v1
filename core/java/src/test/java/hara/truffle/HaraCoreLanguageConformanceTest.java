package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertTrue;
import static org.junit.Assert.fail;

import hara.kernel.base.Parser;
import hara.lang.base.G;
import hara.lang.data.Keyword;
import hara.lang.protocol.ILinearType;
import hara.lang.protocol.IMapType;
import hara.spec.SpecRegistry;
import java.math.BigInteger;
import java.nio.file.Files;
import java.nio.file.Path;
import org.graalvm.polyglot.Context;
import org.graalvm.polyglot.PolyglotException;
import org.graalvm.polyglot.Value;
import org.junit.Test;
import org.junit.experimental.categories.Category;

/** Executes the versioned JVM core-language conformance corpus. */
@Category(hara.spec.RegistryConformance.class)
public class HaraCoreLanguageConformanceTest {
  private static Keyword key(String name) {
    return Keyword.create(null, name);
  }

  @SuppressWarnings({"rawtypes", "unchecked"})
  @Test
  public void executesEveryJvmCoreLanguageCorpusCase() throws Exception {
    executeCorpus("core.edn", System.getenv("HARA_CORE_CASE_PREFIX"));
  }

  @SuppressWarnings({"rawtypes", "unchecked"})
  @Test
  public void executesEveryJvmPortableExceptionCorpusCase() throws Exception {
    executeCorpus("exceptions.edn", null);
  }

  @SuppressWarnings({"rawtypes", "unchecked"})
  private void executeCorpus(String name, String casePrefix) throws Exception {
    String source =
        Files.readString(
            SpecRegistry.require("01-lang/001-language/draft/conformance/" + name));
    IMapType manifest = (IMapType) Parser.LispReader.readString(source, null);
    ILinearType<?> cases = (ILinearType<?>) manifest.lookup(key("cases"));
    assertTrue(cases.count() > 0);
    for (Object item : cases) {
      IMapType testCase = (IMapType) item;
      Keyword idKeyword = (Keyword) testCase.lookup(key("id"));
      String id =
          idKeyword.getNamespace() == null
              ? idKeyword.getName()
              : idKeyword.getNamespace() + "/" + idKeyword.getName();
      if (casePrefix != null && !id.startsWith(casePrefix)) continue;
      Object classValue = testCase.lookup(key("class"));
      String className = classValue == null ? "error" : ((Keyword) classValue).getName();
      String form = (String) testCase.lookup(key("source"));
      IMapType expected = (IMapType) testCase.lookup(key("expect"));
      if ("reader".equals(className)) {
        assertReaderCase(id, form, expected);
      } else if (expected.lookup(key("error")) != null) {
        assertErrorCase(id, form, (String) testCase.lookup(key("setup")), expected);
      } else {
        assertValueCase(id, form, (String) testCase.lookup(key("setup")), expected);
      }
    }
  }

  private void assertReaderCase(String id, String form, IMapType expected) {
    if (expected.lookup(key("error")) != null) {
      try {
        Parser.LispReader.readString(form, null);
        fail(id + " should fail");
      } catch (RuntimeException error) {
        Object message = expectedMessage(expected);
        if (message != null) {
          assertTrue(id, error.getMessage().contains(message.toString()));
        }
      }
      return;
    }
    Object value = Parser.LispReader.readString(form, null);
    java.util.Map.Entry valueEntry = (java.util.Map.Entry) expected.find(key("value"));
    if (valueEntry == null) {
      assertEquals(id, expected.lookup(key("form")), G.display(value));
    } else {
      Object expectedValue = valueEntry.getValue();
      if (expectedValue instanceof String) {
        assertEquals(id, expectedValue, value == null ? null : value.toString());
      } else {
        assertEquals(
            id,
            expectedValue == null ? null : expectedValue.toString(),
            value == null ? null : value.toString());
      }
    }
  }

  private void assertErrorCase(String id, String form, String setup, IMapType expected) {
    try (Context context = context()) {
      if (setup != null) context.eval(HaraLanguage.ID, setup);
      try {
        context.eval(HaraLanguage.ID, form);
        fail(id + " should fail");
      } catch (PolyglotException error) {
        assertTrue(id + " should report an error", error.isGuestException());
        Object message = expectedMessage(expected);
        if (message != null) {
          assertTrue(
              id
                  + " should contain its specified error `"
                  + message
                  + "`, actual: `"
                  + error.getMessage()
                  + "`",
              error.getMessage().contains(message.toString()));
        }
      }
    }
  }

  private Object expectedMessage(IMapType expected) {
    return expected.lookup(key("message"));
  }

  private void assertValueCase(String id, String form, String setup, IMapType expected) {
    try (Context context = context()) {
      if (setup != null) context.eval(HaraLanguage.ID, setup);
      try {
        Value actual = context.eval(HaraLanguage.ID, form);
        assertExpectedValue(id, actual, expected.lookup(key("value")));
      } catch (PolyglotException error) {
        throw new AssertionError(id + " unexpectedly failed: " + error.getMessage(), error);
      }
    }
  }

  private void assertExpectedValue(String id, Value actual, Object expected) {
    if (expected == null) {
      assertTrue(id + " should return nil", actual.isNull());
    } else if (expected instanceof Boolean) {
      assertEquals(id, expected, actual.asBoolean());
    } else if (expected instanceof String) {
      assertEquals(id, expected, actual.asString());
    } else if (expected instanceof BigInteger) {
      assertEquals(id, expected, actual.as(BigInteger.class));
    } else if (expected instanceof Double) {
      assertEquals(id, (Double) expected, actual.asDouble(), 0.0);
    } else if (expected instanceof Keyword) {
      assertEquals(id, G.display(expected), actual.toString());
    } else if (expected instanceof Number) {
      assertEquals(id, ((Number) expected).longValue(), actual.asLong());
    } else {
      fail(id + " has unsupported expected value: " + expected);
    }
  }

  private static Context context() {
    return Context.newBuilder(HaraLanguage.ID).build();
  }
}
