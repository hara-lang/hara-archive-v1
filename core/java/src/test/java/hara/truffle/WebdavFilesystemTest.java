package hara.truffle;

import static org.junit.Assert.assertArrayEquals;
import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;
import static org.junit.Assert.fail;

import java.net.URI;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.CompletionException;
import java.util.concurrent.Executors;
import java.util.concurrent.ScheduledExecutorService;
import org.junit.Test;

public class WebdavFilesystemTest {
  @Test
  public void openRequiresAuthenticatedVerifiedTransportAndRedactsAuthority() throws Exception {
    try (WebdavFilesystemFixture fixture = new WebdavFilesystemFixture()) {
      fixture.setHostKeyVerified(false);
      try (var executors = new FixtureExecutors()) {
        WebdavFilesystem.Factory factory = new WebdavFilesystem.Factory();
        try {
          join(
              factory.open(
                  executors.context(reference -> fixture.client),
                  config("secret:dav-profile")));
          fail("expected host-key verification failure");
        } catch (FilesystemException error) {
          assertEquals("authentication-failed", error.code());
          assertFalse(error.data().toString().contains("secret:dav-profile"));
        }

        fixture.setHostKeyVerified(true);
        IFilesystem filesystem =
            join(factory.open(executors.context(reference -> fixture.client), config("secret:dav-profile")));
        assertEquals("webdav", filesystem.descriptor().kind());
        assertEquals("WebDAV fixture", filesystem.descriptor().display());
        assertTrue(filesystem.descriptor().capabilities().contains(IFilesystem.Capability.READ));
        assertFalse(filesystem.descriptor().toString().contains("secret:dav-profile"));
        assertFalse(filesystem.descriptor().toString().contains("dav.example.com"));
        join(filesystem.close(IFilesystem.CallContext.create()));
      }
    }
  }

  @Test
  public void readsWritesAndReportsStats() throws Exception {
    try (WebdavFilesystemFixture fixture = new WebdavFilesystemFixture();
        var executors = new FixtureExecutors()) {
      IFilesystem filesystem =
          join(
              new WebdavFilesystem.Factory()
                  .open(executors.context(reference -> fixture.client), config("dav:test")));

      assertArrayEquals(
          "hello".getBytes(StandardCharsets.UTF_8),
          join(filesystem.read(IFilesystem.CallContext.create(), "/README.md")));

      IFilesystem.Entry stat = join(filesystem.stat(IFilesystem.CallContext.create(), "/README.md"));
      assertEquals("README.md", stat.name());
      assertEquals(IFilesystem.EntryType.FILE, stat.type());
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

      IFilesystem.Mutation created =
          join(
              filesystem.write(
                  IFilesystem.CallContext.create(),
                  "/nested/value.bin",
                  new byte[] {0, 1, (byte) 255},
                  new IFilesystem.WriteOptions(IFilesystem.WriteMode.CREATE, true),
                  IFilesystem.MutationContext.none()));
      assertEquals("/nested/value.bin", created.path());
      assertArrayEquals(
          new byte[] {0, 1, (byte) 255},
          fixture.client.bytes("https://dav.example.com/workspace/nested/value.bin"));

      join(filesystem.close(IFilesystem.CallContext.create()));
    }
  }

