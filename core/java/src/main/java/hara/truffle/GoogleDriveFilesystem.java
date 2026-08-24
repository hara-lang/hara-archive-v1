package hara.truffle;

import java.util.ArrayList;
import java.util.Comparator;
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

/** Provider-neutral Google Drive folder mount over a trusted authenticated API capability. */
final class GoogleDriveFilesystem implements IFilesystem {
  enum ItemType {
    FILE,
    FOLDER,
    SHORTCUT,
    WORKSPACE,
    OTHER
  }

  record ItemCapabilities(
      boolean read,
      boolean write,
      boolean addChildren,
      boolean trash,
      boolean copy,
      boolean move,
      boolean rename) {}

  record Item(
      String id,
      String parentId,
      String name,
      ItemType type,
      Long size,
      Long modifiedAt,
      String revision,
      String mimeType,
      ItemCapabilities capabilities,
      Map<String, Object> extensions) {
    Item {
      id = requireText(id, "Drive item id");
      name = Objects.requireNonNull(name, "Drive item name");
      type = Objects.requireNonNull(type, "Drive item type");
      capabilities = Objects.requireNonNull(capabilities, "Drive item capabilities");
      if (size != null && size < 0) throw new IllegalArgumentException("negative Drive item size");
      extensions = extensions == null || extensions.isEmpty() ? Map.of() : Map.copyOf(extensions);
    }
  }

  record ItemPage(List<Item> items, String nextToken) {
    ItemPage {
      items = List.copyOf(items);
    }
  }

  interface Client extends AutoCloseable {
    boolean authenticated();

    Set<Capability> capabilities();

    Item get(String id) throws Exception;

    ItemPage listChildren(String parentId, String pageToken, int pageSize) throws Exception;

    byte[] readMedia(String id, long maxBytes) throws Exception;

    Item createFile(String parentId, String name, byte[] bytes) throws Exception;

    Item updateFile(String id, byte[] bytes, String expectedRevision) throws Exception;

    Item createFolder(String parentId, String name) throws Exception;

    void trash(String id, String expectedRevision) throws Exception;

    Item copyFile(
        String sourceId, String parentId, String name, String expectedRevision) throws Exception;

    Item move(
        String id,
        String oldParentId,
        String newParentId,
        String newName,
        String expectedRevision)
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
      super("Google Drive transport operation failed");
      if (code == null || !code.matches("[a-z][a-z0-9-]*")) {
        throw new IllegalArgumentException("invalid Drive failure code");
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
            "root-id",
            "shared-drive-id",
            "read-only?",
            "display",
            "workspace-documents",
            "operation-timeout-ms",
            "max-transfer-bytes");

    @Override
    public String kind() {
      return "google-drive";
    }

