package hara.truffle;

import java.util.ArrayList;
import java.util.Comparator;
import java.util.HashMap;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Objects;
import java.util.Set;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.CompletionException;
import java.util.concurrent.CompletionStage;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.Executor;
import java.util.concurrent.ScheduledExecutorService;
import java.util.concurrent.ScheduledFuture;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.function.Supplier;

/** Provider-neutral SFTP mount over a trusted authenticated transport capability. */
final class SftpFilesystem implements IFilesystem {
  /**
   * Trusted host transport. Authentication and host-key verification happen before Hara receives
   * this capability; filesystem semantics remain owned here.
   */
  interface Client extends AutoCloseable {
    boolean authenticated();

    boolean hostKeyVerified();

    Set<Capability> capabilities();

    RemoteEntry lstat(String path) throws Exception;

    byte[] read(String path, long maxBytes) throws Exception;

    void write(
        String path,
        byte[] bytes,
        WriteMode mode,
        MutationContext mutation)
        throws Exception;

    List<RemoteEntry> entries(String path) throws Exception;

    void mkdir(String path, MutationContext mutation) throws Exception;

    void delete(String path, boolean directory, MutationContext mutation) throws Exception;

    void move(
        String source,
        String target,
        boolean replace,
        boolean atomic,
        MutationContext mutation)
        throws Exception;

    @Override
    void close() throws Exception;
  }

  record RemoteEntry(
      String name,
      EntryType type,
      Long size,
      Long modifiedAt,
      String id,
      String revision,
      Capabilities capabilities,
      Map<String, Object> extensions) {
    RemoteEntry {
      if (name == null || name.isBlank() || name.contains("/") || ".".equals(name) || "..".equals(name)) {
        throw new IllegalArgumentException("invalid SFTP entry name");
      }
      type = Objects.requireNonNull(type, "SFTP entry type");
      if (size != null && size < 0) throw new IllegalArgumentException("negative SFTP entry size");
      extensions = extensions == null || extensions.isEmpty() ? Map.of() : Map.copyOf(extensions);
    }
  }

  /** Typed transport failure. Portable behavior never depends on server message strings. */
  static final class ClientFailure extends Exception {
    private static final long serialVersionUID = 1L;
    private final String code;
    private final String providerCode;
    private final boolean retryable;

    ClientFailure(String code, String providerCode, boolean retryable) {
      super("SFTP transport operation failed");
      this.code = requireCode(code);
      this.providerCode = providerCode;
      this.retryable = retryable;
    }

    String code() {
      return code;
    }

    String providerCode() {
      return providerCode;
    }

    boolean retryable() {
      return retryable;
    }

    private static String requireCode(String value) {
      if (value == null || !value.matches("[a-z][a-z0-9-]*")) {
        throw new IllegalArgumentException("invalid SFTP failure code");
      }
      return value;
    }
  }

  static final class Factory implements IFilesystemFactory {
    private static final Set<String> ALLOWED =
        Set.of(
            "credential-ref",
            "root",
            "read-only?",
            "display",
            "operation-timeout-ms",
            "max-transfer-bytes");

    @Override
    public String kind() {
      return "sftp";
    }

    @Override
    public void validate(Map<String, ?> configuration) {
      IFilesystemFactory.super.validate(configuration);
      for (String key : configuration.keySet()) {
        if (!ALLOWED.contains(key)) {
          throw new IllegalArgumentException("unknown SFTP filesystem option " + key);
        }
      }
      requireText(configuration, "credential-ref");
      remoteRoot(requireText(configuration, "root"));
      Object readOnly = configuration.get("read-only?");
      if (readOnly != null && !(readOnly instanceof Boolean)) {
        throw new IllegalArgumentException("SFTP filesystem read-only? must be a boolean");
      }
      Object display = configuration.get("display");
      if (display != null && (!(display instanceof String text) || text.isBlank())) {
        throw new IllegalArgumentException("SFTP filesystem display must be a nonblank string");
      }
      positiveLong(configuration, "operation-timeout-ms", 30_000L);
      positiveLong(configuration, "max-transfer-bytes", 16L * 1024L * 1024L);
    }

