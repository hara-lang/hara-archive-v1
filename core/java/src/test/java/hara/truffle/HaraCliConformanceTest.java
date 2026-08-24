package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertNull;

import hara.kernel.base.Parser;
import hara.lang.data.Keyword;
import hara.lang.protocol.ILinearType;
import hara.lang.protocol.IMapType;
import java.io.InputStream;
import java.io.ByteArrayOutputStream;
import java.io.PrintStream;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import org.junit.Test;

@org.junit.experimental.categories.Category(hara.spec.RegistryConformance.class)
public class HaraCliConformanceTest {
  @Test
  public void rootHelpIsSuccessfulAndPublicOnly() {
    ByteArrayOutputStream output = new ByteArrayOutputStream();
    ByteArrayOutputStream error = new ByteArrayOutputStream();
    int status =
        Main.run(
            new String[] {"--help"},
            new PrintStream(output),
            new PrintStream(error));
    assertEquals(0, status);
    String help = output.toString(StandardCharsets.UTF_8);
    assertEquals(false, help.contains("compile-halc"));
    assertEquals("", error.toString(StandardCharsets.UTF_8));
  }

  @Test
  @SuppressWarnings({"rawtypes", "unchecked"})
  public void sharedRouteCasesPass() throws Exception {
    String resource = "02-platform/000001-cli/draft/conformance/routes.edn";
    try (InputStream input = getClass().getClassLoader().getResourceAsStream(resource)) {
      if (input == null) throw new AssertionError("Missing " + resource);
      Object document =
          Parser.LispReader.readString(
              new String(input.readAllBytes(), StandardCharsets.UTF_8), null);
      ILinearType cases =
          (ILinearType) ((IMapType) document).lookup(Keyword.create("conformance/cases"));
      for (Object item : cases) {
        IMapType testCase = (IMapType) item;
        String id = ((Keyword) testCase.lookup(Keyword.create("case/id"))).display();
        ArrayList<String> argv = new ArrayList<>();
        for (Object argument :
            (ILinearType) testCase.lookup(Keyword.create("case/argv"))) {
          argv.add((String) argument);
        }
        IMapType expected =
            (IMapType) testCase.lookup(Keyword.create("case/expected"));
        Object routeValue = expected.lookup(Keyword.create("route/id"));
        HaraCliRouter.Resolution resolution =
            HaraCliRouter.instance().resolve(argv.toArray(new String[0]));
        if (routeValue instanceof Keyword route) {
          assertEquals(id, route.display().substring(1), resolution.route().id());
          Object arguments = expected.lookup(Keyword.create("route/arguments"));
          if (arguments instanceof ILinearType values) {
            ArrayList<String> expectedArguments = new ArrayList<>();
            for (Object value : values) expectedArguments.add((String) value);
            assertEquals(id, expectedArguments, resolution.arguments());
          }
        } else {
          assertNull(id, resolution);
        }
      }
    }
  }

  @Test
  public void routeOptionTerminatorIsRemoved() {
    HaraCliRouter.Resolution resolution =
        HaraCliRouter.instance().resolve(new String[] {"eval", "--", "(- 2 1)"});
    assertEquals(java.util.List.of("(- 2 1)"), resolution.arguments());
  }

  @Test
  @SuppressWarnings({"rawtypes", "unchecked"})
  public void sharedOutcomeCasesPass() throws Exception {
    String resource = "02-platform/000001-cli/draft/conformance/outcomes.edn";
    try (InputStream input = getClass().getClassLoader().getResourceAsStream(resource)) {
      if (input == null) throw new AssertionError("Missing " + resource);
      Object document =
          Parser.LispReader.readString(
              new String(input.readAllBytes(), StandardCharsets.UTF_8), null);
      ILinearType cases =
          (ILinearType) ((IMapType) document).lookup(Keyword.create("conformance/cases"));
      for (Object item : cases) {
        IMapType testCase = (IMapType) item;
        Object inputValue = testCase.lookup(Keyword.create("case/input"));
        if (inputValue instanceof Keyword outcome) {
          int expected = (int) (long) testCase.lookup(Keyword.create("case/expected-exit"));
          assertEquals(expected, HaraCliRouter.outcomeExit(outcome.display().substring(1)));
        }
      }
    }
  }
}
