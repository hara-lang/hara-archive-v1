package hara.truffle;

import java.time.Duration;
import java.time.Instant;
import java.util.ArrayList;
import java.util.Collections;
import java.util.EnumSet;
import java.util.List;
import java.util.Map;
import java.util.Objects;
import java.util.Set;
import java.util.concurrent.CompletionStage;
import java.util.concurrent.CopyOnWriteArrayList;
import java.util.concurrent.atomic.AtomicBoolean;

/**
 * An opened, provider-neutral filesystem mounted at a Session's logical root.
 *
 * <p>This is a trusted runtime interface. It is not exposed as a Hara protocol and it carries no
 * provider-construction or credential authority.
 */
public interface IFilesystem {
  enum Capability {
    READ,
    WRITE,
    ENTRIES,
    MKDIR,
    DELETE,
    COPY,
    MOVE,
    APPEND,
    ATOMIC_MOVE,
    PRESERVE_MODIFIED,
    REVISION_CHECK,
    TRANSACTIONS,
    WATCH;

    public String keyword() {
      return name().toLowerCase(java.util.Locale.ROOT).replace('_', '-');
    }
  }

  enum EntryType {
    FILE,
    DIRECTORY,
    SYMLINK,
    OTHER;

    public String keyword() {
      return name().toLowerCase(java.util.Locale.ROOT);
    }
  }

  enum WriteMode {
    CREATE,
    REPLACE,
    APPEND
  }

  record Capabilities(Set<Capability> values) {
    public Capabilities {
      values =
          values == null || values.isEmpty()
              ? Collections.emptySet()
              : Collections.unmodifiableSet(EnumSet.copyOf(values));
    }

    public static Capabilities of(Capability... values) {
      if (values == null || values.length == 0) return new Capabilities(Set.of());
      return new Capabilities(EnumSet.copyOf(List.of(values)));
    }

    public static Capabilities nativeReadWrite() {
      return of(
          Capability.READ,
          Capability.WRITE,
          Capability.ENTRIES,
          Capability.MKDIR,
          Capability.DELETE,
          Capability.COPY,
          Capability.MOVE,
          Capability.APPEND);
    }

    public boolean contains(Capability capability) {
      return values.contains(capability);
    }
  }

  record Descriptor(
      String kind,
      String display,
      boolean readOnly,
      Capabilities capabilities,
      String revision,
      Map<String, Object> extensions) {
    public Descriptor {
      kind = requireText(kind, "filesystem kind");
      display = requireText(display, "filesystem display");
      capabilities = Objects.requireNonNull(capabilities, "filesystem capabilities");
      extensions = immutableMap(extensions);
    }
  }

  record Entry(
      String path,
      String name,
      EntryType type,
      Long size,
      Long modifiedAt,
      String id,
      String revision,
      Capabilities capabilities,
      Map<String, Object> extensions) {
    public Entry {
      path = HaraLogicalPath.normalise(path);
      name = Objects.requireNonNull(name, "filesystem entry name");
      type = Objects.requireNonNull(type, "filesystem entry type");
      if (size != null && size < 0) {
        throw new IllegalArgumentException("filesystem size is negative");
      }
      extensions = immutableMap(extensions);
    }
  }

  record PageRequest(String token, int limit) {
    public static final int DEFAULT_LIMIT = 256;

    public PageRequest {
      if (limit <= 0) throw new IllegalArgumentException("filesystem page limit must be positive");
    }

    public static PageRequest first() {
      return new PageRequest(null, DEFAULT_LIMIT);
    }
  }

  record EntryPage(List<Entry> entries, String nextToken) {
    public EntryPage {
      ArrayList<Entry> sorted = new ArrayList<>(Objects.requireNonNull(entries, "entries"));
      sorted.sort(java.util.Comparator.comparing(Entry::path));
      entries = List.copyOf(sorted);
    }
  }

  record WriteOptions(WriteMode mode, boolean parents) {
    public WriteOptions {
      mode = Objects.requireNonNull(mode, "write mode");
    }
  }

  record MkdirOptions(boolean parents, boolean existsOk) {}

  record DeleteOptions(boolean missingOk) {}

  record CopyOptions(boolean replace, boolean parents, boolean preserveModified) {}

  record MoveOptions(boolean replace, boolean parents, boolean atomic) {}

  record MutationContext(String expectedRevision, String expectedTargetRevision) {
    public static MutationContext none() {
      return new MutationContext(null, null);
    }

    public boolean required() {
      return expectedRevision != null || expectedTargetRevision != null;
    }
  }

