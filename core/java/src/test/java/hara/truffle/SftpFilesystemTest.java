package hara.truffle;

import static org.junit.Assert.assertArrayEquals;
import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;
import static org.junit.Assert.fail;

import java.nio.charset.StandardCharsets;
import java.util.List;
import java.util.Map;
import java.util.concurrent.CompletionException;
import java.util.concurrent.Executors;
import java.util.concurrent.ScheduledExecutorService;
import org.junit.Test;

public class SftpFilesystemTest {
  @Test
  public void openRequiresAuthenticatedVerifiedTransportAndRedactsAuthority() throws Exception {
    try (SftpFilesystemFixture fixture = new SftpFilesystemFixture()) {
      fixture.client.hostKeyVerified = false;
      try (var executors = new FixtureExecutors()) {
        SftpFilesystem.Factory factory = new SftpFilesystem.Factory();
        try {
          join(
              factory.open(
                  executors.context(reference -> fixture.client),
                  config("secret:ssh-profile")));
          fail("expected host-key verification failure");
        } catch (FilesystemException error) {
          assertEquals("authentication-failed", error.code());
          assertFalse(error.data().toString().contains("secret:ssh-profile"));
        }

        fixture.client.hostKeyVerified = true;
        IFilesystem filesystem =
            join(factory.open(executors.context(reference -> fixture.client), config("secret:ssh-profile")));
        assertEquals("sftp", filesystem.descriptor().kind());
        assertEquals("SFTP fixture", filesystem.descriptor().display());
        assertTrue(filesystem.descriptor().capabilities().contains(IFilesystem.Capability.READ));
        assertFalse(filesystem.descriptor().toString().contains("secret:ssh-profile"));
        assertFalse(filesystem.descriptor().toString().contains("/srv/application"));
        join(filesystem.close(IFilesystem.CallContext.create()));
      }
    }
  }

  @Test
  public void preservesBytesPagesMutationsAndRevisionChecks() throws Exception {
    try (SftpFilesystemFixture fixture = new SftpFilesystemFixture();
        var executors = new FixtureExecutors()) {
      IFilesystem filesystem =
          join(
              new SftpFilesystem.Factory()
                  .open(executors.context(reference -> fixture.client), config("sftp:test")));

      assertArrayEquals(
          "hello".getBytes(StandardCharsets.UTF_8),
          join(filesystem.read(IFilesystem.CallContext.create(), "/README.md")));

      IFilesystem.EntryPage first =
          join(
              filesystem.entriesPage(
                  IFilesystem.CallContext.create(),
                  "/data",
                  new IFilesystem.PageRequest(null, 1)));
      assertEquals(List.of("/data/a.bin"), first.entries().stream().map(IFilesystem.Entry::path).toList());
      assertEquals("1", first.nextToken());
      IFilesystem.EntryPage second =
          join(
              filesystem.entriesPage(
                  IFilesystem.CallContext.create(),
                  "/data",
                  new IFilesystem.PageRequest(first.nextToken(), 1)));
      assertEquals(List.of("/data/b.bin"), second.entries().stream().map(IFilesystem.Entry::path).toList());
      assertEquals(null, second.nextToken());

      IFilesystem.Mutation created =
          join(
              filesystem.write(
                  IFilesystem.CallContext.create(),
                  "/nested/value.bin",
                  new byte[] {0, 1, (byte) 255},
                  new IFilesystem.WriteOptions(IFilesystem.WriteMode.CREATE, true),
                  IFilesystem.MutationContext.none()));
      assertEquals("/nested/value.bin", created.path());
      assertArrayEquals(new byte[] {0, 1, (byte) 255}, fixture.client.bytes("/srv/application/nested/value.bin"));

      IFilesystem.Entry stat =
          join(filesystem.stat(IFilesystem.CallContext.create(), "/nested/value.bin"));
      join(
          filesystem.write(
              IFilesystem.CallContext.create(),
              "/nested/value.bin",
              new byte[] {9},
              new IFilesystem.WriteOptions(IFilesystem.WriteMode.REPLACE, false),
              new IFilesystem.MutationContext(stat.revision(), null)));
      try {
        join(
            filesystem.write(
                IFilesystem.CallContext.create(),
                "/nested/value.bin",
                new byte[] {8},
                new IFilesystem.WriteOptions(IFilesystem.WriteMode.REPLACE, false),
                new IFilesystem.MutationContext(stat.revision(), null)));
        fail("expected revision conflict");
      } catch (FilesystemException error) {
        assertEquals("conflict", error.code());
        assertEquals("revision-mismatch", error.providerCode());
      }
      try {
        join(
            filesystem.move(
                IFilesystem.CallContext.create(),
                "/README.md",
                "/README.md",
                new IFilesystem.MoveOptions(false, false, false),
                new IFilesystem.MutationContext("stale", null)));
        fail("expected same-path revision conflict");
      } catch (FilesystemException error) {
        assertEquals("conflict", error.code());
        assertEquals("revision-mismatch", error.providerCode());
      }

      join(
          filesystem.copy(
              IFilesystem.CallContext.create(),
              "/nested/value.bin",
              "/copy.bin",
              new IFilesystem.CopyOptions(false, false, false),
              IFilesystem.MutationContext.none()));
      assertArrayEquals(new byte[] {9}, fixture.client.bytes("/srv/application/copy.bin"));

      join(
          filesystem.move(
              IFilesystem.CallContext.create(),
              "/copy.bin",
              "/moved.bin",
              new IFilesystem.MoveOptions(false, false, false),
              IFilesystem.MutationContext.none()));
      assertFalse(fixture.client.exists("/srv/application/copy.bin"));
      assertTrue(fixture.client.exists("/srv/application/moved.bin"));
      join(filesystem.close(IFilesystem.CallContext.create()));
    }
  }

