package hara.truffle;

import java.util.ArrayList;
import java.util.Collections;
import java.util.List;
import java.util.Map;
import java.util.Objects;
import java.util.Set;
import java.util.TreeMap;
import java.util.TreeSet;

/** Portable Java contracts for Runtime-owned Hara instrumentation. */
final class InstrumentationModel {
  static final String PROTOCOL = "hara.instrumentation/0-alpha";
  static final String EVENT_SCHEMA = "hara.instrumentation.event/0-alpha";
  static final int DEFAULT_QUEUE_CAPACITY = 256;
  static final int MAX_QUEUE_CAPACITY = 65_536;
  static final int MAX_PROJECTION_ITEMS = 65_536;
  static final int MAX_PROJECTION_DEPTH = 256;
  static final int MAX_PROJECTION_BYTES = 16 * 1024 * 1024;

  private InstrumentationModel() {}

  enum InstrumentMode {
    PASSIVE,
    CONTROL,
    TRANSFORM
  }

  enum TargetKind {
    INTERPRETER("interpreter"),
    HBC("hbc"),
    WHOLE_WASM("whole-wasm");

    private final String display;

    TargetKind(String display) {
      this.display = display;
    }

    @Override
    public String toString() {
      return display;
    }
  }

  record RuntimeBackend(String value) implements Comparable<RuntimeBackend> {
    RuntimeBackend {
      value = requiredId(value, "runtime backend");
    }

    @Override
    public int compareTo(RuntimeBackend other) {
      return value.compareTo(other.value);
    }

    @Override
    public String toString() {
      return value;
    }
  }

  enum Capability {
    EVENT_SEMANTIC_BOUNDARY,
    EVENT_INSTRUCTION,
    EVENT_CALL,
    EVENT_EXCEPTION,
    EVENT_EFFECT,
    EVENT_SUSPENSION,
    EVENT_LIFECYCLE,
    INSPECT_SOURCE_LOCATION,
    INSPECT_CURRENT_FRAME,
    INSPECT_FRAMES,
    INSPECT_LOCALS,
    INSPECT_STACK,
    INSPECT_VALUE_PREVIEW,
    INSPECT_SNAPSHOT,
    CONTROL_PAUSE,
    CONTROL_SINGLE_STEP,
    CONTROL_RESUME,
    CONTROL_SETTLE,
    CONTROL_TERMINATE,
    TRANSFORM_HALC,
    TRANSFORM_HBC,
    RETRANSFORM_HALC,
    RETRANSFORM_HBC;

    boolean isControl() {
      return switch (this) {
        case CONTROL_PAUSE,
            CONTROL_SINGLE_STEP,
            CONTROL_RESUME,
            CONTROL_SETTLE,
            CONTROL_TERMINATE -> true;
        default -> false;
      };
    }

    boolean isTransform() {
      return switch (this) {
        case TRANSFORM_HALC, TRANSFORM_HBC, RETRANSFORM_HALC, RETRANSFORM_HBC -> true;
        default -> false;
      };
    }
  }

  enum EventKind {
    SEMANTIC_BOUNDARY(Capability.EVENT_SEMANTIC_BOUNDARY),
    INSTRUCTION_EXECUTE(Capability.EVENT_INSTRUCTION),
    CALL_ENTER(Capability.EVENT_CALL),
    CALL_RETURN(Capability.EVENT_CALL),
    EXCEPTION_RAISE(Capability.EVENT_EXCEPTION),
    EXCEPTION_UNWIND(Capability.EVENT_EXCEPTION),
    VAR_SET(Capability.EVENT_EFFECT),
    FIELD_SET(Capability.EVENT_EFFECT),
    PROMISE_SUSPEND(Capability.EVENT_SUSPENSION),
    PROMISE_RESUME(Capability.EVENT_SUSPENSION),
    MACHINE_SUSPEND(Capability.EVENT_SUSPENSION),
    MACHINE_RESUME(Capability.EVENT_SUSPENSION),
    PROTOCOL_CALL(Capability.EVENT_SEMANTIC_BOUNDARY),
    EXECUTION_TERMINAL(Capability.EVENT_LIFECYCLE);

    private final Capability requiredCapability;

    EventKind(Capability requiredCapability) {
      this.requiredCapability = requiredCapability;
    }

    Capability requiredCapability() {
      return requiredCapability;
    }