    @Override
    public CompletionStage<IFilesystem> open(OpenContext context, Map<String, ?> configuration) {
      Objects.requireNonNull(context, "filesystem open context");
      validate(configuration);
      String credentialReference = (String) configuration.get("credential-ref");
      Object resolved = context.credentials().resolve(credentialReference);
      if (!(resolved instanceof Client client)) {
        return CompletableFuture.failedFuture(
            new IllegalArgumentException(
                "SFTP credential reference did not resolve to a trusted SFTP client"));
      }
      if (!client.authenticated() || !client.hostKeyVerified()) {
        return CompletableFuture.failedFuture(
            failure(
                "authentication-failed",
                "SFTP transport is not authenticated with a verified host key",
                "open",
                null,
                null,
                "transport-unverified",
                false,
                null));
      }
      String root = remoteRoot((String) configuration.get("root"));
      boolean readOnly = Boolean.TRUE.equals(configuration.get("read-only?"));
      String display =
          configuration.get("display") instanceof String value ? value : "SFTP filesystem";
      long operationTimeoutMillis =
          positiveLong(configuration, "operation-timeout-ms", 30_000L);
      long maxTransferBytes =
          positiveLong(configuration, "max-transfer-bytes", 16L * 1024L * 1024L);
      SftpFilesystem filesystem =
          new SftpFilesystem(
              client,
              root,
              display,
              readOnly,
              operationTimeoutMillis,
              maxTransferBytes,
              context.ioExecutor(),
              context.scheduler());
      return filesystem
          .submit(
              IFilesystem.CallContext.create(),
              "open",
              "/",
              null,
              () -> {
                RemoteEntry entry = filesystem.client.lstat(root);
                if (entry.type() == EntryType.SYMLINK) {
                  throw failure(
                      "outside-root",
                      "SFTP root cannot be a symbolic link",
                      "open",
                      "/",
                      null,
                      "root-symlink",
                      false,
                      null);
                }
                if (entry.type() != EntryType.DIRECTORY) {
                  throw failure(
                      "not-directory",
                      "SFTP root is not a directory",
                      "open",
                      "/",
                      null,
                      "root-not-directory",
                      false,
                      null);
                }
                return (IFilesystem) filesystem;
              });
    }

    private static String requireText(Map<String, ?> values, String key) {
      Object value = values.get(key);
      if (!(value instanceof String text) || text.isBlank()) {
        throw new IllegalArgumentException("SFTP filesystem " + key + " is required");
      }
      return text;
    }

    private static long positiveLong(Map<String, ?> values, String key, long fallback) {
      Object value = values.get(key);
      if (value == null) return fallback;
      if (!(value instanceof Number number) || number.longValue() <= 0) {
        throw new IllegalArgumentException("SFTP filesystem " + key + " must be positive");
      }
      return number.longValue();
    }
  }

  private record Pending(CompletableFuture<?> future, String operation, String path, String target) {}

  private final Client client;
  private final String root;
  private final String display;
  private final boolean readOnly;
  private final long operationTimeoutMillis;
  private final long maxTransferBytes;
  private final Executor ioExecutor;
  private final ScheduledExecutorService scheduler;
  private final Set<Capability> transportCapabilities;
  private final Capabilities capabilities;
  private final AtomicBoolean closed = new AtomicBoolean();
  private final Set<Pending> pending = ConcurrentHashMap.newKeySet();