  record Mutation(
      String path,
      String revision,
      String mountRevision,
      Map<String, Object> extensions) {
    public Mutation {
      path = HaraLogicalPath.normalise(path);
      extensions = immutableMap(extensions);
    }

    public static Mutation path(String path) {
      return new Mutation(path, null, null, Map.of());
    }
  }

  /** Shared cancellation and monotonic deadline state for one provider operation. */
  final class CallContext {
    private static final class State {
      final AtomicBoolean cancelled = new AtomicBoolean();
      final CopyOnWriteArrayList<Runnable> cancellationHooks =
          new CopyOnWriteArrayList<>();
    }

    private final boolean hasDeadline;
    private final long deadlineNanos;
    private final String traceId;
    private final State state;

    private CallContext(
        boolean hasDeadline, long deadlineNanos, String traceId, State state) {
      this.hasDeadline = hasDeadline;
      this.deadlineNanos = deadlineNanos;
      this.traceId = traceId;
      this.state = state;
    }

    public static CallContext create() {
      return new CallContext(false, 0L, null, new State());
    }

    public static CallContext until(Instant deadline) {
      Objects.requireNonNull(deadline, "deadline");
      Duration remaining = Duration.between(Instant.now(), deadline);
      return within(remaining.isNegative() ? Duration.ZERO : remaining);
    }

    public static CallContext within(Duration timeout) {
      Objects.requireNonNull(timeout, "timeout");
      long durationNanos;
      try {
        durationNanos = timeout.toNanos();
      } catch (ArithmeticException ignored) {
        durationNanos = Long.MAX_VALUE;
      }
      durationNanos = Math.max(0L, durationNanos);
      long now = System.nanoTime();
      long deadline =
          durationNanos >= Long.MAX_VALUE - now ? Long.MAX_VALUE : now + durationNanos;
      return new CallContext(true, deadline, null, new State());
    }

    public CallContext withTraceId(String traceId) {
      return new CallContext(
          hasDeadline, deadlineNanos, requireText(traceId, "trace id"), state);
    }

    public String traceId() {
      return traceId;
    }

    public boolean hasDeadline() {
      return hasDeadline;
    }

    public long remainingNanos() {
      if (!hasDeadline) return Long.MAX_VALUE;
      return Math.max(0L, deadlineNanos - System.nanoTime());
    }

    public boolean cancelled() {
      return state.cancelled.get();
    }

    public boolean cancel() {
      if (!state.cancelled.compareAndSet(false, true)) return false;
      for (Runnable hook : state.cancellationHooks) {
        try {
          hook.run();
        } catch (RuntimeException ignored) {
          // Cancellation remains terminal even when a provider hook misbehaves.
        }
      }
      state.cancellationHooks.clear();
      return true;
    }

    public AutoCloseable onCancel(Runnable hook) {
      Objects.requireNonNull(hook, "cancellation hook");
      if (cancelled()) {
        hook.run();
        return () -> {};
      }
      state.cancellationHooks.add(hook);
      if (cancelled() && state.cancellationHooks.remove(hook)) hook.run();
      return () -> state.cancellationHooks.remove(hook);
    }

    public void check(String provider, String operation, String path, String target) {
      if (cancelled()) {
        throw FilesystemException.cancelled(provider, operation, path, target);
      }
      if (hasDeadline && remainingNanos() == 0L) {
        throw FilesystemException.timeout(provider, operation, path, target);
      }
    }
  }

  Descriptor descriptor();

  default Capabilities capabilities() {
    return descriptor().capabilities();
  }

  CompletionStage<Entry> stat(CallContext context, String path);

  CompletionStage<byte[]> read(CallContext context, String path);

  CompletionStage<Mutation> write(
      CallContext context,
      String path,
      byte[] bytes,
      WriteOptions options,
      MutationContext mutation);

  CompletionStage<EntryPage> entriesPage(
      CallContext context, String path, PageRequest request);

  CompletionStage<Mutation> mkdir(
      CallContext context, String path, MkdirOptions options, MutationContext mutation);

  CompletionStage<Mutation> delete(
      CallContext context, String path, DeleteOptions options, MutationContext mutation);

  CompletionStage<Mutation> copy(
      CallContext context,
      String source,
      String target,
      CopyOptions options,
      MutationContext mutation);

  CompletionStage<Mutation> move(
      CallContext context,
      String source,
      String target,
      MoveOptions options,
      MutationContext mutation);

  CompletionStage<Void> close(CallContext context);

  private static String requireText(String value, String label) {
    if (value == null || value.isBlank()) {
      throw new IllegalArgumentException(label + " is required");
    }
    return value;
  }

  private static Map<String, Object> immutableMap(Map<String, Object> values) {
    return values == null || values.isEmpty() ? Map.of() : Map.copyOf(values);
  }
}
