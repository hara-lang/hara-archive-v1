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

/** Truthful object-storage filesystem projection for S3-compatible services. */
final class S3Filesystem implements IFilesystem {
  record ObjectInfo(
      String key,
      long size,
      Long modifiedAt,
      String revision,
      Map<String, Object> extensions) {
    ObjectInfo {
      key = requireText(key, "S3 object key");
      if (size < 0) throw new IllegalArgumentException("negative S3 object size");
      extensions = extensions == null || extensions.isEmpty() ? Map.of() : Map.copyOf(extensions);
    }
  }

  record ListPage(List<ObjectInfo> objects, List<String> commonPrefixes, String nextToken) {
    ListPage {
      objects = List.copyOf(objects);
      commonPrefixes = List.copyOf(commonPrefixes);
    }
  }

  /** Trusted authenticated bucket client. Signing, token refresh and endpoint credentials stay host-owned. */
  interface Client extends AutoCloseable {
    boolean authenticated();

    Set<Capability> capabilities();

    ObjectInfo head(String bucket, String key) throws Exception;

    ListPage list(
        String bucket, String prefix, String delimiter, String continuationToken, int limit)
        throws Exception;

    byte[] read(String bucket, String key, long maxBytes) throws Exception;

    ObjectInfo put(
        String bucket,
        String key,
        byte[] bytes,
        boolean createOnly,
        String expectedRevision)
        throws Exception;

    void delete(String bucket, String key, String expectedRevision) throws Exception;

    ObjectInfo copy(
        String bucket,
        String sourceKey,
        String targetKey,
        boolean replace,
        String expectedSourceRevision,
        String expectedTargetRevision)
        throws Exception;

    @Override
    void close() throws Exception;
  }

  static final class ClientFailure extends Exception {
    private static final long serialVersionUID = 1L;
    private final String code;
    private final String providerCode;
    private final boolean retryable;

    ClientFailure(String code, String providerCode, boolean retryable) {
      super("S3-compatible transport operation failed");
      if (code == null || !code.matches("[a-z][a-z0-9-]*")) {
        throw new IllegalArgumentException("invalid S3 failure code");
      }
      this.code = code;
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
  }

  static final class Factory implements IFilesystemFactory {
    private static final Set<String> ALLOWED =
        Set.of(
            "credential-ref",
            "bucket",
            "prefix",
            "read-only?",
            "display",
            "operation-timeout-ms",
            "max-transfer-bytes");

    @Override
    public String kind() {
      return "s3";
    }

    @Override
    public void validate(Map<String, ?> configuration) {
      IFilesystemFactory.super.validate(configuration);
      for (String key : configuration.keySet()) {
        if (!ALLOWED.contains(key)) {
          throw new IllegalArgumentException("unknown S3 filesystem option " + key);
        }
      }
      requireText(configuration.get("credential-ref"), "S3 credential-ref");
      requireText(configuration.get("bucket"), "S3 bucket");
      prefix(configuration.get("prefix") instanceof String value ? value : "");
      Object readOnly = configuration.get("read-only?");
      if (readOnly != null && !(readOnly instanceof Boolean)) {
        throw new IllegalArgumentException("S3 read-only? must be a boolean");
      }
      Object display = configuration.get("display");
      if (display != null) requireText(display, "S3 display");
      positiveLong(configuration, "operation-timeout-ms", 30_000L);
      positiveLong(configuration, "max-transfer-bytes", 16L * 1024L * 1024L);
    }

