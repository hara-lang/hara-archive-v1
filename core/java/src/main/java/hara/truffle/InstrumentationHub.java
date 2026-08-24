package hara.truffle;

import hara.truffle.InstrumentationException.Code;
import hara.truffle.InstrumentationModel.Capability;
import hara.truffle.InstrumentationModel.ControlLease;
import hara.truffle.InstrumentationModel.EventBatch;
import hara.truffle.InstrumentationModel.EventEnvelope;
import hara.truffle.InstrumentationModel.EventProjection;
import hara.truffle.InstrumentationModel.EventKind;
import hara.truffle.InstrumentationModel.EventLocation;
import hara.truffle.InstrumentationModel.EventPhase;
import hara.truffle.InstrumentationModel.InstrumentDirective;
import hara.truffle.InstrumentationModel.InstrumentHandle;
import hara.truffle.InstrumentationModel.InstrumentMode;
import hara.truffle.InstrumentationModel.InstrumentRegistration;
import hara.truffle.InstrumentationModel.TargetDescriptor;
import hara.truffle.InstrumentationModel.TargetHandle;
import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.Collections;
import java.util.Iterator;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Objects;
import java.util.Set;
import java.util.TreeSet;
import java.util.UUID;

/** Runtime-owned registration, delivery, fencing, and control lifecycle. */
final class InstrumentationHub implements AutoCloseable {
  private final String runtimeId = UUID.randomUUID().toString();
  private final LinkedHashMap<String, InstrumentState> instruments = new LinkedHashMap<>();
  private final LinkedHashMap<String, TargetState> targets = new LinkedHashMap<>();
  private final LinkedHashMap<AttachmentKey, Attachment> attachments = new LinkedHashMap<>();
  private final LinkedHashMap<String, Long> instrumentGenerations = new LinkedHashMap<>();
  private final LinkedHashMap<String, Long> targetGenerations = new LinkedHashMap<>();
  private boolean closed;

  private static final class InstrumentState {
    final InstrumentHandle handle;
    final InstrumentRegistration registration;
    final ArrayDeque<hara.truffle.InstrumentationModel.DeliveredEvent> queue =
        new ArrayDeque<>();
    long droppedSinceDrain;
    long droppedTotal;

    InstrumentState(InstrumentHandle handle, InstrumentRegistration registration) {
      this.handle = handle;
      this.registration = registration;
    }
  }

  private static final class TargetState {
    final TargetHandle handle;
    final TargetDescriptor descriptor;
    final ArrayDeque<InstrumentDirective> directives = new ArrayDeque<>();
    long sequence;
    long nextLeaseGeneration;
    LeaseState lease;

    TargetState(TargetHandle handle, TargetDescriptor descriptor) {
      this.handle = handle;
      this.descriptor = descriptor;
    }
  }

  private record AttachmentKey(InstrumentHandle instrument, TargetHandle target) {}

  private record Attachment(AttachmentKey key) {}

  private record LeaseState(ControlLease lease) {}

  String runtimeId() {
    return runtimeId;
  }

  synchronized InstrumentHandle registerInstrument(InstrumentRegistration registration) {
    requireOpen();
    Objects.requireNonNull(registration, "registration");
    if (registration.mode() == InstrumentMode.TRANSFORM
        || registration.capabilities().stream().anyMatch(Capability::isTransform)) {
      throw failure(
          Code.TRANSFORM_DEFERRED,
          "TRANSFORM_CAPABILITY_DEFERRED " + registration.instrumentId(),
          Map.of("instrument", registration.instrumentId()));
    }
    if (instruments.containsKey(registration.instrumentId())) {
      throw failure(
          Code.INSTRUMENT_EXISTS,
          "INSTRUMENT_EXISTS " + registration.instrumentId(),
          Map.of("instrument", registration.instrumentId()));
    }
    long generation =
        instrumentGenerations.getOrDefault(registration.instrumentId(), 0L);
    InstrumentHandle handle =
        new InstrumentHandle(registration.instrumentId(), generation);
    instruments.put(
        registration.instrumentId(), new InstrumentState(handle, registration));
    return handle;
  }

  synchronized void removeInstrument(InstrumentHandle handle) {
    requireOpen();
    InstrumentState state = requireInstrument(handle);
    removeAttachmentsForInstrument(handle);
    releaseLeasesForInstrument(handle);
    instruments.remove(handle.instrumentId());
    instrumentGenerations.put(
        handle.instrumentId(), nextGeneration(handle.generation()));
    state.queue.clear();
  }

