package hara.truffle;

import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertNotNull;
import static org.junit.Assert.assertTrue;

import hara.kernel.base.Parser;
import hara.spec.SpecRegistry;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.HashSet;
import java.util.List;
import java.util.Set;
import org.junit.Test;

@org.junit.experimental.categories.Category(hara.spec.RegistryConformance.class)
public class HaraDifferentialConformanceTest {
  private static final Path MANIFEST =
      specsRegistry()
          .resolve("01-lang/001-language/draft/conformance/parity/jvm-truffle.edn");

  @Test
  public void portableCasesMatchAcrossJvmInterpreterAndTruffle() throws Exception {
    List<HaraDifferentialRunner.TestCase> cases = cases();
    int compared = 0;
    List<String> mismatches = new ArrayList<>();
    for (HaraDifferentialRunner.TestCase testCase : cases) {
      if (!testCase.portable()) continue;
      HaraDifferentialRunner.Result result = HaraDifferentialRunner.run(testCase);
      compared++;
      if (!result.jvm().equals(result.truffle())) {
        mismatches.add(
            testCase.id().display() + ": JVM " + result.jvm() + ", Truffle " + result.truffle());
      }
    }
    assertTrue("The differential manifest must contain portable cases", compared > 0);
    assertTrue("Differential mismatches:\n" + String.join("\n", mismatches), mismatches.isEmpty());
  }

  @Test
  public void knownGapsRemainObservableUntilPromotedToPortable() throws Exception {
    for (HaraDifferentialRunner.TestCase testCase : cases()) {
      if (!testCase.knownGap()) continue;
      HaraDifferentialRunner.Result result = HaraDifferentialRunner.run(testCase);
      assertFalse(
          testCase.id().display() + " is no longer a gap; promote it to :portable",
          result.matches());
    }
  }

  @Test
  public void everyCaseHasAStableIdAndExplicitExecutionClassification() throws Exception {
    List<HaraDifferentialRunner.TestCase> cases = cases();
    Set<Object> ids = new HashSet<>();
    int excluded = 0;
    for (HaraDifferentialRunner.TestCase testCase : cases) {
      assertTrue("Duplicate parity case id " + testCase.id(), ids.add(testCase.id()));
      assertFalse(testCase.id() + " must contain source", testCase.source().isBlank());
      if (testCase.capability() != null) {
        excluded++;
        assertNotNull(testCase.id() + " must name its capability", testCase.capability());
        assertNotNull(testCase.id() + " must explain its exclusion", testCase.reason());
        assertFalse(testCase.id() + " must explain its exclusion", testCase.reason().isBlank());
      } else if (testCase.issue() != null) {
        assertFalse(testCase.id() + " must explain its known gap", testCase.reason().isBlank());
      }
    }
    assertTrue("Capability-specific cases must remain visible in the manifest", excluded > 0);
  }

  private static List<HaraDifferentialRunner.TestCase> cases() throws Exception {
    String source = Files.readString(MANIFEST);
    return HaraDifferentialRunner.readCases(Parser.LispReader.readString(source, null));
  }

  private static Path specsRegistry() {
    return SpecRegistry.root();
  }
}