    boolean supports(TargetKind target) {
      return switch (target) {
        case INTERPRETER ->
            switch (this) {
              case SEMANTIC_BOUNDARY,
                  CALL_ENTER,
                  CALL_RETURN,
                  EXCEPTION_RAISE,
                  VAR_SET,
                  FIELD_SET,
                  PROMISE_SUSPEND,
                  PROMISE_RESUME,
                  EXECUTION_TERMINAL -> true;
              default -> false;
            };
        case HBC ->
            switch (this) {
              case INSTRUCTION_EXECUTE,
                  CALL_ENTER,
                  CALL_RETURN,
                  EXCEPTION_UNWIND,
                  MACHINE_SUSPEND,
                  MACHINE_RESUME,
                  EXECUTION_TERMINAL -> true;
              default -> false;
            };
        case WHOLE_WASM -> this == PROTOCOL_CALL || this == EXECUTION_TERMINAL;
      };
    }
  }

  record ProjectionLimits(int maxItems, int maxDepth, int maxBytes) {
    static ProjectionLimits defaults() {
      return new ProjectionLimits(256, 16, 64 * 1024);
    }

    ProjectionLimits {
      if (maxItems <= 0 || maxItems > MAX_PROJECTION_ITEMS) {
        throw new IllegalArgumentException("INVALID_PROJECTION_ITEMS " + maxItems);
      }
      if (maxDepth <= 0 || maxDepth > MAX_PROJECTION_DEPTH) {
        throw new IllegalArgumentException("INVALID_PROJECTION_DEPTH " + maxDepth);
      }
      if (maxBytes <= 0 || maxBytes > MAX_PROJECTION_BYTES) {
        throw new IllegalArgumentException("INVALID_PROJECTION_BYTES " + maxBytes);
      }
    }
  }

  record ProjectionRequest(
      boolean sourceLocation,
      ProjectionLimits currentFrame,
      ProjectionLimits frames,
      ProjectionLimits locals,
      ProjectionLimits stack,
      ProjectionLimits valuePreview,
      ProjectionLimits machineSnapshot) {
    static ProjectionRequest none() {
      return new ProjectionRequest(false, null, null, null, null, null, null);
    }

    Set<Capability> requiredCapabilities() {
      TreeSet<Capability> required = new TreeSet<>();
      if (sourceLocation) required.add(Capability.INSPECT_SOURCE_LOCATION);
      if (currentFrame != null) required.add(Capability.INSPECT_CURRENT_FRAME);
      if (frames != null) required.add(Capability.INSPECT_FRAMES);
      if (locals != null) required.add(Capability.INSPECT_LOCALS);
      if (stack != null) required.add(Capability.INSPECT_STACK);
      if (valuePreview != null) required.add(Capability.INSPECT_VALUE_PREVIEW);
      if (machineSnapshot != null) required.add(Capability.INSPECT_SNAPSHOT);
      return Collections.unmodifiableSet(required);
    }
  }

  record EventDelivery(int capacity) {
    static EventDelivery queue(int capacity) {
      return new EventDelivery(capacity);
    }

    static EventDelivery defaults() {
      return new EventDelivery(DEFAULT_QUEUE_CAPACITY);
    }

    EventDelivery {
      if (capacity <= 0 || capacity > MAX_QUEUE_CAPACITY) {
        throw new IllegalArgumentException("INVALID_EVENT_QUEUE_CAPACITY " + capacity);
      }
    }
  }

  record InstrumentFilter(
      String sessionId,
      Set<String> targetIds,
      Set<TargetKind> targetKinds,
      Set<RuntimeBackend> backends) {
    static InstrumentFilter all() {
      return new InstrumentFilter(null, Set.of(), Set.of(), Set.of());
    }

    InstrumentFilter {
      if (sessionId != null) sessionId = requiredId(sessionId, "filter session");
      targetIds = immutableIds(targetIds, "filter target");
      targetKinds = immutableSet(targetKinds);
      backends = immutableSet(backends);
    }

    boolean matches(TargetDescriptor target) {
      return (sessionId == null || sessionId.equals(target.sessionId()))
          && (targetIds.isEmpty() || targetIds.contains(target.targetId()))
          && (targetKinds.isEmpty() || targetKinds.contains(target.kind()))
          && (backends.isEmpty() || backends.contains(target.backend()));
    }
  }

  record InstrumentRegistration(
      String instrumentId,
      String sessionId,
      InstrumentMode mode,
      Set<Capability> capabilities,
      Set<EventKind> events,
      InstrumentFilter filter,
      ProjectionRequest projection,
      EventDelivery delivery) {
    InstrumentRegistration {
      instrumentId = requiredId(instrumentId, "instrument");
      sessionId = requiredId(sessionId, "instrument session");
      mode = Objects.requireNonNull(mode, "mode");
      capabilities = immutableSet(capabilities);
      events = immutableSet(events);
      filter = filter == null ? InstrumentFilter.all() : filter;
      projection = projection == null ? ProjectionRequest.none() : projection;
      delivery = delivery == null ? EventDelivery.defaults() : delivery;
      validateRegistration(mode, capabilities, events, projection);
    }
  }