  synchronized TargetHandle registerTarget(TargetDescriptor descriptor) {
    requireOpen();
    Objects.requireNonNull(descriptor, "descriptor");
    if (targets.containsKey(descriptor.targetId())) {
      throw failure(
          Code.TARGET_EXISTS,
          "TARGET_EXISTS " + descriptor.targetId(),
          Map.of("target", descriptor.targetId()));
    }
    long generation = targetGenerations.getOrDefault(descriptor.targetId(), 0L);
    TargetHandle handle = new TargetHandle(descriptor.targetId(), generation);
    targets.put(descriptor.targetId(), new TargetState(handle, descriptor));
    return handle;
  }

  synchronized void removeTarget(TargetHandle handle) {
    requireOpen();
    TargetState state = requireTarget(handle);
    removeAttachmentsForTarget(handle);
    state.lease = null;
    state.directives.clear();
    targets.remove(handle.targetId());
    targetGenerations.put(handle.targetId(), nextGeneration(handle.generation()));
  }

  synchronized InstrumentHandle bindInstrument(String instrumentId, long generation) {
    return requireInstrument(new InstrumentHandle(instrumentId, generation)).handle;
  }

  synchronized TargetHandle bindTarget(String targetId, long generation) {
    return requireTarget(new TargetHandle(targetId, generation)).handle;
  }

  synchronized InstrumentRegistration registration(InstrumentHandle handle) {
    return requireInstrument(handle).registration;
  }

  synchronized TargetDescriptor descriptor(TargetHandle handle) {
    return requireTarget(handle).descriptor;
  }

  synchronized void attach(InstrumentHandle instrument, TargetHandle target) {
    requireOpen();
    InstrumentState instrumentState = requireInstrument(instrument);
    TargetState targetState = requireTarget(target);
    validateAttachment(instrumentState.registration, targetState.descriptor);
    AttachmentKey key = new AttachmentKey(instrumentState.handle, targetState.handle);
    attachments.putIfAbsent(key, new Attachment(key));
  }

  synchronized void detach(InstrumentHandle instrument, TargetHandle target) {
    requireOpen();
    InstrumentState instrumentState = requireInstrument(instrument);
    TargetState targetState = requireTarget(target);
    AttachmentKey key = new AttachmentKey(instrumentState.handle, targetState.handle);
    if (attachments.remove(key) == null) {
      throw failure(
          Code.ATTACHMENT_NOT_FOUND,
          "ATTACHMENT_NOT_FOUND "
              + instrument.instrumentId()
              + " "
              + target.targetId(),
          Map.of(
              "instrument", instrument.instrumentId(), "target", target.targetId()));
    }
    if (targetState.lease != null
        && targetState.lease.lease().instrument().equals(instrumentState.handle)) {
      targetState.lease = null;
      targetState.directives.clear();
      targetState.nextLeaseGeneration =
          nextGeneration(targetState.nextLeaseGeneration);
    }
  }

  synchronized boolean attached(InstrumentHandle instrument, TargetHandle target) {
    requireOpen();
    InstrumentState instrumentState = requireInstrument(instrument);
    TargetState targetState = requireTarget(target);
    return attachments.containsKey(
        new AttachmentKey(instrumentState.handle, targetState.handle));
  }

  synchronized int publish(
      TargetHandle target,
      EventKind event,
      EventPhase phase,
      EventLocation location,
      Map<String, String> data) {
    return publish(target, event, phase, location, data, InstrumentationEventAccess.none());
  }