  SftpFilesystem(
      Client client,
      String root,
      String display,
      boolean readOnly,
      long operationTimeoutMillis,
      long maxTransferBytes,
      Executor ioExecutor,
      ScheduledExecutorService scheduler) {
    this.client = Objects.requireNonNull(client, "SFTP client");
    this.root = remoteRoot(root);
    this.display = Objects.requireNonNull(display, "SFTP display");
    this.readOnly = readOnly;
    this.operationTimeoutMillis = operationTimeoutMillis;
    this.maxTransferBytes = maxTransferBytes;
    this.ioExecutor = Objects.requireNonNull(ioExecutor, "filesystem I/O executor");
    this.scheduler = Objects.requireNonNull(scheduler, "filesystem scheduler");
    this.transportCapabilities = Set.copyOf(client.capabilities());
    this.capabilities = new Capabilities(advertisedCapabilities());
  }

  @Override
  public Descriptor descriptor() {
    return new Descriptor(
        "sftp",
        display,
        readOnly,
        capabilities,
        null,
        Map.of("provider/root-scoped?", true, "provider/host-key-verified?", true));
  }

  @Override
  public CompletionStage<Entry> stat(CallContext context, String path) {
    String logical = normalise(path);
    return submit(
        context,
        "stat",
        logical,
        null,
        () -> {
          require(Capability.READ, "stat", logical, null);
          guardAncestors(logical, false, "stat", null);
          return entry(logical, client.lstat(remote(logical)));
        });
  }

  @Override
  public CompletionStage<byte[]> read(CallContext context, String path) {
    String logical = normalise(path);
    return submit(
        context,
        "read",
        logical,
        null,
        () -> {
          require(Capability.READ, "read", logical, null);
          guardAncestors(logical, false, "read", null);
          RemoteEntry value = client.lstat(remote(logical));
          requireRegular(value, "read", logical, null);
          if (value.size() != null && value.size() > maxTransferBytes) {
            throw failure(
                "quota-exceeded",
                "SFTP file exceeds configured transfer limit",
                "read",
                logical,
                null,
                "transfer-limit",
                false,
                null);
          }
          byte[] bytes = client.read(remote(logical), maxTransferBytes);
          if (bytes.length > maxTransferBytes) {
            throw failure(
                "quota-exceeded",
                "SFTP file exceeds configured transfer limit",
                "read",
                logical,
                null,
                "transfer-limit",
                false,
                null);
          }
          return bytes.clone();
        });
  }

  @Override
  public CompletionStage<Mutation> write(
      CallContext context,
      String path,
      byte[] bytes,
      WriteOptions options,
      MutationContext mutation) {
    String logical = normalise(path);
    byte[] copy = Objects.requireNonNull(bytes, "filesystem bytes").clone();
    Objects.requireNonNull(options, "write options");
    Objects.requireNonNull(mutation, "mutation context");
    return submit(
        context,
        "write",
        logical,
        null,
        () -> {
          requireWritable(Capability.WRITE, "write", logical, null);
          requireRevisionSupport(mutation, "write", logical, null);
          if (options.mode() == WriteMode.APPEND) {
            require(Capability.APPEND, "write", logical, null);
          }
          if (copy.length > maxTransferBytes) {
            throw failure(
                "quota-exceeded",
                "SFTP write exceeds configured transfer limit",
                "write",
                logical,
                null,
                "transfer-limit",
                false,
                null);
          }
          ensureParents(logical, options.parents(), "write");
          RemoteEntry existing = optionalLstat(remote(logical), "write", logical, null);
          if (existing != null) {
            if (existing.type() == EntryType.SYMLINK) {
              throw unsupported("write", logical, null, "symlink-write");
            }
            if (existing.type() == EntryType.DIRECTORY) {
              throw failure(
                  "is-directory", "path is a directory", "write", logical, null, null, false, null);
            }
          }
          client.write(remote(logical), copy, options.mode(), mutation);
          return mutation(logical, client.lstat(remote(logical)));
        });
  }

