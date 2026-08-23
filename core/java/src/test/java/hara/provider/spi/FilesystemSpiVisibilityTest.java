package hara.provider.spi;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;

import hara.truffle.FilesystemException;
import hara.truffle.HaraLogicalPath;
import hara.truffle.IFilesystem;
import java.time.Duration;
import org.junit.Test;

/** Compile-time proof that an external provider can consume the trusted filesystem SPI. */
public class FilesystemSpiVisibilityTest {
  @Test
  public void providerCanUseCanonicalPathsCapabilitiesMutationsAndLifecycle() throws Exception {
    assertEquals("/alpha/beta", HaraLogicalPath.normalise("/alpha/./beta"));
    assertEquals("/alpha/gamma", HaraLogicalPath.join("/alpha", "gamma"));
    assertEquals("alpha", HaraLogicalPath.fileName("/alpha"));

    IFilesystem.Capabilities capabilities =
        IFilesystem.Capabilities.of(
            IFilesystem.Capability.READ, IFilesystem.Capability.REVISION_CHECK);
    assertTrue(capabilities.contains(IFilesystem.Capability.READ));
    assertEquals("revision-check", IFilesystem.Capability.REVISION_CHECK.keyword());
    assertEquals("directory", IFilesystem.EntryType.DIRECTORY.keyword());

    IFilesystem.PageRequest first = IFilesystem.PageRequest.first();
    assertEquals(IFilesystem.PageRequest.DEFAULT_LIMIT, first.limit());
    assertFalse(IFilesystem.MutationContext.none().required());
    assertEquals("/created", IFilesystem.Mutation.path("/created").path());

    IFilesystem.CallContext context =
        IFilesystem.CallContext.within(Duration.ofSeconds(1)).withTraceId("provider-fixture");
    assertTrue(context.hasDeadline());
    assertEquals("provider-fixture", context.traceId());
    assertTrue(context.remainingNanos() > 0L);
    try (AutoCloseable ignored = context.onCancel(() -> {})) {
      context.check("fixture", "read", "/alpha", null);
      assertTrue(context.cancel());
      assertTrue(context.cancelled());
    }

    FilesystemException cancelled =
        FilesystemException.cancelled("fixture", "read", "/alpha", null);
    assertEquals("cancelled", cancelled.code());
    assertEquals("file/cancelled", cancelled.data().get("ex/code"));
  }
}
