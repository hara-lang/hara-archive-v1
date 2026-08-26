package hara.portable;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertTrue;

import hara.truffle.HaraNativeTestRunner;
import java.nio.file.Path;
import org.junit.Test;

public final class HaraTypeScriptFacetTest {
  private static final Path ROOT = Path.of(".").toAbsolutePath().normalize();

  @Test
  public void runsTypeScriptDeclarationFacetSuite() throws Exception {
    HaraNativeTestRunner.Result result =
        HaraNativeTestRunner.runFile(
            ROOT, ROOT.resolve("lib/test-lang/lang/model/v1/spec_js/ts_test.hal"));

    assertTrue(result.failureMessage(), result.passed());
    assertEquals(9, result.facts());
    assertEquals(9, result.checks());
    assertEquals(9, result.passedChecks());
    assertEquals(0, result.failedChecks());
    assertEquals(0, result.errors());
    assertEquals(0, result.timeouts());
  }
}