  synchronized int publish(
      TargetHandle target,
      EventKind event,
      EventPhase phase,
      EventLocation location,
      Map<String, String> data,
      InstrumentationEventAccess access) {
    requireOpen();
    TargetState targetState = requireTarget(target);
    Objects.requireNonNull(event, "event");
    Objects.requireNonNull(phase, "phase");
    Objects.requireNonNull(access, "access");
    if (!event.supports(targetState.descriptor.kind())) {
      throw failure(
          Code.EVENT_TARGET_MISMATCH,
          "EVENT_TARGET_MISMATCH " + event + " " + targetState.descriptor.kind(),
          Map.of("event", event, "targetKind", targetState.descriptor.kind()));
    }
    long sequence = nextSequence(targetState);
    int delivered = 0;
    for (InstrumentState instrumentState : instruments.values()) {
      AttachmentKey key = new AttachmentKey(instrumentState.handle, targetState.handle);
      if (!attachments.containsKey(key)) continue;
      InstrumentRegistration registration = instrumentState.registration;
      if (!registration.events().contains(event)) continue;
      if (!registration.filter().matches(targetState.descriptor)) continue;
      EventLocation projectedLocation =
          registration.projection().sourceLocation() ? location : null;
      EventProjection projection = access.project(registration.projection());
      EventEnvelope envelope =
          new EventEnvelope(
              InstrumentationModel.EVENT_SCHEMA,
              InstrumentationModel.PROTOCOL,
              registration.instrumentId(),
              targetState.descriptor.backend(),
              targetState.descriptor.sessionId(),
              targetState.descriptor.targetId(),
              targetState.descriptor.kind(),
              targetState.handle.generation(),
              sequence,
              phase,
              event,
              projectedLocation,
              data);
      enqueue(instrumentState, envelope, projection);
      delivered++;
    }
    return delivered;
  }

  synchronized EventBatch drain(InstrumentHandle handle) {
    requireOpen();
    InstrumentState state = requireInstrument(handle);
    ArrayList<hara.truffle.InstrumentationModel.DeliveredEvent> events =
        new ArrayList<>(state.queue);
    state.queue.clear();
    EventBatch batch =
        new EventBatch(events, state.droppedSinceDrain, state.droppedTotal);
    state.droppedSinceDrain = 0;
    return batch;
  }

  synchronized ControlLease acquireControlLease(
      InstrumentHandle instrument, TargetHandle target) {
    requireOpen();
    InstrumentState instrumentState = requireInstrument(instrument);
    TargetState targetState = requireTarget(target);
    if (instrumentState.registration.mode() != InstrumentMode.CONTROL) {
      throw failure(
          Code.CONTROL_MODE_REQUIRED,
          "CONTROL_MODE_REQUIRED " + instrument.instrumentId(),
          Map.of("instrument", instrument.instrumentId()));
    }
    AttachmentKey key = new AttachmentKey(instrumentState.handle, targetState.handle);
    if (!attachments.containsKey(key)) {
      throw failure(
          Code.ATTACHMENT_NOT_FOUND,
          "CONTROL_ATTACHMENT_REQUIRED "
              + instrument.instrumentId()
              + " "
              + target.targetId(),
          Map.of(
              "instrument", instrument.instrumentId(), "target", target.targetId()));
    }
    if (targetState.lease != null) {
      ControlLease current = targetState.lease.lease();
      if (current.instrument().equals(instrumentState.handle)) return current;
      throw failure(
          Code.CONTROL_LEASE_CONFLICT,
          "CONTROL_LEASE_CONFLICT " + target.targetId(),
          Map.of(
              "target", target.targetId(),
              "holder", current.instrument().instrumentId(),
              "requester", instrument.instrumentId()));
    }
    ControlLease lease =
        new ControlLease(
            instrumentState.handle,
            targetState.handle,
            targetState.nextLeaseGeneration);
    targetState.lease = new LeaseState(lease);
    return lease;
  }

  synchronized void releaseControlLease(ControlLease lease) {
    requireOpen();
    TargetState targetState = requireTarget(lease.target());
    requireInstrument(lease.instrument());
    requireLease(targetState, lease);
    targetState.lease = null;
    targetState.directives.clear();
    targetState.nextLeaseGeneration = nextGeneration(lease.generation());
  }

  synchronized void issueDirective(
      ControlLease lease, InstrumentDirective directive) {
    requireOpen();
    TargetState targetState = requireTarget(lease.target());
    InstrumentState instrumentState = requireInstrument(lease.instrument());
    requireLease(targetState, lease);
    Objects.requireNonNull(directive, "directive");
    Capability required = requiredCapability(directive);
    if (!instrumentState.registration.capabilities().contains(required)) {
      throw failure(
          Code.UNSUPPORTED_DIRECTIVE,
          "UNSUPPORTED_DIRECTIVE "
              + directive
              + " "
              + lease.instrument().instrumentId(),
          Map.of(
              "instrument", lease.instrument().instrumentId(),
              "target", lease.target().targetId(),
              "directive", directive,
              "required", required));
    }
    targetState.directives.addLast(directive);
  }

