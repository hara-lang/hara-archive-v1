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
import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import org.graalvm.polyglot.Context;
import org.graalvm.polyglot.PolyglotException;
import org.graalvm.polyglot.Value;
import org.junit.Test;
import org.junit.experimental.categories.Category;

/** Executes the shared source-level stack/work corpus on the JVM evaluator. */
@Category(hara.spec.RegistryConformance.class)
public class HaraStackSafetyConformanceTest {
  private static Keyword key(String name) {
    return Keyword.create(null, name);
  }

  @Test
  public void executesEveryJvmStackSafetyCorpusCase() throws Exception {
    IMapType document = readCorpus();
    ILinearType<?> cases = (ILinearType<?>) document.lookup(key("cases"));
    assertTrue("stack-safety corpus must not be empty", cases.count() > 0);

    List<String> results = new ArrayList<>();
    int passed = 0;
    for (Object rawCase : cases) {
      IMapType testCase = (IMapType) rawCase;
      String id = G.display(testCase.lookup(key("id")));
      String source = (String) testCase.lookup(key("source"));
      IMapType expected = (IMapType) testCase.lookup(key("expect"));
      try {
        if (expected.lookup(key("error")) != null) {
          assertErrorCase(id, source, expected);
        } else {
          assertValueCase(id, source, expected.lookup(key("value")));
        }
        results.add("{:id " + id + " :status :passed}");
        passed++;
      } catch (AssertionError error) {
        results.add(
            "{:id " + id + " :status :failed :message " + quote(error.getMessage()) + "}");
        throw error;
      }
    }
    writeReport((int) cases.count(), passed, results);
  }

  private static IMapType readCorpus() throws IOException {
    Path path = SpecRegistry.resolve("01-lang/001-language/draft/conformance/stack-safety.edn");
    Object parsed = Parser.LispReader.readString(Files.readString(path), null);
    assertTrue("stack-safety corpus must be a map", parsed instanceof IMapType);
    return (IMapType) parsed;
  }

  private static void assertErrorCase(String id, String source, IMapType expected) {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      context.eval(HaraLanguage.ID, source);
      fail(id + " should fail");
    } catch (PolyglotException error) {
      assertTrue(id + " should report a guest error", error.isGuestException());
      Object message = expected.lookup(key("message"));
      if (message != null) {
        assertTrue(
            id + " should contain `" + message + "`, actual: `" + error.getMessage() + "`",
            error.getMessage().contains(message.toString()));
      }
    }
  }

  private static void assertValueCase(String id, String source, Object expected) {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      Value actual = context.eval(HaraLanguage.ID, source);
      assertExpectedValue(id, actual, expected);
    } catch (PolyglotException error) {
      throw new AssertionError(id + " unexpectedly failed: " + error.getMessage(), error);
    }
  }

  @SuppressWarnings("rawtypes")
  private static void assertExpectedValue(String id, Value actual, Object expected) {
    if (expected == null) {
      assertTrue(id + " should return nil", actual.isNull());
    } else if (expected instanceof Boolean) {
      assertEquals(id, expected, actual.asBoolean());
    } else if (expected instanceof Number) {
      assertEquals(id, ((Number) expected).longValue(), actual.asLong());
    } else if (expected instanceof String) {
      assertEquals(id, expected, actual.asString());
    } else if (expected instanceof ILinearType) {
      assertEquals(id, G.display(expected), actual.toString());
    } else {
      fail(id + " has unsupported expected value: " + expected);
    }
  }

  private static void writeReport(int total, int passed, List<String> results)
      throws IOException {
    String defaultRoot = System.getProperty("user.dir");
    Path reportDirectory =
        Path.of(
            System.getenv()
                .getOrDefault("HARA_CONFORMANCE_REPORT_DIR", Path.of(defaultRoot, "target/conformance").toString()));
    Files.createDirectories(reportDirectory.resolve("jvm"));
    String report =
        "{:report/schema :hara.conformance.runtime/0-alpha"
            + " :report/suite :hal/stack-safety"
            + " :report/runtime :jvm"
            + " :report/status "
            + (passed == total ? ":passed" : ":failed")
            + " :report/passed "
            + passed
            + " :report/total "
            + total
            + " :report/cases ["
            + String.join(" ", results)
            + "]}\n";
    Files.writeString(reportDirectory.resolve("jvm/stack-safety.edn"), report);
  }

  private static String quote(String value) {
    if (value == null) return "nil";
    return "\""
        + value.replace("\\", "\\\\").replace("\"", "\\\"").replace("\n", "\\n")
        + "\"";
  }
}