  record TargetDescriptor(
      String targetId,
      String sessionId,
      TargetKind kind,
      RuntimeBackend backend,
      Set<Capability> capabilities) {
    TargetDescriptor {
      targetId = requiredId(targetId, "target");
      sessionId = requiredId(sessionId, "target session");
      kind = Objects.requireNonNull(kind, "kind");
      backend = Objects.requireNonNull(backend, "backend");
      capabilities = immutableSet(capabilities);
      if (capabilities.stream().anyMatch(Capability::isTransform)) {
        throw new IllegalArgumentException("TRANSFORM_CAPABILITY_DEFERRED");
      }
    }
  }

  record InstrumentHandle(String instrumentId, long generation) {
    InstrumentHandle {
      instrumentId = requiredId(instrumentId, "instrument handle");
      if (generation < 0) throw new IllegalArgumentException("INVALID_INSTRUMENT_GENERATION");
    }
  }

  record TargetHandle(String targetId, long generation) {
    TargetHandle {
      targetId = requiredId(targetId, "target handle");
      if (generation < 0) throw new IllegalArgumentException("INVALID_TARGET_GENERATION");
    }
  }

  enum EventPhase {
    LIVE,
    REPLAY
  }

  enum InstrumentDirective {
    CONTINUE,
    SUSPEND,
    STEP_NEXT,
    SETTLE,
    TERMINATE
  }

  record SourceSpan(int start, int end) {
    SourceSpan {
      if (start < 0 || end < start) throw new IllegalArgumentException("INVALID_SOURCE_SPAN");
    }
  }

  record EventLocation(
      String sourceId,
      List<Integer> formPath,
      SourceSpan span,
      String function,
      Integer instructionPointer) {
    EventLocation {
      if (sourceId != null && sourceId.isBlank()) {
        throw new IllegalArgumentException("EMPTY_SOURCE_ID");
      }
      if (function != null && function.isBlank()) {
        throw new IllegalArgumentException("EMPTY_FUNCTION");
      }
      if (instructionPointer != null && instructionPointer < 0) {
        throw new IllegalArgumentException("INVALID_INSTRUCTION_POINTER");
      }
      if (formPath == null || formPath.isEmpty()) {
        formPath = List.of();
      } else {
        ArrayList<Integer> frozen = new ArrayList<>(formPath.size());
        for (Integer value : formPath) {
          if (value == null || value < 0) throw new IllegalArgumentException("INVALID_FORM_PATH");
          frozen.add(value);
        }
        formPath = List.copyOf(frozen);
      }
    }
  }

  record EventEnvelope(
      String schema,
      String protocol,
      String instrumentId,
      RuntimeBackend runtime,
      String sessionId,
      String targetId,
      TargetKind targetKind,
      long generation,
      long sequence,
      EventPhase phase,
      EventKind event,
      EventLocation location,
      Map<String, String> data) {
    EventEnvelope {
      if (!EVENT_SCHEMA.equals(schema)) {
        throw new IllegalArgumentException("UNSUPPORTED_EVENT_SCHEMA");
      }
      if (!PROTOCOL.equals(protocol)) {
        throw new IllegalArgumentException("UNSUPPORTED_INSTRUMENTATION_PROTOCOL");
      }
      instrumentId = requiredId(instrumentId, "event instrument");
      runtime = Objects.requireNonNull(runtime, "runtime");
      sessionId = requiredId(sessionId, "event session");
      targetId = requiredId(targetId, "event target");
      targetKind = Objects.requireNonNull(targetKind, "targetKind");
      if (generation < 0) throw new IllegalArgumentException("INVALID_EVENT_GENERATION");
      if (sequence <= 0) throw new IllegalArgumentException("INVALID_EVENT_SEQUENCE");
      phase = Objects.requireNonNull(phase, "phase");
      event = Objects.requireNonNull(event, "event");
      if (!event.supports(targetKind)) throw new IllegalArgumentException("EVENT_TARGET_MISMATCH");
      if (location != null) {
        if (targetKind == TargetKind.INTERPRETER && location.instructionPointer() != null) {
          throw new IllegalArgumentException("INTERPRETER_INSTRUCTION_POINTER");
        }
        if (targetKind == TargetKind.HBC
            && event == EventKind.INSTRUCTION_EXECUTE
            && !location.formPath().isEmpty()) {
          throw new IllegalArgumentException("HBC_INSTRUCTION_FORM_PATH");
        }
      }
      data = immutableMap(data);
    }
  }

