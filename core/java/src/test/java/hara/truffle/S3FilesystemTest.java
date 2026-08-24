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

public class S3FilesystemTest {
  @Test
  public void authenticatedScopedMountRedactsBucketPrefixAndAuthority() throws Exception {
    try (S3FilesystemFixture fixture = new S3FilesystemFixture();
        FixtureExecutors executors = new FixtureExecutors()) {
      fixture.client.authenticated = false;
      try {
        join(
            new S3Filesystem.Factory()
                .open(executors.context(reference -> fixture.client), config("secret:s3")));
        fail("expected authentication failure");
      } catch (FilesystemException error) {
        assertEquals("authentication-failed", error.code());
        assertFalse(error.data().toString().contains("secret:s3"));
      }

      fixture.client.authenticated = true;
      IFilesystem filesystem =
          join(
              new S3Filesystem.Factory()
                  .open(executors.context(reference -> fixture.client), config("secret:s3")));
      assertEquals("s3", filesystem.descriptor().kind());
      assertEquals("Object fixture", filesystem.descriptor().display());
      assertTrue(filesystem.descriptor().capabilities().contains(IFilesystem.Capability.READ));
      assertTrue(filesystem.descriptor().capabilities().contains(IFilesystem.Capability.MOVE));
      assertFalse(filesystem.descriptor().capabilities().contains(IFilesystem.Capability.APPEND));
      assertFalse(filesystem.descriptor().capabilities().contains(IFilesystem.Capability.ATOMIC_MOVE));
      assertFalse(filesystem.descriptor().capabilities().contains(IFilesystem.Capability.MKDIR));
      assertFalse(filesystem.descriptor().toString().contains("secret:s3"));
      assertFalse(filesystem.descriptor().toString().contains(S3FilesystemFixture.BUCKET));
      assertFalse(filesystem.descriptor().toString().contains(S3FilesystemFixture.PREFIX));
      join(filesystem.close(IFilesystem.CallContext.create()));
    }
  }

  @Test
  public void virtualDirectoriesExactBytesPaginationAndNonAtomicMoveAreTruthful() throws Exception {
    try (S3FilesystemFixture fixture = new S3FilesystemFixture();
        FixtureExecutors executors = new FixtureExecutors()) {
      IFilesystem filesystem =
          join(
              new S3Filesystem.Factory()
                  .open(executors.context(reference -> fixture.client), config("s3:test")));

      assertEquals(
          IFilesystem.EntryType.DIRECTORY,
          join(filesystem.stat(IFilesystem.CallContext.create(), "/data")).type());
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
      IFilesystem.EntryPage second =
          join(
              filesystem.entriesPage(
                  IFilesystem.CallContext.create(),
                  "/data",
                  new IFilesystem.PageRequest(first.nextToken(), 1)));
      assertEquals(List.of("/data/b.bin"), second.entries().stream().map(IFilesystem.Entry::path).toList());

      IFilesystem.Mutation created =
          join(
              filesystem.write(
                  IFilesystem.CallContext.create(),
                  "/new.bin",
                  new byte[] {0, 1, (byte) 255},
                  new IFilesystem.WriteOptions(IFilesystem.WriteMode.CREATE, false),
                  IFilesystem.MutationContext.none()));
      assertEquals("/new.bin", created.path());
      assertArrayEquals(
          new byte[] {0, 1, (byte) 255},
          fixture.client.bytes(S3FilesystemFixture.PREFIX + "new.bin"));

      join(
          filesystem.copy(
              IFilesystem.CallContext.create(),
              "/new.bin",
              "/copy.bin",
              new IFilesystem.CopyOptions(false, false, false),
              IFilesystem.MutationContext.none()));
      assertArrayEquals(
          new byte[] {0, 1, (byte) 255},
          fixture.client.bytes(S3FilesystemFixture.PREFIX + "copy.bin"));

      join(
          filesystem.move(
              IFilesystem.CallContext.create(),
              "/copy.bin",
              "/moved.bin",
              new IFilesystem.MoveOptions(false, false, false),
              IFilesystem.MutationContext.none()));
      assertFalse(fixture.client.exists(S3FilesystemFixture.PREFIX + "copy.bin"));
      assertTrue(fixture.client.exists(S3FilesystemFixture.PREFIX + "moved.bin"));

      try {
        join(
            filesystem.move(
                IFilesystem.CallContext.create(),
                "/moved.bin",
                "/atomic.bin",
                new IFilesystem.MoveOptions(false, false, true),
                IFilesystem.MutationContext.none()));
        fail("expected atomic move rejection");
      } catch (FilesystemException error) {
        assertEquals("unsupported", error.code());
        assertEquals("atomic-move-unavailable", error.providerCode());
      }
      try {
        join(
            filesystem.mkdir(
                IFilesystem.CallContext.create(),
                "/empty",
                new IFilesystem.MkdirOptions(false, false),
                IFilesystem.MutationContext.none()));
        fail("expected virtual mkdir rejection");
      } catch (FilesystemException error) {
        assertEquals("unsupported", error.code());
      }
      join(filesystem.close(IFilesystem.CallContext.create()));
    }
  }

  @Test
  public void collisionsAndRevisionsFailClosed() throws Exception {
    try (S3FilesystemFixture fixture = new S3FilesystemFixture();
        FixtureExecutors executors = new FixtureExecutors()) {
      fixture.client.object(S3FilesystemFixture.PREFIX + "collision", new byte[] {1});
      fixture.client.object(S3FilesystemFixture.PREFIX + "collision/child.bin", new byte[] {2});
      IFilesystem filesystem =
          join(
              new S3Filesystem.Factory()
                  .open(executors.context(reference -> fixture.client), config("s3:test")));

      try {
        join(filesystem.stat(IFilesystem.CallContext.create(), "/collision"));
        fail("expected object/prefix ambiguity");
      } catch (FilesystemException error) {
        assertEquals("ambiguous-path", error.code());
        assertEquals("object-prefix-collision", error.providerCode());
      }

      IFilesystem.Entry readme =
          join(filesystem.stat(IFilesystem.CallContext.create(), "/README.md"));
      join(
          filesystem.write(
              IFilesystem.CallContext.create(),
              "/README.md",
              "updated".getBytes(StandardCharsets.UTF_8),
              new IFilesystem.WriteOptions(IFilesystem.WriteMode.REPLACE, false),
              new IFilesystem.MutationContext(readme.revision(), null)));
      try {
        join(
            filesystem.write(
                IFilesystem.CallContext.create(),
                "/README.md",
                "stale".getBytes(StandardCharsets.UTF_8),
                new IFilesystem.WriteOptions(IFilesystem.WriteMode.REPLACE, false),
                new IFilesystem.MutationContext(readme.revision(), null)));
        fail("expected revision conflict");
      } catch (FilesystemException error) {
        assertEquals("conflict", error.code());
      }
      try {
        join(
            filesystem.move(
                IFilesystem.CallContext.create(),
                "/README.md",
                "/README.md",
                new IFilesystem.MoveOptions(false, false, false),
                new IFilesystem.MutationContext(readme.revision(), null)));
        fail("expected same-path revision conflict");
      } catch (FilesystemException error) {
        assertEquals("conflict", error.code());
      }
      join(filesystem.close(IFilesystem.CallContext.create()));
    }
  }

  private static Map<String, Object> config(String credentialReference) {
    return Map.of(
        "credential-ref", credentialReference,
        "bucket", S3FilesystemFixture.BUCKET,
        "prefix", S3FilesystemFixture.PREFIX,
        "display", "Object fixture",
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