  @Test
  public void symlinksFailClosedAndUnsupportedSemanticsDoNotReachTransport() throws Exception {
    try (SftpFilesystemFixture fixture = new SftpFilesystemFixture();
        var executors = new FixtureExecutors()) {
      fixture.client.symlink("/srv/application/link");
      IFilesystem filesystem =
          join(
              new SftpFilesystem.Factory()
                  .open(executors.context(reference -> fixture.client), config("sftp:test")));

      try {
        join(filesystem.read(IFilesystem.CallContext.create(), "/link/secret.bin"));
        fail("expected ancestor symlink rejection");
      } catch (FilesystemException error) {
        assertEquals("outside-root", error.code());
      }
      try {
        join(filesystem.read(IFilesystem.CallContext.create(), "/link"));
        fail("expected final symlink rejection");
      } catch (FilesystemException error) {
        assertEquals("unsupported", error.code());
      }
      try {
        join(
            filesystem.move(
                IFilesystem.CallContext.create(),
                "/README.md",
                "/atomic.md",
                new IFilesystem.MoveOptions(false, false, true),
                IFilesystem.MutationContext.none()));
        fail("expected atomic move rejection");
      } catch (FilesystemException error) {
        assertEquals("unsupported", error.code());
      }
      try {
        join(
            filesystem.copy(
                IFilesystem.CallContext.create(),
                "/README.md",
                "/preserved.md",
                new IFilesystem.CopyOptions(false, false, true),
                IFilesystem.MutationContext.none()));
        fail("expected preserve-modified rejection");
      } catch (FilesystemException error) {
        assertEquals("unsupported", error.code());
      }
      assertFalse(fixture.client.exists("/srv/application/atomic.md"));
      assertFalse(fixture.client.exists("/srv/application/preserved.md"));
      join(filesystem.close(IFilesystem.CallContext.create()));
    }
  }

  private static Map<String, Object> config(String credentialReference) {
    return Map.of(
        "credential-ref", credentialReference,
        "root", "/srv/application",
        "display", "SFTP fixture",
        "operation-timeout-ms", 5_000,
        "max-transfer-bytes", 1024 * 1024);
  }

  private static <T> T join(java.util.concurrent.CompletionStage<T> stage) {
    try {
      return stage.toCompletableFuture().join();
    } catch (CompletionException error) {
      Throwable cause = error.getCause();
      if (cause instanceof RuntimeException runtime) throw runtime;
      throw error;
    }
  }

  private static final class FixtureExecutors implements AutoCloseable {
    private final java.util.concurrent.ExecutorService io = Executors.newCachedThreadPool();
    private final ScheduledExecutorService scheduler = Executors.newSingleThreadScheduledExecutor();

    IFilesystemFactory.OpenContext context(IFilesystemFactory.CredentialResolver credentials) {
      return new IFilesystemFactory.OpenContext(io, scheduler, credentials);
    }

    @Override
    public void close() {
      io.shutdownNow();
      scheduler.shutdownNow();
    }
  }
}