  record PortableProjection(String kind, Map<String, String> fields) {
    PortableProjection {
      kind = requiredId(kind, "projection kind");
      fields = immutableMap(fields);
    }
  }

  record EventProjection(
      PortableProjection currentFrame,
      PortableProjection frames,
      PortableProjection locals,
      PortableProjection stack,
      PortableProjection valuePreview,
      PortableProjection machineSnapshot) {
    static EventProjection none() {
      return new EventProjection(null, null, null, null, null, null);
    }
  }

  /** One delivered event plus the projections requested by that instrument. */
  record DeliveredEvent(EventEnvelope envelope, EventProjection projection, long droppedBefore) {
    DeliveredEvent {
      envelope = Objects.requireNonNull(envelope, "envelope");
      projection = projection == null ? EventProjection.none() : projection;
      if (droppedBefore < 0) throw new IllegalArgumentException("INVALID_EVENT_DROPPED_BEFORE");
    }

    String instrumentId() {
      return envelope.instrumentId();
    }

    RuntimeBackend runtime() {
      return envelope.runtime();
    }

    String sessionId() {
      return envelope.sessionId();
    }

    String targetId() {
      return envelope.targetId();
    }

    TargetKind targetKind() {
      return envelope.targetKind();
    }

    long generation() {
      return envelope.generation();
    }

    long sequence() {
      return envelope.sequence();
    }

    EventPhase phase() {
      return envelope.phase();
    }

    EventKind event() {
      return envelope.event();
    }

    EventLocation location() {
      return envelope.location();
    }

    Map<String, String> data() {
      return envelope.data();
    }
  }

  record EventBatch(List<DeliveredEvent> events, long droppedSinceDrain, long droppedTotal) {
    EventBatch {
      events = events == null ? List.of() : List.copyOf(events);
      if (droppedSinceDrain < 0 || droppedTotal < droppedSinceDrain) {
        throw new IllegalArgumentException("INVALID_EVENT_DROP_COUNTS");
      }
    }
  }

  record ControlLease(InstrumentHandle instrument, TargetHandle target, long generation) {
    ControlLease {
      instrument = Objects.requireNonNull(instrument, "instrument");
      target = Objects.requireNonNull(target, "target");
      if (generation < 0) throw new IllegalArgumentException("INVALID_LEASE_GENERATION");
    }
  }

  private static void validateRegistration(
      InstrumentMode mode,
      Set<Capability> capabilities,
      Set<EventKind> events,
      ProjectionRequest projection) {
    for (EventKind event : events) {
      if (!capabilities.contains(event.requiredCapability())) {
        throw new IllegalArgumentException("EVENT_CAPABILITY_REQUIRED " + event);
      }
    }
    if (!capabilities.containsAll(projection.requiredCapabilities())) {
      throw new IllegalArgumentException("PROJECTION_CAPABILITY_REQUIRED");
    }
    if (mode != InstrumentMode.CONTROL && capabilities.stream().anyMatch(Capability::isControl)) {
      throw new IllegalArgumentException("CONTROL_MODE_REQUIRED");
    }
    if (mode != InstrumentMode.TRANSFORM
        && capabilities.stream().anyMatch(Capability::isTransform)) {
      throw new IllegalArgumentException("TRANSFORM_MODE_REQUIRED");
    }
  }

  private static String requiredId(String value, String label) {
    if (value == null || value.isBlank()) {
      throw new IllegalArgumentException(
          "EMPTY_" + label.toUpperCase().replace(' ', '_'));
    }
    return value;
  }

  private static Set<String> immutableIds(Set<String> values, String label) {
    if (values == null || values.isEmpty()) return Set.of();
    TreeSet<String> result = new TreeSet<>();
    for (String value : values) result.add(requiredId(value, label));
    return Collections.unmodifiableSet(result);
  }

  private static <T extends Comparable<? super T>> Set<T> immutableSet(Set<T> values) {
    if (values == null || values.isEmpty()) return Set.of();
    TreeSet<T> result = new TreeSet<>();
    for (T value : values) result.add(Objects.requireNonNull(value, "set value"));
    return Collections.unmodifiableSet(result);
  }

  private static Map<String, String> immutableMap(Map<String, String> values) {
    if (values == null || values.isEmpty()) return Map.of();
    TreeMap<String, String> result = new TreeMap<>();
    for (Map.Entry<String, String> entry : values.entrySet()) {
      result.put(
          requiredId(entry.getKey(), "event data key"),
          Objects.requireNonNull(entry.getValue(), "event data value"));
    }
    return Collections.unmodifiableMap(result);
  }
}