    @Override
    public CompletionStage<IFilesystem> open(OpenContext context, Map<String, ?> configuration) {
      Objects.requireNonNull(context, "filesystem open context");
      validate(configuration);
      Object resolved = context.credentials().resolve((String) configuration.get("credential-ref"));
      if (!(resolved instanceof Client client)) {
        return CompletableFuture.failedFuture(
            new IllegalArgumentException(
                "S3 credential reference did not resolve to a trusted object client"));
      }
      if (!client.authenticated()) {
        return CompletableFuture.failedFuture(
            failure(
                "authentication-failed",
                "S3-compatible client is not authenticated",
                "open",
                null,
                null,
                "client-unauthenticated",
                false,
                null));
      }
      return CompletableFuture.completedFuture(
          new S3Filesystem(
              client,
              (String) configuration.get("bucket"),
              prefix(configuration.get("prefix") instanceof String value ? value : ""),
              configuration.get("display") instanceof String value ? value : "S3 filesystem",
              Boolean.TRUE.equals(configuration.get("read-only?")),
              positiveLong(configuration, "operation-timeout-ms", 30_000L),
              positiveLong(configuration, "max-transfer-bytes", 16L * 1024L * 1024L),
              context.ioExecutor(),
              context.scheduler()));
    }

    private static long positiveLong(Map<String, ?> values, String key, long fallback) {
      Object value = values.get(key);
      if (value == null) return fallback;
      if (!(value instanceof Number number) || number.longValue() <= 0) {
        throw new IllegalArgumentException("S3 " + key + " must be positive");
      }
      return number.longValue();
    }
  }

  private record Resolved(String path, EntryType type, ObjectInfo object) {}

  private record Pending(CompletableFuture<?> future, String operation, String path, String target) {}

  private final Client client;
  private final String bucket;
  private final String prefix;
  private final String display;
  private final boolean readOnly;
  private final long operationTimeoutMillis;
  private final long maxTransferBytes;
  private final Executor ioExecutor;
  private final ScheduledExecutorService scheduler;
  private final Capabilities capabilities;
  private final AtomicBoolean closed = new AtomicBoolean();
  private final Set<Pending> pending = ConcurrentHashMap.newKeySet();

  S3Filesystem(
      Client client,
      String bucket,
      String prefix,
      String display,
      boolean readOnly,
      long operationTimeoutMillis,
      long maxTransferBytes,
      Executor ioExecutor,
      ScheduledExecutorService scheduler) {
    this.client = Objects.requireNonNull(client, "S3 client");
    this.bucket = requireText(bucket, "S3 bucket");
    this.prefix = prefix(prefix);
    this.display = requireText(display, "S3 display");
    this.readOnly = readOnly;
    this.operationTimeoutMillis = operationTimeoutMillis;
    this.maxTransferBytes = maxTransferBytes;
    this.ioExecutor = Objects.requireNonNull(ioExecutor, "filesystem I/O executor");
    this.scheduler = Objects.requireNonNull(scheduler, "filesystem scheduler");
    this.capabilities = new Capabilities(advertised(client.capabilities(), readOnly));
  }