  synchronized InstrumentDirective pollDirective(TargetHandle target) {
    requireOpen();
    return requireTarget(target).directives.pollFirst();
  }

  synchronized List<InstrumentRegistration> registrations() {
    requireOpen();
    ArrayList<InstrumentRegistration> result =
        new ArrayList<>(instruments.size());
    for (InstrumentState state : instruments.values()) {
      result.add(state.registration);
    }
    return List.copyOf(result);
  }

  synchronized List<TargetDescriptor> targets() {
    requireOpen();
    ArrayList<TargetDescriptor> result = new ArrayList<>(targets.size());
    for (TargetState state : targets.values()) result.add(state.descriptor);
    return List.copyOf(result);
  }

  synchronized void cleanupSession(String sessionId) {
    requireOpen();
    String required = requiredSessionId(sessionId);
    ArrayList<InstrumentHandle> sessionInstruments = new ArrayList<>();
    for (InstrumentState state : instruments.values()) {
      if (required.equals(state.registration.sessionId())) {
        sessionInstruments.add(state.handle);
      }
    }
    for (InstrumentHandle handle : sessionInstruments) removeInstrument(handle);

    ArrayList<TargetHandle> sessionTargets = new ArrayList<>();
    for (TargetState state : targets.values()) {
      if (required.equals(state.descriptor.sessionId())) {
        sessionTargets.add(state.handle);
      }
    }
    for (TargetHandle handle : sessionTargets) removeTarget(handle);
  }

  synchronized int instrumentCount() {
    requireOpen();
    return instruments.size();
  }

  synchronized int targetCount() {
    requireOpen();
    return targets.size();
  }

  synchronized int attachmentCount() {
    requireOpen();
    return attachments.size();
  }

  synchronized boolean hasSubscribers(TargetHandle target, EventKind event) {
    requireOpen();
    TargetState targetState = requireTarget(target);
    for (InstrumentState instrumentState : instruments.values()) {
      AttachmentKey key = new AttachmentKey(instrumentState.handle, targetState.handle);
      InstrumentRegistration registration = instrumentState.registration;
      if (attachments.containsKey(key)
          && registration.events().contains(event)
          && registration.filter().matches(targetState.descriptor)) {
        return true;
      }
    }
    return false;
  }

  synchronized boolean hasSourceLocationSubscribers(TargetHandle target, EventKind event) {
    requireOpen();
    TargetState targetState = requireTarget(target);
    for (InstrumentState instrumentState : instruments.values()) {
      AttachmentKey key = new AttachmentKey(instrumentState.handle, targetState.handle);
      InstrumentRegistration registration = instrumentState.registration;
      if (attachments.containsKey(key)
          && registration.events().contains(event)
          && registration.projection().sourceLocation()
          && registration.filter().matches(targetState.descriptor)) {
        return true;
      }
    }
    return false;
  }

  synchronized boolean hasAttachments(TargetHandle target) {
    requireOpen();
    TargetState targetState = requireTarget(target);
    return attachments.keySet().stream().anyMatch(key -> key.target().equals(targetState.handle));
  }

  /**
   * Whether generated HBC operations may retain the passive instrumentation path.
   *
   * <p>Control, suspension, call, and exception subscriptions need the resumable machine state.
   * They therefore select the portable machine before a generated root is built. The native tier
   * currently emits instruction and terminal events only.
   */
  synchronized boolean hbcNativeExecutionAllowed(TargetHandle target) {
    requireOpen();
    TargetState targetState = requireTarget(target);
    for (InstrumentState instrumentState : instruments.values()) {
      AttachmentKey key = new AttachmentKey(instrumentState.handle, targetState.handle);
      if (!attachments.containsKey(key)
          || !instrumentState.registration.filter().matches(targetState.descriptor)) {
        continue;
      }
      InstrumentRegistration registration = instrumentState.registration;
      if (registration.mode() != InstrumentMode.PASSIVE) return false;
      for (EventKind event : registration.events()) {
        if (event != EventKind.INSTRUCTION_EXECUTE && event != EventKind.EXECUTION_TERMINAL) {
          return false;
        }
      }
      InstrumentationModel.ProjectionRequest projection = registration.projection();
      if (projection.currentFrame() != null
          || projection.frames() != null
          || projection.locals() != null
          || projection.stack() != null
          || projection.valuePreview() != null
          || projection.machineSnapshot() != null) {
        return false;
      }
    }
    return true;
  }