  private static Map<String, Object> config(String credentialReference) {
    return Map.of(
        "credential-ref", credentialReference,
        "root-url", "https://dav.example.com/workspace",
        "display", "WebDAV fixture",
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

  private static final class WebdavFilesystemFixture implements AutoCloseable {
    private static final String ROOT = "https://dav.example.com/workspace";
    private final Map<String, byte[]> files = new HashMap<>();
    private final Set<String> directories = new java.util.HashSet<>();
    private final Client client = new Client();

    WebdavFilesystemFixture() {
      files.put("/README.md", "hello".getBytes(StandardCharsets.UTF_8));
      directories.add("/");
      directories.add("/data");
      files.put("/data/a.bin", new byte[] {1, 2, 3});
    }

    void setHostKeyVerified(boolean verified) {
      client.transportVerified = verified;
    }

    @Override
    public void close() {}

    private final class Client implements WebdavFilesystem.Client {
      boolean authenticated = true;
      boolean transportVerified = true;

      @Override
      public boolean authenticated() {
        return authenticated;
      }

      @Override
      public boolean transportVerified() {
        return transportVerified;
      }

      @Override
      public Set<IFilesystem.Capability> capabilities() {
        return Set.of(
            IFilesystem.Capability.READ,
            IFilesystem.Capability.WRITE,
            IFilesystem.Capability.ENTRIES,
            IFilesystem.Capability.MKDIR,
            IFilesystem.Capability.DELETE,
            IFilesystem.Capability.COPY,
            IFilesystem.Capability.MOVE,
            IFilesystem.Capability.APPEND,
            IFilesystem.Capability.REVISION_CHECK);
      }

      @Override
      public WebdavFilesystem.RemoteEntry lstat(String path) throws Exception {
        String logical = toLogical(path);
        if ("/".equals(logical)) {
          return new WebdavFilesystem.RemoteEntry(
              "workspace",
              IFilesystem.EntryType.DIRECTORY,
              null,
              null,
              "root",
              null,
              new IFilesystem.Capabilities(Set.of(IFilesystem.Capability.READ)),
              Map.of());
        }
        if (directories.contains(logical)) {
          return new WebdavFilesystem.RemoteEntry(
              logical.substring(logical.lastIndexOf('/') + 1),
              IFilesystem.EntryType.DIRECTORY,
              null,
              null,
              logical,
              "rev:" + logical,
              new IFilesystem.Capabilities(Set.of(IFilesystem.Capability.READ)),
              Map.of());
        }
        if (files.containsKey(logical)) {
          byte[] bytes = files.get(logical);
          return new WebdavFilesystem.RemoteEntry(
              logical.substring(logical.lastIndexOf('/') + 1),
              IFilesystem.EntryType.FILE,
              (long) bytes.length,
              null,
              logical,
              "rev:" + logical,
              new IFilesystem.Capabilities(Set.of(IFilesystem.Capability.READ)),
              Map.of());
        }
        throw new WebdavFilesystem.ClientFailure("not-found", "not-found", false);
      }

      @Override
      public byte[] read(String path, long maxBytes) throws Exception {
        String logical = toLogical(path);
        byte[] value = files.get(logical);
        if (value == null) throw new WebdavFilesystem.ClientFailure("not-found", "not-found", false);
        return value.clone();
      }

      @Override
      public void write(
          String path,
          byte[] bytes,
          IFilesystem.WriteMode mode,
          IFilesystem.MutationContext mutation) {
        String logical = toLogical(path);
        if (mode == IFilesystem.WriteMode.APPEND && files.containsKey(logical)) {
          byte[] existing = files.get(logical);
          byte[] combined = new byte[existing.length + bytes.length];
          System.arraycopy(existing, 0, combined, 0, existing.length);
          System.arraycopy(bytes, 0, combined, existing.length, bytes.length);
          files.put(logical, combined);
          return;
        }
        files.put(logical, bytes.clone());
      }

      @Override
      public List<WebdavFilesystem.RemoteEntry> entries(String path) {
        String logical = toLogical(path);
        List<WebdavFilesystem.RemoteEntry> values = new ArrayList<>();
        for (String child : new java.util.TreeSet<>(files.keySet())) {
          if (child.startsWith(logical.endsWith("/") ? logical : logical + "/")) {
            String relative = child.substring(logical.length()).replaceFirst("^/", "");
            if (relative.isEmpty()) continue;
            int slash = relative.indexOf('/');
            String name = slash >= 0 ? relative.substring(0, slash) : relative;
            String childPath = logical.equals("/") ? "/" + name : logical + "/" + name;
            if (!values.stream().anyMatch(v -> v.name().equals(name))) {
              values.add(new WebdavFilesystem.RemoteEntry(
                  name,
                  files.containsKey(childPath) ? IFilesystem.EntryType.FILE : IFilesystem.EntryType.DIRECTORY,
                  files.containsKey(childPath) ? (long) files.get(childPath).length : null,
                  null,
                  childPath,
                  "rev:" + childPath,
                  new IFilesystem.Capabilities(Set.of(IFilesystem.Capability.READ)),
                  Map.of()));
            }
          }
        }
        return values;
      }

      @Override
      public void mkdir(String path, IFilesystem.MutationContext mutation) {
        String logical = toLogical(path);
        directories.add(logical);
      }

      @Override
      public void delete(String path, boolean directory, IFilesystem.MutationContext mutation) {
        String logical = toLogical(path);
        if (directory) {
          directories.remove(logical);
        } else {
          files.remove(logical);
        }
      }

      @Override
      public void move(
          String source,
          String target,
          boolean replace,
          boolean atomic,
          IFilesystem.MutationContext mutation) {
        String sourceLogical = toLogical(source);
        String targetLogical = toLogical(target);
        if (files.containsKey(sourceLogical)) {
          byte[] value = files.remove(sourceLogical);
          files.put(targetLogical, value);
        } else {
          directories.remove(sourceLogical);
          directories.add(targetLogical);
        }
      }

      byte[] bytes(String url) {
        return files.get(toLogical(url));
      }

      private String toLogical(String value) {
        if (value == null) throw new IllegalArgumentException("missing path");
        String url = value;
        if (value.startsWith(ROOT)) {
          url = value.substring(ROOT.length());
        }
        if (value.startsWith("http://") || value.startsWith("https://")) {
          URI uri = URI.create(value);
          String path = uri.getPath();
          if (path == null || path.isBlank()) return "/";
          String normalized = path.startsWith("/") ? path : "/" + path;
          if (normalized.startsWith("/workspace")) {
            normalized = normalized.substring("/workspace".length());
          }
          if (normalized.isEmpty()) return "/";
          return normalized;
        }
        return url.startsWith("/") ? url : "/" + url;
      }

      @Override
      public void close() {}
    }
  }
}
