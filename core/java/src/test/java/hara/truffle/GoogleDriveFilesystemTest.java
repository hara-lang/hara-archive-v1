package hara.truffle;

import static org.junit.Assert.assertArrayEquals;
import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertNotEquals;
import static org.junit.Assert.assertTrue;
import static org.junit.Assert.fail;

import java.nio.charset.StandardCharsets;
import java.util.List;
import java.util.Map;
import java.util.concurrent.CompletionException;
import java.util.concurrent.Executors;
import java.util.concurrent.ScheduledExecutorService;
import org.junit.Test;

public class GoogleDriveFilesystemTest {
  @Test
  public void authenticatedMountRedactsAuthorityAndRejectsWorkspaceReads() throws Exception {
    try (GoogleDriveFilesystemFixture fixture = new GoogleDriveFilesystemFixture();
        var executors = new FixtureExecutors()) {
      fixture.client.authenticated = false;
      try {
        join(
            new GoogleDriveFilesystem.Factory()
                .open(executors.context(reference -> fixture.client), config(fixture, "secret:drive")));
        fail("expected authentication failure");
      } catch (FilesystemException error) {
        assertEquals("authentication-failed", error.code());
        assertFalse(error.data().toString().contains("secret:drive"));
      }

      fixture.client.authenticated = true;
      IFilesystem filesystem =
          join(
              new GoogleDriveFilesystem.Factory()
                  .open(executors.context(reference -> fixture.client), config(fixture, "secret:drive")));
      assertEquals("google-drive", filesystem.descriptor().kind());
      assertEquals("Drive fixture", filesystem.descriptor().display());
      assertFalse(filesystem.descriptor().toString().contains("secret:drive"));
      assertFalse(filesystem.descriptor().toString().contains(fixture.rootId));
      try {
        join(filesystem.read(IFilesystem.CallContext.create(), "/notes.gdoc"));
        fail("expected Workspace read rejection");
      } catch (FilesystemException error) {
        assertEquals("unsupported", error.code());
        assertEquals("workspace-export-unconfigured", error.providerCode());
      }
      try {
        join(filesystem.read(IFilesystem.CallContext.create(), "/shortcut"));
        fail("expected shortcut no-follow rejection");
      } catch (FilesystemException error) {
        assertEquals("unsupported", error.code());
      }
      join(filesystem.close(IFilesystem.CallContext.create()));
    }
  }

  @Test
  public void preservesStableIdsExactBytesPaginationAndMutations() throws Exception {
    try (GoogleDriveFilesystemFixture fixture = new GoogleDriveFilesystemFixture();
        var executors = new FixtureExecutors()) {
      IFilesystem filesystem =
          join(
              new GoogleDriveFilesystem.Factory()
                  .open(executors.context(reference -> fixture.client), config(fixture, "drive:test")));

      assertArrayEquals(
          "hello".getBytes(StandardCharsets.UTF_8),
          join(filesystem.read(IFilesystem.CallContext.create(), "/README.md")));
      IFilesystem.Entry readme =
          join(filesystem.stat(IFilesystem.CallContext.create(), "/README.md"));
      assertTrue(readme.id().startsWith("file-"));

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

      join(
          filesystem.write(
              IFilesystem.CallContext.create(),
              "/created/deep/value.bin",
              new byte[] {0, 1, (byte) 255},
              new IFilesystem.WriteOptions(IFilesystem.WriteMode.CREATE, true),
              IFilesystem.MutationContext.none()));
      GoogleDriveFilesystem.Item createdFolder = fixture.client.byName(fixture.rootId, "created");
      GoogleDriveFilesystem.Item deepFolder = fixture.client.byName(createdFolder.id(), "deep");
      GoogleDriveFilesystem.Item created = fixture.client.byName(deepFolder.id(), "value.bin");
      assertArrayEquals(new byte[] {0, 1, (byte) 255}, fixture.client.bytes(created.id()));

      IFilesystem.Entry beforeMove =
          join(filesystem.stat(IFilesystem.CallContext.create(), "/README.md"));
      IFilesystem.Mutation moved =
          join(
              filesystem.move(
                  IFilesystem.CallContext.create(),
                  "/README.md",
                  "/RENAMED.md",
                  new IFilesystem.MoveOptions(false, false, false),
                  new IFilesystem.MutationContext(beforeMove.revision(), null)));
      IFilesystem.Entry afterMove =
          join(filesystem.stat(IFilesystem.CallContext.create(), "/RENAMED.md"));
      assertEquals(beforeMove.id(), afterMove.id());
      assertNotEquals(beforeMove.revision(), afterMove.revision());
      assertEquals("/RENAMED.md", moved.path());

      join(
          filesystem.copy(
              IFilesystem.CallContext.create(),
              "/RENAMED.md",
              "/COPY.md",
              new IFilesystem.CopyOptions(false, false, false),
              IFilesystem.MutationContext.none()));
      assertArrayEquals(
          "hello".getBytes(StandardCharsets.UTF_8),
          join(filesystem.read(IFilesystem.CallContext.create(), "/COPY.md")));
      join(
          filesystem.delete(
              IFilesystem.CallContext.create(),
              "/COPY.md",
              new IFilesystem.DeleteOptions(false),
              IFilesystem.MutationContext.none()));
      try {
        join(filesystem.stat(IFilesystem.CallContext.create(), "/COPY.md"));
        fail("expected trashed file to disappear");
      } catch (FilesystemException error) {
        assertEquals("not-found", error.code());
      }
      join(filesystem.close(IFilesystem.CallContext.create()));
    }
  }

  @Test
  public void duplicateNamesFailClosedAndRevisionConflictsAreStable() throws Exception {
    try (GoogleDriveFilesystemFixture fixture = new GoogleDriveFilesystemFixture();
        var executors = new FixtureExecutors()) {
      fixture.client.file(fixture.rootId, "duplicate.bin", new byte[] {1});
      fixture.client.file(fixture.rootId, "duplicate.bin", new byte[] {2});
      IFilesystem filesystem =
          join(
              new GoogleDriveFilesystem.Factory()
                  .open(executors.context(reference -> fixture.client), config(fixture, "drive:test")));

      try {
        join(filesystem.stat(IFilesystem.CallContext.create(), "/duplicate.bin"));
        fail("expected ambiguous path");
      } catch (FilesystemException error) {
        assertEquals("ambiguous-path", error.code());
        assertEquals("duplicate-name", error.providerCode());
      }

      IFilesystem.Entry readme =
          join(filesystem.stat(IFilesystem.CallContext.create(), "/README.md"));
      join(
          filesystem.write(
              IFilesystem.CallContext.create(),
              "/README.md",
              "new".getBytes(StandardCharsets.UTF_8),
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
        fail("expected stale revision conflict");
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
      try {
        join(
            filesystem.mkdir(
                IFilesystem.CallContext.create(),
                "/data",
                new IFilesystem.MkdirOptions(false, true),
                new IFilesystem.MutationContext("stale", null)));
        fail("expected mkdir revision conflict");
      } catch (FilesystemException error) {
        assertEquals("conflict", error.code());
        assertEquals("revision-mismatch", error.providerCode());
      }
      join(filesystem.close(IFilesystem.CallContext.create()));
    }
  }

  private static Map<String, Object> config(
      GoogleDriveFilesystemFixture fixture, String credentialReference) {
    return Map.of(
        "credential-ref", credentialReference,
        "root-id", fixture.rootId,
        "display", "Drive fixture",
        "workspace-documents", "unsupported",
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