  @Override
  public CompletionStage<EntryPage> entriesPage(
      CallContext context, String path, PageRequest request) {
    String logical = normalise(path);
    Objects.requireNonNull(request, "filesystem page request");
    return submit(
        context,
        "entries",
        logical,
        null,
        () -> {
          require(Capability.ENTRIES, "entries", logical, null);
          guardAncestors(logical, false, "entries", null);
          RemoteEntry directory = client.lstat(remote(logical));
          if (directory.type() == EntryType.SYMLINK || directory.type() != EntryType.DIRECTORY) {
            throw failure(
                "not-directory",
                "path is not a directory",
                "entries",
                logical,
                null,
                null,
                false,
                null);
          }
          ArrayList<Entry> entries = new ArrayList<>();
          for (RemoteEntry child : client.entries(remote(logical))) {
            entries.add(entry(HaraLogicalPath.join(logical, child.name()), child));
          }
          entries.sort(Comparator.comparing(Entry::path));
          int offset = pageOffset(request.token(), entries.size());
          int end = Math.min(entries.size(), offset + request.limit());
          String next = end < entries.size() ? Integer.toString(end) : null;
          return new EntryPage(entries.subList(offset, end), next);
        });
  }

  @Override
  public CompletionStage<Mutation> mkdir(
      CallContext context, String path, MkdirOptions options, MutationContext mutation) {
    String logical = normalise(path);
    Objects.requireNonNull(options, "mkdir options");
    Objects.requireNonNull(mutation, "mutation context");
    return submit(
        context,
        "mkdir",
        logical,
        null,
        () -> {
          requireWritable(Capability.MKDIR, "mkdir", logical, null);
          requireRevisionSupport(mutation, "mkdir", logical, null);
          if ("/".equals(logical)) {
            if (options.existsOk()) return Mutation.path("/");
            throw failure(
                "already-exists", "mounted root already exists", "mkdir", logical, null, null, false, null);
          }
          RemoteEntry existing = optionalLstat(remote(logical), "mkdir", logical, null);
          if (existing != null) {
            if (existing.type() == EntryType.DIRECTORY && options.existsOk()) {
              return mutation(logical, existing);
            }
            throw failure(
                "already-exists", "path already exists", "mkdir", logical, null, null, false, null);
          }
          ensureParents(logical, options.parents(), "mkdir");
          client.mkdir(remote(logical), mutation);
          return mutation(logical, client.lstat(remote(logical)));
        });
  }

  @Override
  public CompletionStage<Mutation> delete(
      CallContext context, String path, DeleteOptions options, MutationContext mutation) {
    String logical = normalise(path);
    Objects.requireNonNull(options, "delete options");
    Objects.requireNonNull(mutation, "mutation context");
    return submit(
        context,
        "delete",
        logical,
        null,
        () -> {
          requireWritable(Capability.DELETE, "delete", logical, null);
          requireRevisionSupport(mutation, "delete", logical, null);
          if ("/".equals(logical)) {
            throw failure("denied", "cannot delete mounted root", "delete", logical, null, null, false, null);
          }
          guardAncestors(logical, false, "delete", null);
          RemoteEntry existing = optionalLstat(remote(logical), "delete", logical, null);
          if (existing == null) {
            if (options.missingOk()) return Mutation.path(logical);
            throw failure("not-found", "path does not exist", "delete", logical, null, null, false, null);
          }
          if (existing.type() == EntryType.SYMLINK) {
            throw unsupported("delete", logical, null, "symlink-delete");
          }
          client.delete(remote(logical), existing.type() == EntryType.DIRECTORY, mutation);
          return Mutation.path(logical);
        });
  }