  synchronized TargetHandle targetFor(String targetId) {
    requireOpen();
    TargetState state = targets.get(targetId);
    if (state == null) {
      throw failure(
          Code.TARGET_NOT_FOUND,
          "TARGET_NOT_FOUND " + targetId,
          Map.of("target", targetId));
    }
    return state.handle;
  }

  synchronized TargetHandle targetIfPresent(String targetId) {
    requireOpen();
    TargetState state = targets.get(targetId);
    return state == null ? null : state.handle;
  }

  synchronized boolean isClosed() {
    return closed;
  }

  @Override
  public synchronized void close() {
    if (closed) return;
    closed = true;
    attachments.clear();
    for (InstrumentState state : instruments.values()) state.queue.clear();
    for (TargetState state : targets.values()) {
      state.directives.clear();
      state.lease = null;
    }
    instruments.clear();
    targets.clear();
    instrumentGenerations.clear();
    targetGenerations.clear();
  }

  private void validateAttachment(
      InstrumentRegistration registration, TargetDescriptor target) {
    if (!registration.sessionId().equals(target.sessionId())) {
      throw failure(
          Code.CROSS_SESSION,
          "INSTRUMENT_TARGET_SESSION_MISMATCH "
              + registration.sessionId()
              + " "
              + target.sessionId(),
          Map.of(
              "instrumentSession", registration.sessionId(),
              "targetSession", target.sessionId()));
    }
    if (!registration.filter().matches(target)) {
      throw failure(
          Code.FILTER_REJECTED,
          "INSTRUMENT_FILTER_REJECTED "
              + registration.instrumentId()
              + " "
              + target.targetId(),
          Map.of(
              "instrument", registration.instrumentId(),
              "target", target.targetId()));
    }
    for (EventKind event : registration.events()) {
      if (!event.supports(target.kind())) {
        throw failure(
            Code.EVENT_TARGET_MISMATCH,
            "EVENT_TARGET_MISMATCH " + event + " " + target.kind(),
            Map.of("event", event, "targetKind", target.kind()));
      }
    }
    TreeSet<Capability> missing = new TreeSet<>(registration.capabilities());
    missing.removeAll(target.capabilities());
    if (!missing.isEmpty()) {
      throw failure(
          Code.UNSUPPORTED_CAPABILITIES,
          "UNSUPPORTED_CAPABILITIES " + target.targetId() + " " + missing,
          capabilityEvidence(registration, target, missing));
    }
  }

  private static Map<String, Object> capabilityEvidence(
      InstrumentRegistration registration,
      TargetDescriptor target,
      Set<Capability> missing) {
    LinkedHashMap<String, Object> evidence = new LinkedHashMap<>();
    evidence.put("target", target.targetId());
    evidence.put("backend", target.backend());
    evidence.put("requested", registration.capabilities());
    evidence.put("potential", target.capabilities());
    evidence.put("missing", Collections.unmodifiableSet(new TreeSet<>(missing)));
    return evidence;
  }

  private void enqueue(
      InstrumentState state,
      EventEnvelope envelope,
      EventProjection projection) {
    int capacity = state.registration.delivery().capacity();
    if (state.queue.size() == capacity) {
      state.queue.removeFirst();
      state.droppedSinceDrain++;
      state.droppedTotal++;
    }
    state.queue.addLast(
        new hara.truffle.InstrumentationModel.DeliveredEvent(
            envelope, projection, state.droppedTotal));
  }

  private InstrumentState requireInstrument(InstrumentHandle handle) {
    requireOpen();
    Objects.requireNonNull(handle, "instrument handle");
    InstrumentState state = instruments.get(handle.instrumentId());
    if (state == null) {
      long next = instrumentGenerations.getOrDefault(handle.instrumentId(), 0L);
      Code code =
          next > handle.generation()
              ? Code.STALE_INSTRUMENT
              : Code.INSTRUMENT_NOT_FOUND;
      throw failure(
          code,
          code + " " + handle.instrumentId() + " " + handle.generation(),
          Map.of(
              "instrument", handle.instrumentId(),
              "generation", handle.generation()));
    }
    if (!state.handle.equals(handle)) {
      throw failure(
          Code.STALE_INSTRUMENT,
          "STALE_INSTRUMENT "
              + handle.instrumentId()
              + " "
              + handle.generation(),
          Map.of(
              "instrument", handle.instrumentId(),
              "generation", handle.generation(),
              "currentGeneration", state.handle.generation()));
    }
    return state;
  }

