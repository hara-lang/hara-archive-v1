package hara.truffle;

import static org.junit.Assert.assertTrue;

import java.nio.file.Files;
import java.nio.file.Path;
import org.graalvm.polyglot.Context;
import org.junit.Test;

public class HaraDbGraphTest {
  private static void assertHalFixturePasses(String path) throws Exception {
    String source = Files.readString(Path.of(path));
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      String result = context.eval(HaraLanguage.ID, source).asString();
      assertTrue(result, !result.contains(":pass false"));
    }
  }

  @Test
  public void schemaScopeCompilerFixturePasses() throws Exception {
    assertHalFixturePasses("lib/test/db/text/base_scope_test.hal");
  }

  @Test
  public void graphNormalizationFixturePasses() throws Exception {
    assertHalFixturePasses("lib/test/db/text/base_graph_test.hal");
  }

  @Test
  public void recursiveGraphSqlFixturePasses() throws Exception {
    assertHalFixturePasses("lib/test/db/text/sql_graph_test.hal");
  }

  @Test
  public void graphPlannerFixturePasses() throws Exception {
    assertHalFixturePasses("lib/test/db/text/base_tree_test.hal");
  }

  @Test
  public void plannedSqlFixturePasses() throws Exception {
    assertHalFixturePasses("lib/test/db/text/sql_tree_test.hal");
  }

  @Test
  public void viewCompatibilityFixturePasses() throws Exception {
    assertHalFixturePasses("lib/test/db/text/sql_view_test.hal");
  }

  @Test
  public void sqlFunctionCallFixturePasses() throws Exception {
    assertHalFixturePasses("lib/test/db/text/sql_call_test.hal");
  }
}
