package hara.truffle;

import com.oracle.truffle.api.CompilerDirectives.TruffleBoundary;
import hara.lang.base.NumUtils;
import hara.lang.data.Keyword;
import hara.lang.data.Symbol;
import hara.lang.protocol.ILinearType;
import hara.lang.protocol.IMapType;
import hara.lang.protocol.ISetType;
import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.Collections;
import java.util.List;
import java.util.LinkedHashMap;
import java.util.Map;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.concurrent.atomic.AtomicLong;
import java.util.function.Supplier;

/** Bounded, runtime-neutral diagnostic journal for semantic HAL operations. */
public final class EvaluationJournal {
  public static final String SCHEMA = "hal.evaluation-journal/0-alpha";
  public static final Limits DEFAULT_LIMITS = new Limits(10_000, 100, 1_000);

  private static final AtomicInteger ENABLED = new AtomicInteger();
  private static final AtomicLong NEXT_JOURNAL = new AtomicLong(1);
  private static final ThreadLocal<Collector> ACTIVE = new ThreadLocal<>();

  private EvaluationJournal() {}

  public record Limits(int maxEvents, int maxDepth, int maxValueCharacters) {
    public Limits {
      if (maxEvents < 0 || maxDepth < 0 || maxValueCharacters < 0) {
        throw new IllegalArgumentException("Journal limits must be non-negative");
      }
    }
  }

  public record ValuePreview(String type, String display, boolean truncated) {}

  public record Event(
      long id,
      long sequence,
      String kind,
      Long operation,
      Long parent,
      int depth,
      String name,
      List<ValuePreview> values,
      String message) {}

  public record Journal(
      String schema,
      String id,
      String status,
      List<Event> events,
      ValuePreview result,
      String error) {
    /** Returns ordinary HAL-shaped data using the portable journal field names. */
    public Map<Object, Object> portableData() {
      Map<Object, Object> data = new LinkedHashMap<>();
      data.put(keyword("journal/schema"), schema);
      data.put(keyword("journal/id"), id);
      data.put(keyword("journal/status"), keyword(status));
      List<Map<Object, Object>> encodedEvents = new ArrayList<>();
      for (Event event : events) encodedEvents.add(eventData(event));
      data.put(keyword("journal/events"), List.copyOf(encodedEvents));
      if (result != null) data.put(keyword("journal/result"), previewData(result));
      if (error != null) data.put(keyword("journal/error"), error);
      return Map.copyOf(data);
    }
  }

  /** Collects one evaluation. Failures are represented in the returned journal. */
  public static Journal collect(Supplier<?> evaluation) {
    return collect(DEFAULT_LIMITS, evaluation);
  }

  public static Journal collect(Limits limits, Supplier<?> evaluation) {
    if (ACTIVE.get() != null) throw new IllegalStateException("Nested journals are not supported");
    Collector collector = new Collector(NEXT_JOURNAL.getAndIncrement(), limits);
    ACTIVE.set(collector);
    ENABLED.incrementAndGet();
    collector.record("evaluation/start", null, null, 0, null, List.of(), null);
    try {
      Object result = evaluation.get();
      return collector.finish(result);
    } catch (RuntimeException failure) {
      return collector.fail(failure.getMessage() == null ? failure.getClass().getName() : failure.getMessage());
    } finally {
      ACTIVE.remove();
      ENABLED.decrementAndGet();
    }
  }

  /** Fast disabled path used from Truffle roots. Zero means no ThreadLocal lookup. */
  @TruffleBoundary
  public static long enter(String name, Object[] arguments, int offset) {
    if (ENABLED.get() == 0) return 0;
    Collector collector = ACTIVE.get();
    return collector == null ? 0 : collector.enter(name, arguments, offset);
  }

  @TruffleBoundary
  public static void returned(long operation, Object result) {
    if (operation == 0 || ENABLED.get() == 0) return;
    Collector collector = ACTIVE.get();
    if (collector != null) collector.returned(operation, result);
  }

  @TruffleBoundary
  public static void failed(long operation, RuntimeException failure) {
    if (operation == 0 || ENABLED.get() == 0) return;
    Collector collector = ACTIVE.get();
    if (collector != null) collector.leave(operation);
  }

  public static void macro(String name, Object source, Object expansion) {
    if (ENABLED.get() == 0) return;
    Collector collector = ACTIVE.get();
    if (collector != null) collector.macro(name, source, expansion);
  }

  private static final class Collector {
    private final long journalId;
    private final Limits limits;
    private final List<Event> events = new ArrayList<>();
    private final ArrayDeque<Long> stack = new ArrayDeque<>();
    private long nextEvent = 1;
    private long nextOperation = 1;
    private boolean truncated;

    Collector(long journalId, Limits limits) {
      this.journalId = journalId;
      this.limits = limits;
    }

    long enter(String name, Object[] arguments, int offset) {
      long operation = nextOperation++;
      Long parent = stack.peekLast();
      int depth = stack.size();
      List<ValuePreview> previews = new ArrayList<>();
      for (int index = offset; index < arguments.length; index++) previews.add(preview(arguments[index]));
      record("operation/enter", operation, parent, depth, name, previews, null);
      stack.addLast(operation);
      return operation;
    }