  @Override
  public CompletionStage<Mutation> copy(
      CallContext context,
      String source,
      String target,
      CopyOptions options,
      MutationContext mutation) {
    String sourceLogical = normalise(source);
    String targetLogical = normalise(target);
    Objects.requireNonNull(options, "copy options");
    Objects.requireNonNull(mutation, "mutation context");
    return submit(
        context,
        "copy",
        sourceLogical,
        targetLogical,
        () -> {
          requireWritable(Capability.WRITE, "copy", sourceLogical, targetLogical);
          require(Capability.READ, "copy", sourceLogical, targetLogical);
          requireRevisionSupport(mutation, "copy", sourceLogical, targetLogical);
          if (options.preserveModified()) {
            require(Capability.PRESERVE_MODIFIED, "copy", sourceLogical, targetLogical);
          }
          if (sourceLogical.equals(targetLogical)) {
            throw failure(
                "already-exists",
                "source and target are the same path",
                "copy",
                sourceLogical,
                targetLogical,
                null,
                false,
                null);
          }
          guardAncestors(sourceLogical, false, "copy", targetLogical);
          RemoteEntry sourceEntry = client.lstat(remote(sourceLogical));
          requireRegular(sourceEntry, "copy", sourceLogical, targetLogical);
          ensureParents(targetLogical, options.parents(), "copy");
          RemoteEntry targetEntry = optionalLstat(remote(targetLogical), "copy", sourceLogical, targetLogical);
          if (targetEntry != null) {
            if (!options.replace()) {
              throw failure(
                  "already-exists",
                  "target already exists",
                  "copy",
                  sourceLogical,
                  targetLogical,
                  null,
                  false,
                  null);
            }
            if (targetEntry.type() == EntryType.DIRECTORY || targetEntry.type() == EntryType.SYMLINK) {
              throw unsupported("copy", sourceLogical, targetLogical, "target-type");
            }
          }
          byte[] bytes = client.read(remote(sourceLogical), maxTransferBytes);
          if (bytes.length > maxTransferBytes) {
            throw failure(
                "quota-exceeded",
                "SFTP file exceeds configured transfer limit",
                "copy",
                sourceLogical,
                targetLogical,
                "transfer-limit",
                false,
                null);
          }
          client.write(
              remote(targetLogical),
              bytes,
              targetEntry == null ? WriteMode.CREATE : WriteMode.REPLACE,
              mutation);
          return mutation(targetLogical, client.lstat(remote(targetLogical)));
        });
  }

  @Override
  public CompletionStage<Mutation> move(
      CallContext context,
      String source,
      String target,
      MoveOptions options,
      MutationContext mutation) {
    String sourceLogical = normalise(source);
    String targetLogical = normalise(target);
    Objects.requireNonNull(options, "move options");
    Objects.requireNonNull(mutation, "mutation context");
    return submit(
        context,
        "move",
        sourceLogical,
        targetLogical,
        () -> {
          requireWritable(Capability.MOVE, "move", sourceLogical, targetLogical);
          requireRevisionSupport(mutation, "move", sourceLogical, targetLogical);
          if ("/".equals(sourceLogical) || "/".equals(targetLogical)) {
            throw failure(
                "denied", "cannot move mounted root", "move", sourceLogical, targetLogical, null, false, null);
          }
          if (sourceLogical.equals(targetLogical)) {
            RemoteEntry same = client.lstat(remote(sourceLogical));
            checkExpected(same, mutation.expectedRevision(), "move", sourceLogical, targetLogical);
            checkExpected(same, mutation.expectedTargetRevision(), "move", sourceLogical, targetLogical);
            return mutation(sourceLogical, same);
          }
          if (targetLogical.startsWith(sourceLogical + "/")) {
            throw failure(
                "invalid-path",
                "cannot move a directory beneath itself",
                "move",
                sourceLogical,
                targetLogical,
                null,
                false,
                null);
          }
          if (options.atomic()) {
            require(Capability.ATOMIC_MOVE, "move", sourceLogical, targetLogical);
          }
          guardAncestors(sourceLogical, false, "move", targetLogical);
          RemoteEntry sourceEntry = client.lstat(remote(sourceLogical));
          if (sourceEntry.type() == EntryType.SYMLINK) {
            throw unsupported("move", sourceLogical, targetLogical, "symlink-move");
          }
          ensureParents(targetLogical, options.parents(), "move");
          RemoteEntry targetEntry = optionalLstat(remote(targetLogical), "move", sourceLogical, targetLogical);
          if (targetEntry != null && !options.replace()) {
            throw failure(
                "already-exists",
                "target already exists",
                "move",
                sourceLogical,
                targetLogical,
                null,
                false,
                null);
          }
          if (targetEntry != null && targetEntry.type() == EntryType.SYMLINK) {
            throw unsupported("move", sourceLogical, targetLogical, "target-symlink");
          }
          client.move(
              remote(sourceLogical), remote(targetLogical), options.replace(), options.atomic(), mutation);
          return mutation(targetLogical, client.lstat(remote(targetLogical)));
        });
  }

