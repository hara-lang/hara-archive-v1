package hara.truffle;

import static org.junit.Assert.assertTrue;

import java.nio.file.Files;
import java.nio.file.Path;
import org.graalvm.polyglot.Context;
import org.junit.Test;

public class HaraDbRuntimeTest {
  private static void assertHalFixturePasses(String path) throws Exception {
    String source = Files.readString(Path.of(path));
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      String result = context.eval(HaraLanguage.ID, source).asString();
      assertTrue(result, !result.contains(":pass false"));
    }
  }

  @Test
  public void databaseKernelClientAndProxyRuntimeFixturePasses() throws Exception {
    assertHalFixturePasses("lib/test/db/node/runtime_test.hal");
  }

  @Test
  public void databaseBatchAndTransactionFixturePasses() throws Exception {
    assertHalFixturePasses("lib/test/db/node/batch_test.hal");
  }

  @Test
  public void databaseSerializedExecutionFixturePasses() throws Exception {
    assertHalFixturePasses("lib/test/db/node/serial_test.hal");
  }

  @Test
  public void databaseDynamicServiceLifecycleFixturePasses() throws Exception {
    assertHalFixturePasses("lib/test/db/node/service_test.hal");
  }

  @Test
  public void databaseRuntimeStatusFixturePasses() throws Exception {
    assertHalFixturePasses("lib/test/db/node/status_test.hal");
  }

  @Test
  public void databaseSupabaseRuntimeFixturePasses() throws Exception {
    assertHalFixturePasses("lib/test/db/node/supabase_test.hal");
  }

  @Test
  public void databaseWorkerMessageTransportFixturePasses() throws Exception {
    assertHalFixturePasses("lib/test/db/node/transport_test.hal");
  }
}