    void returned(long operation, Object result) {
      leave(operation);
      record("operation/return", operation, null, stack.size(), null, List.of(preview(result)), null);
    }

    void leave(long operation) {
      if (!stack.isEmpty() && stack.peekLast() == operation) stack.removeLast();
    }

    void macro(String name, Object source, Object expansion) {
      record(
          "macro/expand",
          null,
          stack.peekLast(),
          stack.size(),
          name,
          List.of(preview(source), preview(expansion)),
          null);
    }

    void record(
        String kind,
        Long operation,
        Long parent,
        int depth,
        String name,
        List<ValuePreview> values,
        String message) {
      if (truncated) return;
      if (depth > limits.maxDepth() || events.size() >= limits.maxEvents()) {
        truncated = true;
        events.add(
            new Event(
                nextEvent,
                nextEvent++,
                "journal/truncated",
                null,
                null,
                0,
                null,
                List.of(),
                "journal limit reached; evaluation continued"));
        return;
      }
      events.add(
          new Event(nextEvent, nextEvent++, kind, operation, parent, depth, name, List.copyOf(values), message));
    }

    Journal finish(Object result) {
      return new Journal(
          SCHEMA,
          "journal-" + journalId,
          truncated ? "truncated" : "ok",
          List.copyOf(events),
          preview(result),
          null);
    }

    Journal fail(String error) {
      record("evaluation/error", null, stack.peekLast(), stack.size(), null, List.of(), error);
      return new Journal(
          SCHEMA,
          "journal-" + journalId,
          truncated ? "truncated" : "error",
          List.copyOf(events),
          null,
          error);
    }

    ValuePreview preview(Object value) {
      String display;
      try {
        display = String.valueOf(value);
      } catch (RuntimeException unreadable) {
        return new ValuePreview("unreadable", "<unreadable>", false);
      }
      int limit = limits.maxValueCharacters();
      int characters = display.codePointCount(0, display.length());
      if (characters <= limit) return new ValuePreview(type(value), display, false);
      int retained = Math.max(0, limit - 1);
      int end = display.offsetByCodePoints(0, retained);
      return new ValuePreview(type(value), display.substring(0, end) + "…", true);
    }
  }

  private static String type(Object value) {
    if (value == null) return "nil";
    if (value instanceof Boolean) return "boolean";
    Object raw = HaraBox.unwrap(value);
    if (NumUtils.isLongValue(raw)) return "long";
    if (NumUtils.isBigIntegerValue(raw)) return "bigint";
    if (value instanceof Float || value instanceof Double) return "float";
    if (value instanceof hara.lang.data.HaraCharacter || value instanceof Character)
      return "character";
    if (value instanceof String) return "string";
    if (value instanceof Symbol) return "symbol";
    if (value instanceof Keyword) return "keyword";
    if (value instanceof hara.lang.data.List<?>) return "list";
    if (value instanceof hara.lang.data.Vector<?>) return "vector";
    if (value instanceof IMapType<?, ?> || value instanceof Map<?, ?>) return "map";
    if (value instanceof ISetType<?> || value instanceof java.util.Set<?>) return "set";
    if (value instanceof HaraFunction) return "function";
    if (value instanceof ILinearType<?>) return "vector";
    return "host-handle";
  }

  private static Map<Object, Object> eventData(Event event) {
    Map<Object, Object> data = new LinkedHashMap<>();
    data.put(keyword("event/id"), event.id());
    data.put(keyword("event/sequence"), event.sequence());
    data.put(keyword("event/kind"), keyword(event.kind()));
    if (event.kind().startsWith("operation/")) {
      data.put(keyword("operation/id"), event.operation());
      data.put(keyword("operation/parent"), event.parent());
      data.put(keyword("operation/depth"), event.depth());
      data.put(keyword("operation/name"), event.name());
      if (event.kind().equals("operation/enter")) {
        data.put(keyword("operation/arguments"), event.values().stream().map(EvaluationJournal::previewData).toList());
      } else {
        data.put(keyword("operation/result"), event.values().isEmpty() ? null : previewData(event.values().get(0)));
      }
    } else if (event.kind().equals("macro/expand")) {
      data.put(keyword("macro/name"), event.name());
      data.put(keyword("macro/values"), event.values().stream().map(EvaluationJournal::previewData).toList());
    } else if (event.kind().equals("evaluation/error")) {
      data.put(keyword("error/message"), event.message());
    } else if (event.kind().equals("journal/truncated")) {
      data.put(keyword("truncation/reason"), event.message());
    }
    return Collections.unmodifiableMap(data);
  }

  private static Map<Object, Object> previewData(ValuePreview preview) {
    return Map.of(
        keyword("value/type"), preview.type(),
        keyword("value/display"), preview.display(),
        keyword("value/truncated"), preview.truncated());
  }

  private static Keyword keyword(String value) {
    return Keyword.create(value);
  }
}