  @Override
  public Descriptor descriptor() {
    return new Descriptor(
        "s3",
        display,
        readOnly,
        capabilities,
        null,
        Map.of(
            "provider/root-scoped?", true,
            "provider/virtual-directories?", true,
            "provider/atomic-move?", false));
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
          return entry(resolve(logical, "stat", null));
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
          Resolved resolved = resolve(logical, "read", null);
          if (resolved.type() == EntryType.DIRECTORY) {
            throw failure("is-directory", "path is a virtual directory", "read", logical, null, null, false, null);
          }
          ObjectInfo object = resolved.object();
          if (object.size() > maxTransferBytes) throw transferLimit("read", logical, null);
          byte[] bytes = client.read(bucket, object.key(), maxTransferBytes);
          if (bytes.length > maxTransferBytes) throw transferLimit("read", logical, null);
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
            throw unsupported("write", logical, null, "append-unavailable");
          }
          if ("/".equals(logical)) {
            throw failure("is-directory", "mounted root is a directory", "write", logical, null, null, false, null);
          }
          if (copy.length > maxTransferBytes) throw transferLimit("write", logical, null);
          String key = key(logical);
          Resolved existing = optionalResolve(logical, "write", null);
          if (existing != null && existing.type() == EntryType.DIRECTORY) {
            throw failure("is-directory", "path is a virtual directory", "write", logical, null, null, false, null);
          }
          if (options.mode() == WriteMode.CREATE && existing != null) {
            throw failure("already-exists", "path already exists", "write", logical, null, null, false, null);
          }
          if (mutation.expectedRevision() != null) {
            if (existing == null || !mutation.expectedRevision().equals(existing.object().revision())) {
              throw conflict("write", logical, null);
            }
          }
          ObjectInfo written =
              client.put(
                  bucket,
                  key,
                  copy,
                  options.mode() == WriteMode.CREATE,
                  mutation.expectedRevision());
          return mutation(logical, written);
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
          Resolved resolved = resolve(logical, "entries", null);
          if (resolved.type() != EntryType.DIRECTORY) {
            throw failure("not-directory", "path is not a directory", "entries", logical, null, null, false, null);
          }
          String directoryPrefix = directoryPrefix(logical);
          ListPage page = client.list(bucket, directoryPrefix, "/", request.token(), request.limit());
          LinkedHashMap<String, Entry> byName = new LinkedHashMap<>();
          HashSet<String> collisions = new HashSet<>();
          for (ObjectInfo object : page.objects()) {
            String remainder = object.key().substring(directoryPrefix.length());
            if (remainder.isEmpty() || remainder.contains("/")) continue;
            Entry value = objectEntry(HaraLogicalPath.join(logical, remainder), object);
            if (byName.putIfAbsent(remainder, value) != null) collisions.add(remainder);
          }
          for (String common : page.commonPrefixes()) {
            if (!common.startsWith(directoryPrefix)) continue;
            String remainder = common.substring(directoryPrefix.length());
            if (remainder.endsWith("/")) remainder = remainder.substring(0, remainder.length() - 1);
            if (remainder.isEmpty() || remainder.contains("/")) continue;
            Entry value = directoryEntry(HaraLogicalPath.join(logical, remainder));
            if (byName.putIfAbsent(remainder, value) != null) collisions.add(remainder);
          }
          if (!collisions.isEmpty()) {
            throw failure(
                "ambiguous-path",
                "S3 object and prefix project to the same logical path",
                "entries",
                logical,
                null,
                "object-prefix-collision",
                false,
                null);
          }
          ArrayList<Entry> values = new ArrayList<>(byName.values());
          values.sort(Comparator.comparing(Entry::path));
          return new EntryPage(values, page.nextToken());
        });
  }

  @Override
  public CompletionStage<Mutation> mkdir(
      CallContext context, String path, MkdirOptions options, MutationContext mutation) {
    Objects.requireNonNull(context, "filesystem call context");
    String logical = normalise(path);
    if ("/".equals(logical) && options.existsOk()) {
      return CompletableFuture.completedFuture(Mutation.path("/"));
    }
    return CompletableFuture.failedFuture(
        unsupported("mkdir", logical, null, "virtual-directory-no-marker-policy"));
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
          Resolved resolved = optionalResolve(logical, "delete", null);
          if (resolved == null) {
            if (options.missingOk()) return Mutation.path(logical);
            throw failure("not-found", "path does not exist", "delete", logical, null, null, false, null);
          }
          if (resolved.type() == EntryType.DIRECTORY) {
            throw unsupported("delete", logical, null, "recursive-prefix-delete-unavailable");
          }
          checkExpected(resolved.object(), mutation.expectedRevision(), "delete", logical, null);
          client.delete(bucket, resolved.object().key(), mutation.expectedRevision());
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
          requireWritable(Capability.COPY, "copy", sourceLogical, targetLogical);
          requireRevisionSupport(mutation, "copy", sourceLogical, targetLogical);
          if (options.preserveModified()) {
            throw unsupported("copy", sourceLogical, targetLogical, "preserve-modified-unavailable");
          }
          Resolved sourceValue = resolve(sourceLogical, "copy", targetLogical);
          if (sourceValue.type() == EntryType.DIRECTORY) {
            throw unsupported("copy", sourceLogical, targetLogical, "recursive-prefix-copy-unavailable");
          }
          checkExpected(
              sourceValue.object(), mutation.expectedRevision(), "copy", sourceLogical, targetLogical);
          Resolved targetValue = optionalResolve(targetLogical, "copy", sourceLogical);
          if (targetValue != null) {
            if (targetValue.type() == EntryType.DIRECTORY) {
              throw failure("is-directory", "target is a virtual directory", "copy", sourceLogical, targetLogical, null, false, null);
            }
            if (!options.replace()) {
              throw failure("already-exists", "target already exists", "copy", sourceLogical, targetLogical, null, false, null);
            }
            checkExpected(
                targetValue.object(),
                mutation.expectedTargetRevision(),
                "copy",
                sourceLogical,
                targetLogical);
          } else if (mutation.expectedTargetRevision() != null) {
            throw conflict("copy", sourceLogical, targetLogical);
          }
          ObjectInfo copied =
              client.copy(
                  bucket,
                  sourceValue.object().key(),
                  key(targetLogical),
                  options.replace(),
                  mutation.expectedRevision(),
                  mutation.expectedTargetRevision());
          return mutation(targetLogical, copied);
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
    if (options.atomic()) {
      return CompletableFuture.failedFuture(
          unsupported("move", sourceLogical, targetLogical, "atomic-move-unavailable"));
    }
    return submit(
        context,
        "move",
        sourceLogical,
        targetLogical,
        () -> {
          requireWritable(Capability.COPY, "move", sourceLogical, targetLogical);
          requireWritable(Capability.DELETE, "move", sourceLogical, targetLogical);
          requireRevisionSupport(mutation, "move", sourceLogical, targetLogical);
          if (sourceLogical.equals(targetLogical)) {
            Resolved same = resolve(sourceLogical, "move", targetLogical);
            checkExpected(
                same.object(), mutation.expectedRevision(), "move", sourceLogical, targetLogical);
            checkExpected(
                same.object(),
                mutation.expectedTargetRevision(),
                "move",
                sourceLogical,
                targetLogical);
            return same.type() == EntryType.FILE ? mutation(targetLogical, same.object()) : Mutation.path(targetLogical);
          }
          Resolved sourceValue = resolve(sourceLogical, "move", targetLogical);
          if (sourceValue.type() == EntryType.DIRECTORY) {
            throw unsupported("move", sourceLogical, targetLogical, "recursive-prefix-move-unavailable");
          }
          checkExpected(
              sourceValue.object(), mutation.expectedRevision(), "move", sourceLogical, targetLogical);
          Resolved targetValue = optionalResolve(targetLogical, "move", sourceLogical);
          if (targetValue != null) {
            if (targetValue.type() == EntryType.DIRECTORY) {
              throw failure("is-directory", "target is a virtual directory", "move", sourceLogical, targetLogical, null, false, null);
            }
            if (!options.replace()) {
              throw failure("already-exists", "target already exists", "move", sourceLogical, targetLogical, null, false, null);
            }
            checkExpected(
                targetValue.object(),
                mutation.expectedTargetRevision(),
                "move",
                sourceLogical,
                targetLogical);
          } else if (mutation.expectedTargetRevision() != null) {
            throw conflict("move", sourceLogical, targetLogical);
          }
          ObjectInfo copied =
              client.copy(
                  bucket,
                  sourceValue.object().key(),
                  key(targetLogical),
                  options.replace(),
                  mutation.expectedRevision(),
                  mutation.expectedTargetRevision());
          try {
            client.delete(bucket, sourceValue.object().key(), mutation.expectedRevision());
          } catch (Exception error) {
            throw failure(
                "io",
                "S3 move copied the target but could not remove the source",
                "move",
                sourceLogical,
                targetLogical,
                "copy-succeeded-delete-failed",
                false,
                error);
          }
          return mutation(targetLogical, copied);
        });
  }

  @Override
  public CompletionStage<Void> close(CallContext context) {
    Objects.requireNonNull(context, "filesystem call context");
    if (!closed.compareAndSet(false, true)) return CompletableFuture.completedFuture(null);
    for (Pending operation : pending) {
      operation.future().completeExceptionally(
          FilesystemException.providerClosed(
              "s3", operation.operation(), operation.path(), operation.target()));
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

  private Resolved resolve(String logical, String operation, String target) throws Exception {
    Resolved value = optionalResolve(logical, operation, target);
    if (value == null) {
      throw failure("not-found", "path does not exist", operation, logical, target, null, false, null);
    }
    return value;
  }

  private Resolved optionalResolve(String logical, String operation, String target) throws Exception {
    String normal = normalise(logical);
    if ("/".equals(normal)) return new Resolved("/", EntryType.DIRECTORY, null);
    String objectKey = key(normal);
    ObjectInfo object = optionalHead(objectKey);
    boolean directory = prefixExists(objectKey + "/");
    if (object != null && directory) {
      throw failure(
          "ambiguous-path",
          "S3 object and prefix project to the same logical path",
          operation,
          normal,
          target,
          "object-prefix-collision",
          false,
          null);
    }
    if (object != null) return new Resolved(normal, EntryType.FILE, object);
    if (directory) return new Resolved(normal, EntryType.DIRECTORY, null);
    return null;
  }

  private ObjectInfo optionalHead(String key) throws Exception {
    try {
      return client.head(bucket, key);
    } catch (ClientFailure error) {
      if ("not-found".equals(error.code())) return null;
      throw error;
    }
  }

  private boolean prefixExists(String objectPrefix) throws Exception {
    ListPage page = client.list(bucket, objectPrefix, "/", null, 1);
    return !page.objects().isEmpty() || !page.commonPrefixes().isEmpty();
  }

  private Entry entry(Resolved resolved) {
    return resolved.type() == EntryType.FILE
        ? objectEntry(resolved.path(), resolved.object())
        : directoryEntry(resolved.path());
  }

  private Entry objectEntry(String logical, ObjectInfo object) {
    LinkedHashMap<String, Object> extensions = new LinkedHashMap<>(object.extensions());
    return new Entry(
        logical,
        HaraLogicalPath.fileName(logical),
        EntryType.FILE,
        object.size(),
        object.modifiedAt(),
        null,
        object.revision(),
        capabilities,
        extensions);
  }

  private Entry directoryEntry(String logical) {
    return new Entry(
        logical,
        "/".equals(logical) ? "" : HaraLogicalPath.fileName(logical),
        EntryType.DIRECTORY,
        null,
        null,
        null,
        null,
        new Capabilities(Set.of(Capability.ENTRIES)),
        Map.of("provider/virtual?", true));
  }

  private Mutation mutation(String logical, ObjectInfo object) {
    return new Mutation(logical, object.revision(), null, Map.of());
  }

  private static Set<Capability> advertised(Set<Capability> transport, boolean readOnly) {
    HashSet<Capability> output = new HashSet<>();
    if (transport.contains(Capability.READ)) output.add(Capability.READ);
    if (transport.contains(Capability.ENTRIES)) output.add(Capability.ENTRIES);
    if (!readOnly) {
      for (Capability value :
          List.of(Capability.WRITE, Capability.DELETE, Capability.COPY, Capability.REVISION_CHECK)) {
        if (transport.contains(value)) output.add(value);
      }
      if (output.contains(Capability.COPY) && output.contains(Capability.DELETE)) {
        output.add(Capability.MOVE);
      }
    }
    return Set.copyOf(output);
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
          "S3 mount is read-only",
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
      throw FilesystemException.unsupportedRevision("s3", operation, path, target);
    }
  }

  private static void checkExpected(
      ObjectInfo object, String expected, String operation, String path, String target) {
    if (expected == null) return;
    if (object == null || object.revision() == null || !expected.equals(object.revision())) {
      throw conflict(operation, path, target);
    }
  }

  private <T> CompletionStage<T> submit(
      CallContext context,
      String operation,
      String path,
      String target,
      ThrowingSupplier<T> body) {
    Objects.requireNonNull(context, "filesystem call context");
    if (closed.get()) {
      return CompletableFuture.failedFuture(
          FilesystemException.providerClosed("s3", operation, path, target));
    }
    try {
      context.check("s3", operation, path, target);
    } catch (RuntimeException error) {
      return CompletableFuture.failedFuture(error);
    }
    CompletableFuture<T> result = new CompletableFuture<>();
    Pending tracked = new Pending(result, operation, path, target);
    pending.add(tracked);
    long timeoutNanos = TimeUnit.MILLISECONDS.toNanos(operationTimeoutMillis);
    if (context.hasDeadline()) timeoutNanos = Math.min(timeoutNanos, context.remainingNanos());
    ScheduledFuture<?> timeout =
        scheduler.schedule(
            () -> result.completeExceptionally(FilesystemException.timeout("s3", operation, path, target)),
            Math.max(0L, timeoutNanos),
            TimeUnit.NANOSECONDS);
    AutoCloseable cancellation =
        context.onCancel(
            () -> result.completeExceptionally(FilesystemException.cancelled("s3", operation, path, target)));
    result.whenComplete(
        (ignored, error) -> {
          timeout.cancel(false);
          pending.remove(tracked);
          try {
            cancellation.close();
          } catch (Exception ignoredClose) {
            // Hook removal is best-effort after settlement.
          }
        });
    ioExecutor.execute(
        () -> {
          if (result.isDone()) return;
          try {
            context.check("s3", operation, path, target);
            result.complete(body.get());
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
          "S3-compatible transport operation failed",
          operation,
          path,
          target,
          failure.providerCode(),
          failure.retryable(),
          failure);
    }
    if (current instanceof HaraLogicalPath.Error logical) {
      return failure(logical.code(), logical.getMessage(), operation, path, target, null, false, logical);
    }
    return failure(
        "io",
        "S3 filesystem operation failed",
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

  private static FilesystemException conflict(String operation, String path, String target) {
    return failure(
        "conflict",
        "S3 object revision does not match",
        operation,
        path,
        target,
        "revision-mismatch",
        false,
        null);
  }

  private static FilesystemException transferLimit(String operation, String path, String target) {
    return failure(
        "quota-exceeded",
        "S3 transfer exceeds configured limit",
        operation,
        path,
        target,
        "transfer-limit",
        false,
        null);
  }

  private static FilesystemException unsupported(
      String operation, String path, String target, String providerCode) {
    return failure(
        "unsupported",
        "S3 provider does not support the requested operation",
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
        code, message, "s3", operation, path, target, providerCode, retryable, cause);
  }

  private String key(String logical) {
    String normal = normalise(logical);
    if ("/".equals(normal)) return prefix;
    return prefix + normal.substring(1);
  }

  private String directoryPrefix(String logical) {
    String value = key(logical);
    if (value.isEmpty() || value.endsWith("/")) return value;
    return value + "/";
  }

  private static String prefix(String value) {
    if (value == null || value.isBlank()) return "";
    if (value.indexOf('\0') >= 0 || value.indexOf('\\') >= 0 || value.startsWith("/")) {
      throw new IllegalArgumentException("S3 prefix must be a relative canonical key prefix");
    }
    ArrayList<String> segments = new ArrayList<>();
    for (String segment : value.split("/+")) {
      if (segment.isEmpty()) continue;
      if (".".equals(segment) || "..".equals(segment)) {
        throw new IllegalArgumentException("S3 prefix cannot contain dot segments");
      }
      segments.add(segment);
    }
    return segments.isEmpty() ? "" : String.join("/", segments) + "/";
  }

  private static String normalise(String path) {
    return HaraLogicalPath.normalise(path);
  }

  private static String requireText(Object value, String label) {
    if (!(value instanceof String text) || text.isBlank()) {
      throw new IllegalArgumentException(label + " is required");
    }
    return text;
  }

  @FunctionalInterface
  private interface ThrowingSupplier<T> {
    T get() throws Exception;
  }
}