  @Override
  public CompletionStage<Void> close(CallContext context) {
    Objects.requireNonNull(context, "filesystem call context");
    if (!closed.compareAndSet(false, true)) return CompletableFuture.completedFuture(null);
    for (Pending operation : pending) {
      operation.future().completeExceptionally(
          FilesystemException.providerClosed(
              "sftp", operation.operation(), operation.path(), operation.target()));
    }
    pending.clear();
    CompletableFuture<Void> result = new CompletableFuture<>();
    ioExecutor.execute(
        () -> {
          try {
            client.close();
            result.complete(null);
          } catch (Throwable error) {
            result.completeExceptionally(mapFailure(error, "close", null, null));
          }
        });
    return result;
  }

  private Set<Capability> advertisedCapabilities() {
    HashSet<Capability> output = new HashSet<>();
    if (transportCapabilities.contains(Capability.READ)) output.add(Capability.READ);
    if (transportCapabilities.contains(Capability.ENTRIES)) output.add(Capability.ENTRIES);
    if (!readOnly) {
      for (Capability value :
          List.of(
              Capability.WRITE,
              Capability.MKDIR,
              Capability.DELETE,
              Capability.MOVE,
              Capability.APPEND,
              Capability.ATOMIC_MOVE,
              Capability.PRESERVE_MODIFIED,
              Capability.REVISION_CHECK)) {
        if (transportCapabilities.contains(value)) output.add(value);
      }
      if (output.contains(Capability.READ) && output.contains(Capability.WRITE)) {
        output.add(Capability.COPY);
      }
    }
    return Set.copyOf(output);
  }

  private void guardAncestors(
      String logical, boolean allowMissing, String operation, String target) throws Exception {
    List<String> segments = logicalSegments(logical);
    String current = root;
    for (int index = 0; index + 1 < segments.size(); index++) {
      current = remoteJoin(current, segments.get(index));
      RemoteEntry value;
      try {
        value = client.lstat(current);
      } catch (ClientFailure error) {
        if (allowMissing && "not-found".equals(error.code())) return;
        throw error;
      }
      if (value.type() == EntryType.SYMLINK) {
        throw failure(
            "outside-root",
            "path traverses a symbolic link",
            operation,
            logical,
            target,
            "ancestor-symlink",
            false,
            null);
      }
      if (value.type() != EntryType.DIRECTORY) {
        throw failure(
            "not-directory",
            "path ancestor is not a directory",
            operation,
            logical,
            target,
            null,
            false,
            null);
      }
    }
  }

  private void ensureParents(String logical, boolean parents, String operation) throws Exception {
    List<String> segments = logicalSegments(logical);
    String current = root;
    for (int index = 0; index + 1 < segments.size(); index++) {
      current = remoteJoin(current, segments.get(index));
      RemoteEntry value;
      try {
        value = client.lstat(current);
      } catch (ClientFailure error) {
        if (!"not-found".equals(error.code())) throw error;
        if (!parents) {
          throw failure(
              "not-found",
              "parent directory does not exist",
              operation,
              logical,
              null,
              error.providerCode(),
              false,
              error);
        }
        requireWritable(Capability.MKDIR, operation, logical, null);
        client.mkdir(current, MutationContext.none());
        value = client.lstat(current);
      }
      if (value.type() == EntryType.SYMLINK) {
        throw failure(
            "outside-root",
            "path traverses a symbolic link",
            operation,
            logical,
            null,
            "ancestor-symlink",
            false,
            null);
      }
      if (value.type() != EntryType.DIRECTORY) {
        throw failure(
            "not-directory",
            "path ancestor is not a directory",
            operation,
            logical,
            null,
            null,
            false,
            null);
      }
    }
  }