  private TargetState requireTarget(TargetHandle handle) {
    requireOpen();
    Objects.requireNonNull(handle, "target handle");
    TargetState state = targets.get(handle.targetId());
    if (state == null) {
      long next = targetGenerations.getOrDefault(handle.targetId(), 0L);
      Code code =
          next > handle.generation() ? Code.STALE_TARGET : Code.TARGET_NOT_FOUND;
      throw failure(
          code,
          code + " " + handle.targetId() + " " + handle.generation(),
          Map.of("target", handle.targetId(), "generation", handle.generation()));
    }
    if (!state.handle.equals(handle)) {
      throw failure(
          Code.STALE_TARGET,
          "STALE_TARGET " + handle.targetId() + " " + handle.generation(),
          Map.of(
              "target", handle.targetId(),
              "generation", handle.generation(),
              "currentGeneration", state.handle.generation()));
    }
    return state;
  }

  private static void requireLease(TargetState targetState, ControlLease lease) {
    Objects.requireNonNull(lease, "lease");
    if (targetState.lease == null || !targetState.lease.lease().equals(lease)) {
      throw failure(
          Code.INVALID_CONTROL_LEASE,
          "INVALID_CONTROL_LEASE "
              + lease.target().targetId()
              + " "
              + lease.generation(),
          Map.of(
              "target", lease.target().targetId(),
              "instrument", lease.instrument().instrumentId(),
              "generation", lease.generation()));
    }
  }

  private void removeAttachmentsForInstrument(InstrumentHandle handle) {
    Iterator<AttachmentKey> iterator = attachments.keySet().iterator();
    while (iterator.hasNext()) {
      if (iterator.next().instrument().equals(handle)) iterator.remove();
    }
  }

  private void removeAttachmentsForTarget(TargetHandle handle) {
    Iterator<AttachmentKey> iterator = attachments.keySet().iterator();
    while (iterator.hasNext()) {
      if (iterator.next().target().equals(handle)) iterator.remove();
    }
  }

  private void releaseLeasesForInstrument(InstrumentHandle handle) {
    for (TargetState target : targets.values()) {
      if (target.lease != null
          && target.lease.lease().instrument().equals(handle)) {
        target.lease = null;
        target.directives.clear();
        target.nextLeaseGeneration =
            nextGeneration(target.nextLeaseGeneration);
      }
    }
  }

  private static Capability requiredCapability(InstrumentDirective directive) {
    return switch (directive) {
      case CONTINUE -> Capability.CONTROL_RESUME;
      case SUSPEND -> Capability.CONTROL_PAUSE;
      case STEP_NEXT -> Capability.CONTROL_SINGLE_STEP;
      case SETTLE -> Capability.CONTROL_SETTLE;
      case TERMINATE -> Capability.CONTROL_TERMINATE;
    };
  }

  private static long nextSequence(TargetState state) {
    if (state.sequence == Long.MAX_VALUE) {
      throw new IllegalStateException(
          "INSTRUMENTATION_SEQUENCE_EXHAUSTED " + state.handle.targetId());
    }
    state.sequence++;
    return state.sequence;
  }

  private static long nextGeneration(long generation) {
    if (generation == Long.MAX_VALUE) {
      throw new IllegalStateException("INSTRUMENTATION_GENERATION_EXHAUSTED");
    }
    return generation + 1;
  }

  private static String requiredSessionId(String sessionId) {
    if (sessionId == null || sessionId.isBlank()) {
      throw new IllegalArgumentException("EMPTY_INSTRUMENTATION_SESSION");
    }
    return sessionId;
  }

  private void requireOpen() {
    if (closed) {
      throw failure(Code.RUNTIME_CLOSED, "INSTRUMENTATION_RUNTIME_CLOSED");
    }
  }

  private static InstrumentationException failure(Code code, String message) {
    return new InstrumentationException(code, message);
  }

  private static InstrumentationException failure(
      Code code, String message, Map<String, ?> evidence) {
    return new InstrumentationException(code, message, evidence);
  }
}