    @Override
    public void validate(Map<String, ?> configuration) {
      IFilesystemFactory.super.validate(configuration);
      for (String key : configuration.keySet()) {
        if (!ALLOWED.contains(key)) {
          throw new IllegalArgumentException("unknown Google Drive filesystem option " + key);
        }
      }
      requireText(configuration.get("credential-ref"), "Google Drive credential-ref");
      requireText(configuration.get("root-id"), "Google Drive root-id");
      Object sharedDrive = configuration.get("shared-drive-id");
      if (sharedDrive != null) requireText(sharedDrive, "Google Drive shared-drive-id");
      Object readOnly = configuration.get("read-only?");
      if (readOnly != null && !(readOnly instanceof Boolean)) {
        throw new IllegalArgumentException("Google Drive read-only? must be a boolean");
      }
      Object display = configuration.get("display");
      if (display != null) requireText(display, "Google Drive display");
      Object workspace = configuration.get("workspace-documents");
      if (workspace != null && !"unsupported".equals(String.valueOf(workspace))) {
        throw new IllegalArgumentException(
            "Google Drive workspace-documents currently supports only unsupported");
      }
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
                "Google Drive credential reference did not resolve to a trusted Drive client"));
      }
      if (!client.authenticated()) {
        return CompletableFuture.failedFuture(
            failure(
                "authentication-failed",
                "Google Drive client is not authenticated",
                "open",
                null,
                null,
                "client-unauthenticated",
                false,
                null));
      }
      GoogleDriveFilesystem filesystem =
          new GoogleDriveFilesystem(
              client,
              (String) configuration.get("root-id"),
              configuration.get("shared-drive-id") instanceof String value ? value : null,
              configuration.get("display") instanceof String value ? value : "Google Drive filesystem",
              Boolean.TRUE.equals(configuration.get("read-only?")),
              positiveLong(configuration, "operation-timeout-ms", 30_000L),
              positiveLong(configuration, "max-transfer-bytes", 16L * 1024L * 1024L),
              context.ioExecutor(),
              context.scheduler());
      return filesystem.submit(
          CallContext.create(),
          "open",
          "/",
          null,
          () -> {
            Item root = filesystem.client.get(filesystem.rootId);
            if (root.type() != ItemType.FOLDER) {
              throw failure(
                  "not-directory",
                  "Google Drive root is not a folder",
                  "open",
                  "/",
                  null,
                  "root-not-folder",
                  false,
                  null);
            }
            if (!root.capabilities().read()) {
              throw failure(
                  "permission-denied",
                  "Google Drive root is not readable",
                  "open",
                  "/",
                  null,
                  "root-unreadable",
                  false,
                  null);
            }
            return (IFilesystem) filesystem;
          });
    }

    private static long positiveLong(Map<String, ?> values, String key, long fallback) {
      Object value = values.get(key);
      if (value == null) return fallback;
      if (!(value instanceof Number number) || number.longValue() <= 0) {
        throw new IllegalArgumentException("Google Drive " + key + " must be positive");
      }
      return number.longValue();
    }
  }

  private record Resolved(String path, Item item) {}

  private record Pending(CompletableFuture<?> future, String operation, String path, String target) {}

  private final Client client;
  private final String rootId;
  private final String sharedDriveId;
  private final String display;
  private final boolean readOnly;
  private final long operationTimeoutMillis;
  private final long maxTransferBytes;
  private final Executor ioExecutor;
  private final ScheduledExecutorService scheduler;
  private final Capabilities capabilities;
  private final AtomicBoolean closed = new AtomicBoolean();
  private final Set<Pending> pending = ConcurrentHashMap.newKeySet();

  GoogleDriveFilesystem(
      Client client,
      String rootId,
      String sharedDriveId,
      String display,
      boolean readOnly,
      long operationTimeoutMillis,
      long maxTransferBytes,
      Executor ioExecutor,
      ScheduledExecutorService scheduler) {
    this.client = Objects.requireNonNull(client, "Google Drive client");
    this.rootId = requireText(rootId, "Google Drive root id");
    this.sharedDriveId = sharedDriveId;
    this.display = requireText(display, "Google Drive display");
    this.readOnly = readOnly;
    this.operationTimeoutMillis = operationTimeoutMillis;
    this.maxTransferBytes = maxTransferBytes;
    this.ioExecutor = Objects.requireNonNull(ioExecutor, "filesystem I/O executor");
    this.scheduler = Objects.requireNonNull(scheduler, "filesystem scheduler");
    this.capabilities = new Capabilities(advertised(client.capabilities(), readOnly));
  }

  @Override
  public Descriptor descriptor() {
    LinkedHashMap<String, Object> extensions = new LinkedHashMap<>();
    extensions.put("provider/root-scoped?", true);
    extensions.put("provider/workspace-documents", "unsupported");
    extensions.put("provider/shared-drive?", sharedDriveId != null);
    return new Descriptor("google-drive", display, readOnly, capabilities, null, extensions);
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
          Item item = resolved.item();
          requireReadableFile(item, "read", logical, null);
          if (item.size() != null && item.size() > maxTransferBytes) {
            throw transferLimit("read", logical, null);
          }
          byte[] bytes = client.readMedia(item.id(), maxTransferBytes);
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
          if (copy.length > maxTransferBytes) throw transferLimit("write", logical, null);
          Resolved parent = resolveParent(logical, options.parents(), "write");
          requireFolderMutation(parent.item(), "write", logical, null);
          String name = HaraLogicalPath.fileName(logical);
          Item existing = uniqueChild(parent.item().id(), name, "write", logical, null);
          Item written;
          if (options.mode() == WriteMode.CREATE) {
            if (existing != null) {
              throw failure(
                  "already-exists", "path already exists", "write", logical, null, null, false, null);
            }
            written = client.createFile(parent.item().id(), name, copy);
          } else {
            if (existing == null) {
              written = client.createFile(parent.item().id(), name, copy);
            } else {
              requireOrdinaryFile(existing, "write", logical, null);
              requireItem(existing.capabilities().write(), "write", logical, null);
              checkExpected(existing, mutation.expectedRevision(), "write", logical, null);
              written = client.updateFile(existing.id(), copy, mutation.expectedRevision());
            }
          }
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
          requireDirectory(resolved.item(), "entries", logical, null);
          ItemPage page = client.listChildren(resolved.item().id(), request.token(), request.limit());
          ArrayList<Entry> entries = new ArrayList<>();
          for (Item child : page.items()) {
            entries.add(entry(HaraLogicalPath.join(logical, child.name()), child));
          }
          entries.sort(Comparator.comparing(Entry::path));
          return new EntryPage(entries, page.nextToken());
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
            if (options.existsOk()) {
              Item root = client.get(rootId);
              checkExpected(root, mutation.expectedRevision(), "mkdir", logical, null);
              return mutation("/", root);
            }
            throw failure(
                "already-exists", "mounted root exists", "mkdir", logical, null, null, false, null);
          }
          Resolved parent = resolveParent(logical, options.parents(), "mkdir");
          requireFolderMutation(parent.item(), "mkdir", logical, null);
          String name = HaraLogicalPath.fileName(logical);
          Item existing = uniqueChild(parent.item().id(), name, "mkdir", logical, null);
          if (existing != null) {
            if (existing.type() == ItemType.FOLDER && options.existsOk()) {
              checkExpected(existing, mutation.expectedRevision(), "mkdir", logical, null);
              return mutation(logical, existing);
            }
            throw failure(
                "already-exists", "path already exists", "mkdir", logical, null, null, false, null);
          }
          Item created = client.createFolder(parent.item().id(), name);
          return mutation(logical, created);
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
          Resolved resolved;
          try {
            resolved = resolve(logical, "delete", null);
          } catch (FilesystemException error) {
            if (options.missingOk() && "not-found".equals(error.code())) return Mutation.path(logical);
            throw error;
          }
          Item item = resolved.item();
          if (item.type() == ItemType.SHORTCUT) {
            throw unsupported("delete", logical, null, "shortcut-no-follow");
          }
          requireItem(item.capabilities().trash(), "delete", logical, null);
          checkExpected(item, mutation.expectedRevision(), "delete", logical, null);
          client.trash(item.id(), mutation.expectedRevision());
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
          Item sourceItem = sourceValue.item();
          requireOrdinaryFile(sourceItem, "copy", sourceLogical, targetLogical);
          requireItem(sourceItem.capabilities().copy(), "copy", sourceLogical, targetLogical);
          checkExpected(sourceItem, mutation.expectedRevision(), "copy", sourceLogical, targetLogical);
          Resolved parent = resolveParent(targetLogical, options.parents(), "copy");
          requireFolderMutation(parent.item(), "copy", sourceLogical, targetLogical);
          String name = HaraLogicalPath.fileName(targetLogical);
          Item targetItem = uniqueChild(parent.item().id(), name, "copy", sourceLogical, targetLogical);
          if (targetItem != null) {
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
            requireItem(targetItem.capabilities().trash(), "copy", sourceLogical, targetLogical);
            checkExpected(
                targetItem,
                mutation.expectedTargetRevision(),
                "copy",
                sourceLogical,
                targetLogical);
            client.trash(targetItem.id(), mutation.expectedTargetRevision());
          }
          Item copied =
              client.copyFile(
                  sourceItem.id(), parent.item().id(), name, mutation.expectedRevision());
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
    return submit(
        context,
        "move",
        sourceLogical,
        targetLogical,
        () -> {
          requireWritable(Capability.MOVE, "move", sourceLogical, targetLogical);
          requireRevisionSupport(mutation, "move", sourceLogical, targetLogical);
          if (options.atomic()) {
            throw unsupported("move", sourceLogical, targetLogical, "atomic-move-unavailable");
          }
          if ("/".equals(sourceLogical) || "/".equals(targetLogical)) {
            throw failure(
                "denied", "cannot move mounted root", "move", sourceLogical, targetLogical, null, false, null);
          }
          if (sourceLogical.equals(targetLogical)) {
            Resolved same = resolve(sourceLogical, "move", targetLogical);
            checkExpected(
                same.item(), mutation.expectedRevision(), "move", sourceLogical, targetLogical);
            checkExpected(
                same.item(), mutation.expectedTargetRevision(), "move", sourceLogical, targetLogical);
            return mutation(targetLogical, same.item());
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
          Resolved sourceValue = resolve(sourceLogical, "move", targetLogical);
          Item sourceItem = sourceValue.item();
          if (sourceItem.type() == ItemType.SHORTCUT) {
            throw unsupported("move", sourceLogical, targetLogical, "shortcut-no-follow");
          }
          requireItem(
              sourceItem.capabilities().move() && sourceItem.capabilities().rename(),
              "move",
              sourceLogical,
              targetLogical);
          checkExpected(sourceItem, mutation.expectedRevision(), "move", sourceLogical, targetLogical);
          Resolved parent = resolveParent(targetLogical, options.parents(), "move");
          requireFolderMutation(parent.item(), "move", sourceLogical, targetLogical);
          String name = HaraLogicalPath.fileName(targetLogical);
          Item targetItem = uniqueChild(parent.item().id(), name, "move", sourceLogical, targetLogical);
          if (targetItem != null) {
            if (!options.replace()) {
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
            requireItem(targetItem.capabilities().trash(), "move", sourceLogical, targetLogical);
            checkExpected(
                targetItem,
                mutation.expectedTargetRevision(),
                "move",
                sourceLogical,
                targetLogical);
            client.trash(targetItem.id(), mutation.expectedTargetRevision());
          }
          Item moved =
              client.move(
                  sourceItem.id(),
                  sourceItem.parentId(),
                  parent.item().id(),
                  name,
                  mutation.expectedRevision());
          return mutation(targetLogical, moved);
        });
  }

  @Override
  public CompletionStage<Void> close(CallContext context) {
    Objects.requireNonNull(context, "filesystem call context");
    if (!closed.compareAndSet(false, true)) return CompletableFuture.completedFuture(null);
    for (Pending operation : pending) {
      operation.future().completeExceptionally(
          FilesystemException.providerClosed(
              "google-drive", operation.operation(), operation.path(), operation.target()));
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
    String normal = normalise(logical);
    Item current = client.get(rootId);
    if ("/".equals(normal)) return new Resolved("/", current);
    String path = "/";
    for (String segment : segments(normal)) {
      requireDirectory(current, operation, path, target);
      Item next = uniqueChild(current.id(), segment, operation, normal, target);
      if (next == null) {
        throw failure(
            "not-found", "path does not exist", operation, normal, target, null, false, null);
      }
      if (next.type() == ItemType.SHORTCUT) {
        if (segment.equals(segments(normal).get(segments(normal).size() - 1))) {
          current = next;
          path = HaraLogicalPath.join(path, segment);
          continue;
        }
        throw failure(
            "outside-root",
            "Google Drive path traverses a shortcut",
            operation,
            normal,
            target,
            "shortcut-no-follow",
            false,
            null);
      }
      current = next;
      path = HaraLogicalPath.join(path, segment);
    }
    return new Resolved(normal, current);
  }

  private Resolved resolveParent(String logical, boolean createParents, String operation)
      throws Exception {
    String normal = normalise(logical);
    if ("/".equals(normal)) {
      throw failure("invalid-path", "mounted root has no parent", operation, normal, null, null, false, null);
    }
    List<String> segments = segments(normal);
    Item current = client.get(rootId);
    String path = "/";
    for (int index = 0; index + 1 < segments.size(); index++) {
      String segment = segments.get(index);
      requireDirectory(current, operation, path, null);
      Item child = uniqueChild(current.id(), segment, operation, normal, null);
      if (child == null) {
        if (!createParents) {
          throw failure(
              "not-found", "parent folder does not exist", operation, normal, null, null, false, null);
        }
        requireWritable(Capability.MKDIR, operation, normal, null);
        requireFolderMutation(current, operation, normal, null);
        child = client.createFolder(current.id(), segment);
      }
      if (child.type() == ItemType.SHORTCUT) {
        throw failure(
            "outside-root",
            "Google Drive path traverses a shortcut",
            operation,
            normal,
            null,
            "shortcut-no-follow",
            false,
            null);
      }
      requireDirectory(child, operation, normal, null);
      current = child;
      path = HaraLogicalPath.join(path, segment);
    }
    return new Resolved(path, current);
  }

  private Item uniqueChild(
      String parentId, String name, String operation, String path, String target) throws Exception {
    ArrayList<Item> matches = new ArrayList<>();
    String token = null;
    do {
      ItemPage page = client.listChildren(parentId, token, 200);
      for (Item item : page.items()) {
        if (name.equals(item.name())) matches.add(item);
      }
      token = page.nextToken();
    } while (token != null);
    if (matches.size() > 1) {
      throw failure(
          "ambiguous-path",
          "Google Drive path resolves to multiple items",
          operation,
          path,
          target,
          "duplicate-name",
          false,
          null);
    }
    return matches.isEmpty() ? null : matches.get(0);
  }

  private Entry entry(Resolved resolved) {
    return entry(resolved.path(), resolved.item());
  }

  private Entry entry(String logical, Item item) {
    LinkedHashMap<String, Object> extensions = new LinkedHashMap<>(item.extensions());
    extensions.put("provider/mime", item.mimeType());
    extensions.put("provider/workspace?", item.type() == ItemType.WORKSPACE);
    return new Entry(
        logical,
        "/".equals(logical) ? "" : HaraLogicalPath.fileName(logical),
        entryType(item.type()),
        item.type() == ItemType.FILE ? item.size() : null,
        item.modifiedAt(),
        item.id(),
        item.revision(),
        itemCapabilities(item),
        extensions);
  }

  private Mutation mutation(String path, Item item) {
    return new Mutation(path, item.revision(), null, Map.of("file/id", item.id()));
  }

  private static EntryType entryType(ItemType type) {
    return switch (type) {
      case FILE -> EntryType.FILE;
      case FOLDER -> EntryType.DIRECTORY;
      case SHORTCUT -> EntryType.SYMLINK;
      case WORKSPACE, OTHER -> EntryType.OTHER;
    };
  }

  private Capabilities itemCapabilities(Item item) {
    HashSet<Capability> output = new HashSet<>();
    ItemCapabilities itemCaps = item.capabilities();
    if (itemCaps.read() && item.type() == ItemType.FILE) output.add(Capability.READ);
    if (item.type() == ItemType.FOLDER) output.add(Capability.ENTRIES);
    if (!readOnly) {
      if (itemCaps.write() && item.type() == ItemType.FILE) output.add(Capability.WRITE);
      if (itemCaps.addChildren() && item.type() == ItemType.FOLDER) output.add(Capability.MKDIR);
      if (itemCaps.trash()) output.add(Capability.DELETE);
      if (itemCaps.copy() && item.type() == ItemType.FILE) output.add(Capability.COPY);
      if (itemCaps.move() && itemCaps.rename()) output.add(Capability.MOVE);
      if (capabilities.contains(Capability.REVISION_CHECK)) output.add(Capability.REVISION_CHECK);
    }
    return new Capabilities(output);
  }

  private static Set<Capability> advertised(Set<Capability> transport, boolean readOnly) {
    HashSet<Capability> output = new HashSet<>();
    if (transport.contains(Capability.READ)) output.add(Capability.READ);
    if (transport.contains(Capability.ENTRIES)) output.add(Capability.ENTRIES);
    if (!readOnly) {
      for (Capability value :
          List.of(
              Capability.WRITE,
              Capability.MKDIR,
              Capability.DELETE,
              Capability.COPY,
              Capability.MOVE,
              Capability.REVISION_CHECK)) {
        if (transport.contains(value)) output.add(value);
      }
    }
    return Set.copyOf(output);
  }

  private void requireReadableFile(Item item, String operation, String path, String target) {
    requireOrdinaryFile(item, operation, path, target);
    requireItem(item.capabilities().read(), operation, path, target);
  }

  private static void requireOrdinaryFile(
      Item item, String operation, String path, String target) {
    if (item.type() == ItemType.SHORTCUT) {
      throw unsupported(operation, path, target, "shortcut-no-follow");
    }
    if (item.type() == ItemType.WORKSPACE) {
      throw unsupported(operation, path, target, "workspace-export-unconfigured");
    }
    if (item.type() == ItemType.FOLDER) {
      throw failure(
          "is-directory", "path is a directory", operation, path, target, null, false, null);
    }
    if (item.type() != ItemType.FILE) {
      throw unsupported(operation, path, target, "unsupported-item-type");
    }
  }

  private static void requireDirectory(
      Item item, String operation, String path, String target) {
    if (item.type() == ItemType.SHORTCUT) {
      throw failure(
          "outside-root",
          "Google Drive path traverses a shortcut",
          operation,
          path,
          target,
          "shortcut-no-follow",
          false,
          null);
    }
    if (item.type() != ItemType.FOLDER) {
      throw failure(
          "not-directory", "path is not a directory", operation, path, target, null, false, null);
    }
  }

  private static void requireFolderMutation(
      Item folder, String operation, String path, String target) {
    requireDirectory(folder, operation, path, target);
    requireItem(folder.capabilities().addChildren(), operation, path, target);
  }

  private static void requireItem(
      boolean allowed, String operation, String path, String target) {
    if (!allowed) {
      throw failure(
          "permission-denied",
          "Google Drive item capability denies the mutation",
          operation,
          path,
          target,
          "item-capability",
          false,
          null);
    }
  }

  private static void checkExpected(
      Item item, String expected, String operation, String path, String target) {
    if (expected == null) return;
    if (item.revision() == null || !expected.equals(item.revision())) {
      throw failure(
          "conflict",
          "Google Drive item revision does not match",
          operation,
          path,
          target,
          "revision-mismatch",
          false,
          null);
    }
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
          "Google Drive mount is read-only",
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
      throw FilesystemException.unsupportedRevision("google-drive", operation, path, target);
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
          FilesystemException.providerClosed("google-drive", operation, path, target));
    }
    try {
      context.check("google-drive", operation, path, target);
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
            () ->
                result.completeExceptionally(
                    FilesystemException.timeout("google-drive", operation, path, target)),
            Math.max(0L, timeoutNanos),
            TimeUnit.NANOSECONDS);
    AutoCloseable cancellation =
        context.onCancel(
            () ->
                result.completeExceptionally(
                    FilesystemException.cancelled("google-drive", operation, path, target)));
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
            context.check("google-drive", operation, path, target);
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
          "Google Drive transport operation failed",
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
        "Google Drive filesystem operation failed",
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

  private static FilesystemException transferLimit(String operation, String path, String target) {
    return failure(
        "quota-exceeded",
        "Google Drive transfer exceeds configured limit",
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
        "Google Drive provider does not support the requested operation",
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
        code, message, "google-drive", operation, path, target, providerCode, retryable, cause);
  }

  private static String normalise(String path) {
    return HaraLogicalPath.normalise(path);
  }

  private static List<String> segments(String logical) {
    String normal = normalise(logical);
    if ("/".equals(normal)) return List.of();
    return List.of(normal.substring(1).split("/"));
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