  private RemoteEntry optionalLstat(
      String remote, String operation, String path, String target) throws Exception {
    try {
      return client.lstat(remote);
    } catch (ClientFailure error) {
      if ("not-found".equals(error.code())) return null;
      throw error;
    }
  }

  private void requireRegular(RemoteEntry value, String operation, String path, String target) {
    if (value.type() == EntryType.SYMLINK) {
      throw unsupported(operation, path, target, "symlink");
    }
    if (value.type() == EntryType.DIRECTORY) {
      throw failure(
          "is-directory", "path is a directory", operation, path, target, null, false, null);
    }
    if (value.type() != EntryType.FILE) {
      throw unsupported(operation, path, target, "non-regular-file");
    }
  }

  private Entry entry(String logical, RemoteEntry remoteEntry) {
    LinkedHashMap<String, Object> extensions = new LinkedHashMap<>(remoteEntry.extensions());
    return new Entry(
        logical,
        "/".equals(logical) ? "" : HaraLogicalPath.fileName(logical),
        remoteEntry.type(),
        remoteEntry.size(),
        remoteEntry.modifiedAt(),
        remoteEntry.id(),
        remoteEntry.revision(),
        remoteEntry.capabilities(),
        extensions);
  }

  private Mutation mutation(String logical, RemoteEntry value) {
    return new Mutation(logical, value.revision(), null, Map.of());
  }

  private void require(Capability capability, String operation, String path, String target) {
    if (!capabilities.contains(capability)) {
      throw unsupported(operation, path, target, capability.keyword() + "-unavailable");
    }
  }

  private void requireWritable(
      Capability capability, String operation, String path, String target) {
    if (readOnly) {
      throw failure(
          "permission-denied",
          "SFTP mount is read-only",
          operation,
          path,
          target,
          "read-only",
          false,
          null);
    }
    require(capability, operation, path, target);
  }

  private void requireRevisionSupport(
      MutationContext mutation, String operation, String path, String target) {
    if (mutation.required() && !capabilities.contains(Capability.REVISION_CHECK)) {
      throw FilesystemException.unsupportedRevision("sftp", operation, path, target);
    }
  }

  private static void checkExpected(
      RemoteEntry entry, String expected, String operation, String path, String target) {
    if (expected == null) return;
    if (entry.revision() == null || !expected.equals(entry.revision())) {
      throw failure(
          "conflict",
          "SFTP entry revision does not match",
          operation,
          path,
          target,
          "revision-mismatch",
          false,
          null);
    }
  }

  private <T> CompletionStage<T> submit(
      CallContext context,
      String operation,
      String path,
      String target,
      SupplierWithException<T> operationBody) {
    Objects.requireNonNull(context, "filesystem call context");
    if (closed.get()) {
      return CompletableFuture.failedFuture(
          FilesystemException.providerClosed("sftp", operation, path, target));
    }
    try {
      context.check("sftp", operation, path, target);
    } catch (RuntimeException error) {
      return CompletableFuture.failedFuture(error);
    }

    CompletableFuture<T> result = new CompletableFuture<>();
    Pending tracked = new Pending(result, operation, path, target);
    pending.add(tracked);
    long timeoutNanos = TimeUnit.MILLISECONDS.toNanos(operationTimeoutMillis);
    if (context.hasDeadline()) timeoutNanos = Math.min(timeoutNanos, context.remainingNanos());
    long delay = Math.max(0L, timeoutNanos);
    ScheduledFuture<?> timeout =
        scheduler.schedule(
            () ->
                result.completeExceptionally(
                    FilesystemException.timeout("sftp", operation, path, target)),
            delay,
            TimeUnit.NANOSECONDS);
    AutoCloseable cancellation =
        context.onCancel(
            () ->
                result.completeExceptionally(
                    FilesystemException.cancelled("sftp", operation, path, target)));
    result.whenComplete(
        (ignored, error) -> {
          timeout.cancel(false);
          pending.remove(tracked);
          try {
            cancellation.close();
          } catch (Exception ignoredClose) {
            // Cancellation hook removal is best-effort after settlement.
          }
        });
    ioExecutor.execute(
        () -> {
          if (result.isDone()) return;
          try {
            context.check("sftp", operation, path, target);
            T value = operationBody.get();
            result.complete(value);
          } catch (Throwable error) {
            result.completeExceptionally(mapFailure(error, operation, path, target));
          }
        });
    return result;
  }

  private static FilesystemException mapFailure(
      Throwable error, String operation, String path, String target) {
    Throwable current = unwrap(error);
    if (current instanceof FilesystemException filesystem) return filesystem;
    if (current instanceof ClientFailure failure) {
      return failure(
          failure.code(),
          "SFTP transport operation failed",
          operation,
          path,
          target,
          failure.providerCode(),
          failure.retryable(),
          failure);
    }
    if (current instanceof HaraLogicalPath.Error logical) {
      return failure(
          logical.code(), logical.getMessage(), operation, path, target, null, false, logical);
    }
    return failure(
        "io",
        "SFTP filesystem operation failed",
        operation,
        path,
        target,
        "transport-error",
        true,
        current);
  }

  private static Throwable unwrap(Throwable error) {
    Throwable current = error;
    while ((current instanceof CompletionException
            || current instanceof java.util.concurrent.ExecutionException)
        && current.getCause() != null) {
      current = current.getCause();
    }
    return current;
  }

  private static FilesystemException unsupported(
      String operation, String path, String target, String providerCode) {
    return failure(
        "unsupported",
        "SFTP provider does not support the requested operation",
        operation,
        path,
        target,
        providerCode,
        false,
        null);
  }

  private static FilesystemException failure(
      String code,
      String message,
      String operation,
      String path,
      String target,
      String providerCode,
      boolean retryable,
      Throwable cause) {
    return new FilesystemException(
        code, message, "sftp", operation, path, target, providerCode, retryable, cause);
  }

  private static String normalise(String path) {
    return HaraLogicalPath.normalise(path);
  }

  private String remote(String logical) {
    String normal = normalise(logical);
    if ("/".equals(normal)) return root;
    return "/".equals(root) ? normal : root + normal;
  }

  private static List<String> logicalSegments(String logical) {
    String normal = normalise(logical);
    if ("/".equals(normal)) return List.of();
    return List.of(normal.substring(1).split("/"));
  }

  private static String remoteRoot(String value) {
    if (value == null || value.isBlank() || !value.startsWith("/")) {
      throw new IllegalArgumentException("SFTP root must be an absolute POSIX path");
    }
    if (value.indexOf('\0') >= 0 || value.indexOf('\\') >= 0) {
      throw new IllegalArgumentException("SFTP root contains an invalid character");
    }
    ArrayList<String> segments = new ArrayList<>();
    for (String segment : value.split("/+")) {
      if (segment.isEmpty()) continue;
      if (".".equals(segment) || "..".equals(segment)) {
        throw new IllegalArgumentException("SFTP root cannot contain dot segments");
      }
      segments.add(segment);
    }
    return segments.isEmpty() ? "/" : "/" + String.join("/", segments);
  }

  private static String remoteJoin(String parent, String child) {
    return "/".equals(parent) ? "/" + child : parent + "/" + child;
  }

  private static int pageOffset(String token, int size) {
    if (token == null) return 0;
    try {
      int offset = Integer.parseInt(token);
      if (offset < 0 || offset > size) throw new NumberFormatException();
      return offset;
    } catch (NumberFormatException error) {
      throw new IllegalArgumentException("invalid SFTP page token");
    }
  }

  @FunctionalInterface
  private interface SupplierWithException<T> {
    T get() throws Exception;
  }
}
