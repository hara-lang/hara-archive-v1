package hara.truffle;

import hara.lang.base.NumUtils;
import hara.truffle.InstrumentationModel.Capability;
import hara.truffle.InstrumentationModel.DeliveredEvent;
import hara.truffle.InstrumentationModel.EventBatch;
import hara.truffle.InstrumentationModel.EventDelivery;
import hara.truffle.InstrumentationModel.EventKind;
import hara.truffle.InstrumentationModel.EventLocation;
import hara.truffle.InstrumentationModel.EventPhase;
import hara.truffle.InstrumentationModel.EventProjection;
import hara.truffle.InstrumentationModel.InstrumentFilter;
import hara.truffle.InstrumentationModel.InstrumentHandle;
import hara.truffle.InstrumentationModel.InstrumentMode;
import hara.truffle.InstrumentationModel.InstrumentRegistration;
import hara.truffle.InstrumentationModel.ProjectionLimits;
import hara.truffle.InstrumentationModel.ProjectionRequest;
import hara.truffle.InstrumentationModel.RuntimeBackend;
import hara.truffle.InstrumentationModel.TargetDescriptor;
import hara.truffle.InstrumentationModel.TargetHandle;
import hara.truffle.InstrumentationModel.TargetKind;
import hara.truffle.NativeInstrumentation.NativeInstrumentHandle;
import hara.truffle.NativeInstrumentation.NativeTargetHandle;
import hara.truffle.bytecode.HbcProgram;
import hara.truffle.bytecode.HbcProgram.Function;
import hara.truffle.bytecode.HbcProgram.Instruction;
import hara.truffle.bytecode.HbcProgram.Opcode;
import java.io.PrintStream;
import java.math.BigInteger;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Collections;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.TreeSet;
import org.graalvm.polyglot.Value;

/** Produces the shared production-backed instrumentation conformance report. */
public final class InstrumentationConformance {
  private static final String CORPUS_SCHEMA = "hara.instrumentation.conformance-corpus/1";
  private static final String REPORT_SCHEMA = "hara.instrumentation.conformance-report/1";
  private static final String SESSION_ID = "instrum-freeze";

  private InstrumentationConformance() {}

  public static void main(String[] args) {
    int status = run(args, System.out, System.err);
    if (status != 0) System.exit(status);
  }

  static int run(String[] args, PrintStream output, PrintStream error) {
    try {
      Path corpusPath = corpusPath(args);
      JsonValue.Object corpus = asObject(StrictJson.parseValue(Files.readString(corpusPath)));
      if (!CORPUS_SCHEMA.equals(requiredString(corpus, "schema"))) {
        throw new IllegalArgumentException("unsupported instrumentation corpus schema");
      }
      TreeSet<String> ids = new TreeSet<>();
      List<Object> cases = new ArrayList<>();
      for (JsonValue value : array(corpus, "cases")) {
        JsonValue.Object testCase = asObject(value);
        String id = requiredString(testCase, "id");
        if (!ids.add(id)) throw new IllegalArgumentException("duplicate instrumentation corpus case " + id);
        cases.add(observe(testCase));
      }
      Map<String, Object> report =
          object(
              "schema",
              REPORT_SCHEMA,
              "corpus",
              object("schema", requiredString(corpus, "schema"), "id", requiredString(corpus, "id")),
              "runtime",
              "java",
              "cases",
              cases);
      String encoded = CodeVmConformanceDocument.Json.write(report, true);
      String reportPath = System.getProperty("hara.instrumentationReport");
      if (reportPath == null || reportPath.isBlank()) {
        output.println(encoded);
      } else {
        Path target = Path.of(reportPath);
        Path parent = target.toAbsolutePath().getParent();
        if (parent != null) Files.createDirectories(parent);
        Files.writeString(target, encoded + System.lineSeparator(), StandardCharsets.UTF_8);
      }
      return 0;
    } catch (Exception failure) {
      error.println("Java instrumentation conformance failed: " + message(failure));
      return 1;
    }
  }

  private static Path corpusPath(String[] args) {
    if (args.length == 0) {
      String configured = System.getProperty("hara.instrumentationCorpus");
      if (configured == null || configured.isBlank()) {
        throw new IllegalArgumentException("missing --corpus PATH or hara.instrumentationCorpus");
      }
      return Path.of(configured);
    }
    if (args.length == 2 && "--corpus".equals(args[0])) return Path.of(args[1]);
    throw new IllegalArgumentException("usage: InstrumentationConformance --corpus PATH");
  }

  private static Map<String, Object> observe(JsonValue.Object testCase) {
    String id = requiredString(testCase, "id");
    return switch (requiredString(testCase, "kind")) {
      case "execution" -> observeExecution(testCase, id);
      case "hub" -> observeHub(testCase, id);
      case "live-session" -> observeLiveSession(testCase, id);
      case "code-vm" -> observeCodeVm(testCase, id);
      default -> throw new IllegalArgumentException(id + ": unsupported conformance case kind");
    };
  }

  private static Map<String, Object> observeExecution(JsonValue.Object testCase, String id) {
    TargetKind targetKind = targetKind(requiredString(testCase, "targetKind"));
    if (targetKind == TargetKind.WHOLE_WASM) {
      throw new IllegalArgumentException(id + ": whole-wasm execution uses the artifact lane");
    }
    TreeSet<EventKind> events = eventSet(testCase, id);
    ProjectionRequest projection = projection(testCase, id);
    TreeSet<Capability> capabilities = capabilities(events, projection);
    String sourceId = requiredString(testCase, "sourceId");
    String source = requiredString(testCase, "source");
    SessionModel.SessionId sessionId = SessionModel.SessionId.parse(SESSION_ID);

    try (SessionKernel kernel = new SessionKernel(false, false)) {
      SessionKernel.Session session = kernel.create(sessionId);
      NativeInstrumentation service = kernel.instrumentation(sessionId);
      NativeTargetHandle target =
          service.bindTargetIdentity(sessionId.value() + "/" + targetKind, 0);
      NativeInstrumentHandle instrument =
          service.register(
              new InstrumentRegistration(
                  id + "/instrument",
                  sessionId.value(),
                  InstrumentMode.PASSIVE,
                  capabilities,
                  events,
                  new InstrumentFilter(sessionId.value(), Set.of(), Set.of(), Set.of()),
                  projection,
                  EventDelivery.queue(queueCapacity(testCase, id))));
      service.attach(instrument, target);

      String status = "returned";
      Object result = null;
      try {
        if (targetKind == TargetKind.INTERPRETER) {
          result = session.eval(source, sourceId, 1, 1);
        } else {
          result = session.executeHbc(program(value(testCase, "program"), id));
        }
      } catch (RuntimeException failure) {
        status = "failed";
      }
      EventBatch batch = service.drainEvents(instrument);
      Map<String, Object> summary =
          eventSummary(batch.events(), status, result, targetKind == TargetKind.INTERPRETER);
      validateExecutionExpectations(testCase, id, summary);
      return object(
          "id", id,
          "kind", "execution",
          "targetKind", targetKind.toString(),
          "observation", summary);
    }
  }

  private static Map<String, Object> observeHub(JsonValue.Object testCase, String id) {
    return switch (requiredString(testCase, "operation")) {
      case "registration-filter-order" -> observeRegistrationFilterOrder(id);
      case "queue-generation" -> observeQueueGeneration(id);
      case "control-lease" -> observeControlLease(id);
      case "unsupported-capability" -> observeUnsupportedCapability(id);
      case "zero-instrument" -> observeZeroInstrument(id);
      case "session-cleanup" -> observeSessionCleanup(id);
      default -> throw new IllegalArgumentException(id + ": unsupported hub operation");
    };
  }

  private static Map<String, Object> observeRegistrationFilterOrder(String id) {
    try (InstrumentationHub hub = new InstrumentationHub()) {
      TargetHandle hbc = registerPortableTarget(hub, "hub/hbc", TargetKind.HBC);
      TargetHandle interpreter = registerPortableTarget(hub, "hub/interpreter", TargetKind.INTERPRETER);
      InstrumentHandle first = hub.registerInstrument(portablePassive("first", null, 8));
      InstrumentHandle second = hub.registerInstrument(portablePassive("second", TargetKind.HBC, 8));
      hub.attach(first, hbc);
      hub.attach(second, hbc);
      hub.attach(first, interpreter);
      boolean filterRejected;
      try {
        hub.attach(second, interpreter);
        filterRejected = false;
      } catch (InstrumentationException rejected) {
        filterRejected = rejected.code() == InstrumentationException.Code.FILTER_REJECTED;
      }
      List<String> order =
          hub.registrations().stream().map(InstrumentRegistration::instrumentId).toList();
      hub.publish(
          hbc,
          EventKind.EXECUTION_TERMINAL,
          EventPhase.LIVE,
          null,
          Map.of("status", "returned"));
      int firstCount = hub.drain(first).events().size();
      int secondCount = hub.drain(second).events().size();
      return object(
          "id",
          id,
          "kind",
          "hub",
          "operation",
          "registration-filter-order",
          "attachmentOrder",
          order,
          "filterRejected",
          filterRejected,
          "delivered",
          object("first", firstCount, "second", secondCount));
    }
  }

  private static Map<String, Object> observeQueueGeneration(String id) {
    try (InstrumentationHub hub = new InstrumentationHub()) {
      TargetHandle target = registerPortableTarget(hub, "hub/queue-target", TargetKind.HBC);
      InstrumentHandle instrument = hub.registerInstrument(portablePassive("queue", null, 1));
      hub.attach(instrument, target);
      for (String status : List.of("first", "second")) {
        hub.publish(
            target,
            EventKind.EXECUTION_TERMINAL,
            EventPhase.LIVE,
            null,
            Map.of("status", status));
      }
      EventBatch batch = hub.drain(instrument);
      DeliveredEvent retained = batch.events().get(0);
      Map<String, Object> envelope = canonicalEnvelope(retained.envelope());
      long dropped = batch.droppedSinceDrain();
      hub.removeInstrument(instrument);
      InstrumentHandle replacement = hub.registerInstrument(portablePassive("queue", null, 1));
      hub.removeTarget(target);
      TargetHandle replacementTarget = registerPortableTarget(hub, "hub/queue-target", TargetKind.HBC);
      return object(
          "id",
          id,
          "kind",
          "hub",
          "operation",
          "queue-generation",
          "dropped",
          dropped,
          "retained",
          envelope,
          "instrumentGeneration",
          replacement.generation(),
          "targetGeneration",
          replacementTarget.generation());
    }
  }

  private static Map<String, Object> observeControlLease(String id) {
    try (InstrumentationHub hub = new InstrumentationHub()) {
      TargetHandle target =
          hub.registerTarget(
              new TargetDescriptor(
                  "hub/lease-target",
                  SESSION_ID,
                  TargetKind.HBC,
                  new RuntimeBackend("portable"),
                  Set.of(Capability.EVENT_LIFECYCLE, Capability.CONTROL_PAUSE)));
      InstrumentHandle first = hub.registerInstrument(portableControl("lease-first"));
      InstrumentHandle second = hub.registerInstrument(portableControl("lease-second"));
      hub.attach(first, target);
      hub.attach(second, target);
      hub.acquireControlLease(first, target);
      InstrumentationException conflict;
      try {
        hub.acquireControlLease(second, target);
        throw new IllegalStateException(id + ": second control lease unexpectedly succeeded");
      } catch (InstrumentationException failure) {
        conflict = failure;
      }
      return object(
          "id",
          id,
          "kind",
          "hub",
          "operation",
          "control-lease",
          "error",
          object(
              "code", "control-lease-conflict", "holder", conflict.evidence().get("holder")));
    }
  }

  private static Map<String, Object> observeUnsupportedCapability(String id) {
    try (InstrumentationHub hub = new InstrumentationHub()) {
      TargetHandle target =
          hub.registerTarget(
              new TargetDescriptor(
                  "hub/unsupported-target",
                  SESSION_ID,
                  TargetKind.INTERPRETER,
                  new RuntimeBackend("portable"),
                  Set.of()));
      InstrumentHandle instrument = hub.registerInstrument(portablePassive("unsupported", null, 8));
      InstrumentationException failure;
      try {
        hub.attach(instrument, target);
        throw new IllegalStateException(id + ": unsupported attachment unexpectedly succeeded");
      } catch (InstrumentationException error) {
        failure = error;
      }
      return object(
          "id",
          id,
          "kind",
          "hub",
          "operation",
          "unsupported-capability",
          "error",
          object(
              "code",
              "unsupported-capabilities",
              "target",
              failure.evidence().get("target"),
              "backend",
              String.valueOf(failure.evidence().get("backend")),
              "requested",
              capabilityNames(failure.evidence().get("requested")),
              "potential",
              capabilityNames(failure.evidence().get("potential")),
              "missing",
              capabilityNames(failure.evidence().get("missing"))));
    }
  }

  private static Map<String, Object> observeZeroInstrument(String id) {
    try (InstrumentationHub hub = new InstrumentationHub()) {
      TargetHandle target = registerPortableTarget(hub, "hub/zero-target", TargetKind.INTERPRETER);
      hub.registerInstrument(portablePassive("zero", null, 8));
      return object(
          "id",
          id,
          "kind",
          "hub",
          "operation",
          "zero-instrument",
          "enabled",
          hub.hasSubscribers(target, EventKind.EXECUTION_TERMINAL),
          "instrumentCount",
          hub.instrumentCount(),
          "targetCount",
          hub.targetCount(),
          "attachmentCount",
          hub.attachmentCount());
    }
  }

  private static Map<String, Object> observeSessionCleanup(String id) {
    try (InstrumentationHub hub = new InstrumentationHub()) {
      TargetHandle target = registerPortableTarget(hub, "hub/cleanup-target", TargetKind.HBC);
      InstrumentHandle instrument = hub.registerInstrument(portablePassive("cleanup", null, 8));
      hub.attach(instrument, target);
      int beforeInstruments = hub.instrumentCount();
      int beforeTargets = hub.targetCount();
      hub.cleanupSession(SESSION_ID);
      return object(
          "id",
          id,
          "kind",
          "hub",
          "operation",
          "session-cleanup",
          "removed",
          object("instruments", beforeInstruments, "targets", beforeTargets),
          "remaining",
          object(
              "instruments",
              hub.instrumentCount(),
              "targets",
              hub.targetCount(),
              "attachments",
              hub.attachmentCount(),
              "eventsEnabled",
              false));
    }
  }

  private static Map<String, Object> observeLiveSession(JsonValue.Object testCase, String id) {
    String sourceId = requiredString(testCase, "sourceId");
    String source = requiredString(testCase, "source");
    SessionModel.SessionId sessionId = SessionModel.SessionId.parse(SESSION_ID);
    boolean instrumented;
    long resetGeneration;
    try (SessionKernel kernel = new SessionKernel(false, false)) {
      SessionKernel.Session session = kernel.create(sessionId);
      NativeInstrumentation service = kernel.instrumentation(sessionId);
      NativeTargetHandle target =
          service.bindTargetIdentity(sessionId.value() + "/interpreter", 0);
      NativeInstrumentHandle instrument =
          service.register(
              new InstrumentRegistration(
                  "live/instrument",
                  sessionId.value(),
                  InstrumentMode.PASSIVE,
                  Set.of(Capability.EVENT_SEMANTIC_BOUNDARY, Capability.EVENT_LIFECYCLE),
                  Set.of(EventKind.SEMANTIC_BOUNDARY, EventKind.EXECUTION_TERMINAL),
                  new InstrumentFilter(sessionId.value(), Set.of(), Set.of(), Set.of()),
                  ProjectionRequest.none(),
                  EventDelivery.queue(128)));
      service.attach(instrument, target);
      try {
        session.eval(source, sourceId, 1, 1);
      } catch (RuntimeException failure) {
        throw new IllegalArgumentException(id + ": live run failed", failure);
      }
      instrumented = !service.drainEvents(instrument).events().isEmpty();
      kernel.closeSession(sessionId);
      kernel.create(sessionId);
      NativeInstrumentation replacement = kernel.instrumentation(sessionId);
      resetGeneration =
          replacement
              .bindTargetIdentity(sessionId.value() + "/interpreter", 1)
              .generation();
      kernel.closeSession(sessionId);
    }

    Map<String, Object> initial = stateSummary(sourceId, 0, "ready");
    Map<String, Object> reset = stateSummary(sourceId, resetGeneration, "running");
    Map<String, Object> dispose = stateSummary(sourceId, resetGeneration, "disposed");
    JsonValue.Object expected = optionalObject(testCase, "expect");
    if (expected != null) {
      expectString(expected, "runStatus", "returned", id);
      expectLong(expected, "resetGeneration", resetGeneration, id);
      expectString(expected, "disposeStatus", "disposed", id);
    }
    return object(
        "id",
        id,
        "kind",
        "live-session",
        "backend",
        "interpreter",
        "initial",
        initial,
        "run",
        object(
            "status",
            "returned",
            "generation",
            0L,
            "advanced",
            true,
            "instrumented",
            instrumented),
        "reset",
        reset,
        "dispose",
        dispose);
  }

  private static Map<String, Object> observeCodeVm(JsonValue.Object testCase, String id) {
    SessionModel.SessionId sessionId = SessionModel.SessionId.parse("code-vm");
    Object result;
    String status = "returned";
    try (SessionKernel kernel = new SessionKernel(false, false)) {
      SessionKernel.Session session = kernel.create(sessionId);
      try {
        result = session.executeHbc(program(value(testCase, "program"), id));
      } catch (RuntimeException failure) {
        status = "failed";
        result = null;
      }
    }
    String resultType = result == null ? null : portableType(result);
    String resultDisplay = result == null ? null : portableDisplay(result);
    JsonValue.Object expected = optionalObject(testCase, "expect");
    if (expected != null) {
      expectString(expected, "status", status, id);
      if (expectedValue(expected, "resultType") != null) {
        expectString(expected, "resultType", resultType, id);
      }
      if (expectedValue(expected, "result") != null) {
        expectString(expected, "result", resultDisplay, id);
      }
    }
    return object(
        "id", id, "kind", "code-vm", "status", status, "resultType", resultType, "result", resultDisplay);
  }

  private static Map<String, Object> eventSummary(
      List<DeliveredEvent> events, String status, Object result, boolean interpreter) {
    TreeSet<String> eventSet = new TreeSet<>();
    List<String> eventOrder = new ArrayList<>();
    TreeSet<String> phases = new TreeSet<>();
    TreeSet<Long> generations = new TreeSet<>();
    TreeSet<String> projections = new TreeSet<>();
    TreeSet<String> terminalStatuses = new TreeSet<>();
    List<Object> portableEvents = new ArrayList<>();
    Long firstSequence = null;
    Long previousSequence = null;
    boolean strictSequence = true;
    boolean locationsPresent = true;
    for (DeliveredEvent delivered : events) {
      var envelope = delivered.envelope();
      String name = eventName(envelope.event());
      if (eventSet.add(name)) eventOrder.add(name);
      phases.add(phaseName(envelope.phase()));
      generations.add(envelope.generation());
      if (firstSequence == null) firstSequence = envelope.sequence();
      if (previousSequence != null && envelope.sequence() != previousSequence + 1) {
        strictSequence = false;
      }
      previousSequence = envelope.sequence();
      locationsPresent &= envelope.location() != null;
      EventProjection projection = delivered.projection();
      List<String> eventProjections = new ArrayList<>();
      if (projection.currentFrame() != null) {
        projections.add("current-frame");
        eventProjections.add("current-frame");
      }
      if (projection.frames() != null) {
        projections.add("frames");
        eventProjections.add("frames");
      }
      if (projection.locals() != null) {
        projections.add("locals");
        eventProjections.add("locals");
      }
      if (projection.stack() != null) {
        projections.add("stack");
        eventProjections.add("stack");
      }
      if (projection.valuePreview() != null) {
        projections.add("value-preview");
        eventProjections.add("value-preview");
      }
      if (projection.machineSnapshot() != null) {
        projections.add("machine-snapshot");
        eventProjections.add("machine-snapshot");
      }
      if (envelope.event() == EventKind.EXECUTION_TERMINAL) {
        String terminal = envelope.data().get("status");
        if (terminal != null) terminalStatuses.add(normalizeStatus(terminal));
      }
      portableEvents.add(
          object(
              "event",
              name,
              "phase",
              phaseName(envelope.phase()),
              "location",
              envelope.location() != null,
              "projections",
              eventProjections));
    }
    return object(
        "status",
        status,
        "resultType",
        result == null || "failed".equals(status) ? null : portableType(result),
        "result",
        result == null || "failed".equals(status) ? null : portableDisplay(result),
        "events",
        portableEvents,
        "eventSet",
        new ArrayList<>(eventSet),
        "eventOrder",
        eventOrder,
        "phases",
        new ArrayList<>(phases),
        "sequence",
        object("first", firstSequence, "strict", strictSequence, "generations", new ArrayList<>(generations)),
        "locations",
        object("present", !events.isEmpty() && locationsPresent, "any", events.stream().anyMatch(event -> event.location() != null)),
        "projections",
        new ArrayList<>(projections),
        "terminal",
        object(
            "count",
            events.stream().filter(event -> event.event() == EventKind.EXECUTION_TERMINAL).count(),
            "statuses",
            new ArrayList<>(terminalStatuses)));
  }

  private static void validateExecutionExpectations(
      JsonValue.Object testCase, String id, Map<String, Object> summary) {
    JsonValue.Object expected = optionalObject(testCase, "expect");
    if (expected == null) throw new IllegalArgumentException(id + ": execution case requires expect");
    @SuppressWarnings("unchecked")
    List<String> actualEvents = (List<String>) summary.get("eventSet");
    JsonValue required = expectedValue(expected, "requiredEvents");
    if (required instanceof JsonValue.Array requiredEvents) {
      for (JsonValue event : requiredEvents.values()) {
        String name = stringValue(event, "required event");
        if (!actualEvents.contains(name)) {
          throw new IllegalStateException(id + ": required event " + name + " was not produced");
        }
      }
    }
    expectString(expected, "terminalStatus", String.valueOf(summary.get("status")), id);
    if (expectedValue(expected, "resultType") != null) {
      expectString(expected, "resultType", (String) summary.get("resultType"), id);
    }
    long eventCount = ((List<?>) summary.get("events")).size();
    if (expectedValue(expected, "minimumEvents") != null
        && eventCount < number(expectedValue(expected, "minimumEvents"))) {
      throw new IllegalStateException(id + ": expected at least " + number(expectedValue(expected, "minimumEvents")) + " events, got " + eventCount);
    }
    if (bool(expected, "locationAll") && !Boolean.TRUE.equals(((Map<?, ?>) summary.get("locations")).get("present"))) {
      throw new IllegalStateException(id + ": requested locations were not present on every event");
    }
    if (bool(expected, "sequenceStrict") && !Boolean.TRUE.equals(((Map<?, ?>) summary.get("sequence")).get("strict"))) {
      throw new IllegalStateException(id + ": event sequence is not strictly increasing");
    }
    JsonValue.Object projectionExpectations = optionalObject(expected, "projections");
    if (projectionExpectations != null) {
      @SuppressWarnings("unchecked")
      List<String> actual = (List<String>) summary.get("projections");
      for (Map.Entry<String, JsonValue> entry : projectionExpectations.values().entrySet()) {
        if (entry.getValue() instanceof JsonValue.Bool value && value.value() && !actual.contains(entry.getKey())) {
          throw new IllegalStateException(id + ": expected projection " + entry.getKey() + " was not delivered");
        }
      }
    }
  }

  private static TreeSet<EventKind> eventSet(JsonValue.Object testCase, String id) {
    TreeSet<EventKind> result = new TreeSet<>();
    for (JsonValue value : array(testCase, "capture")) result.add(eventKind(stringValue(value, id + " capture event")));
    return result;
  }

  private static TreeSet<Capability> capabilities(
      Set<EventKind> events, ProjectionRequest projection) {
    TreeSet<Capability> result = new TreeSet<>();
    for (EventKind event : events) result.add(event.requiredCapability());
    result.addAll(projection.requiredCapabilities());
    return result;
  }

  private static ProjectionRequest projection(JsonValue.Object testCase, String id) {
    JsonValue.Object value = optionalObject(testCase, "projection");
    if (value == null) return ProjectionRequest.none();
    ProjectionLimits limits = ProjectionLimits.defaults();
    return new ProjectionRequest(
        bool(value, "sourceLocation"),
        flag(value, "currentFrame", limits),
        flag(value, "frames", limits),
        flag(value, "locals", limits),
        flag(value, "stack", limits),
        flag(value, "valuePreview", limits),
        flag(value, "machineSnapshot", limits));
  }

  private static ProjectionLimits flag(JsonValue.Object value, String key, ProjectionLimits limits) {
    return bool(value, key) ? limits : null;
  }

  private static int queueCapacity(JsonValue.Object testCase, String id) {
    JsonValue value = expectedValue(testCase, "queueCapacity");
    long capacity = value == null ? 256 : number(value);
    if (capacity <= 0 || capacity > Integer.MAX_VALUE) throw new IllegalArgumentException(id + ": invalid queueCapacity");
    return (int) capacity;
  }

  private static TargetHandle registerPortableTarget(
      InstrumentationHub hub, String targetId, TargetKind kind) {
    return hub.registerTarget(
        new TargetDescriptor(
            targetId,
            SESSION_ID,
            kind,
            new RuntimeBackend("portable"),
            Set.of(Capability.EVENT_LIFECYCLE)));
  }

  private static InstrumentRegistration portablePassive(
      String id, TargetKind targetKind, int capacity) {
    return new InstrumentRegistration(
        id,
        SESSION_ID,
        InstrumentMode.PASSIVE,
        Set.of(Capability.EVENT_LIFECYCLE),
        Set.of(EventKind.EXECUTION_TERMINAL),
        new InstrumentFilter(null, Set.of(), targetKind == null ? Set.of() : Set.of(targetKind), Set.of()),
        ProjectionRequest.none(),
        EventDelivery.queue(capacity));
  }

  private static InstrumentRegistration portableControl(String id) {
    return new InstrumentRegistration(
        id,
        SESSION_ID,
        InstrumentMode.CONTROL,
        Set.of(Capability.EVENT_LIFECYCLE, Capability.CONTROL_PAUSE),
        Set.of(EventKind.EXECUTION_TERMINAL),
        InstrumentFilter.all(),
        ProjectionRequest.none(),
        EventDelivery.queue(8));
  }

  private static Map<String, Object> canonicalEnvelope(
      InstrumentationModel.EventEnvelope envelope) {
    return object(
        "schema",
        envelope.schema(),
        "protocol",
        envelope.protocol(),
        "instrumentId",
        envelope.instrumentId(),
        "runtime",
        envelope.runtime().toString(),
        "sessionId",
        envelope.sessionId(),
        "targetId",
        envelope.targetId(),
        "targetKind",
        envelope.targetKind().toString(),
        "generation",
        envelope.generation(),
        "sequence",
        envelope.sequence(),
        "phase",
        phaseName(envelope.phase()),
        "event",
        eventName(envelope.event()),
        "location",
        locationValue(envelope.location()),
        "data",
        envelope.data());
  }

  private static Map<String, Object> stateSummary(String sourceId, long generation, String status) {
    return object("sourceId", sourceId, "generation", generation, "status", status, "backend", "interpreter");
  }

  private static HbcProgram program(JsonValue value, String id) {
    JsonValue.Object object = asObject(value);
    List<Object> constants = new ArrayList<>();
    for (JsonValue constant : array(object, "constants")) constants.add(toJava(constant));
    List<Function> functions = new ArrayList<>();
    for (JsonValue function : array(object, "functions")) functions.add(function(asObject(function), id));
    int entry = Math.toIntExact(number(value(object, "entry")));
    return new HbcProgram(
        optionalString(object, "namespace"),
        constants,
        List.of(),
        Map.of(),
        Map.of(),
        Map.of(),
        functions,
        entry);
  }

  private static Function function(JsonValue.Object object, String id) {
    List<Instruction> code = new ArrayList<>();
    for (JsonValue instruction : array(object, "code")) code.add(instruction(asObject(instruction), id));
    List<HbcProgram.Position> sourceMap = new ArrayList<>();
    JsonValue sourceMapValue = expectedValue(object, "sourceMap");
    if (sourceMapValue == null) {
      sourceMap.addAll(Collections.nCopies(code.size(), null));
    } else {
      for (JsonValue value : asArray(sourceMapValue).values()) {
        sourceMap.add(position(asObject(value)));
      }
    }
    return new Function(
        optionalString(object, "name"),
        bool(object, "asyncFunction"),
        Math.toIntExact(optionalNumber(object, "arity", 0)),
        bool(object, "variadic"),
        Math.toIntExact(optionalNumber(object, "captureCount", 0)),
        Math.toIntExact(optionalNumber(object, "localCount", 0)),
        Math.toIntExact(optionalNumber(object, "maxStack", 4)),
        code,
        sourceMap,
        List.of());
  }

  private static HbcProgram.Position position(JsonValue.Object object) {
    return new HbcProgram.Position(
        number(value(object, "offset")), number(value(object, "line")), number(value(object, "column")));
  }

  private static Instruction instruction(JsonValue.Object object, String id) {
    String opcode = requiredString(object, "opcode");
    long first = optionalNumber(object, "first", 0);
    long second = optionalNumber(object, "second", 0);
    long third = optionalNumber(object, "third", 0);
    return new Instruction(
        switch (opcode) {
          case "CONSTANT" -> Opcode.CONSTANT;
          case "NIL" -> Opcode.NIL;
          case "TRUE" -> Opcode.TRUE;
          case "FALSE" -> Opcode.FALSE;
          case "PRIMITIVE" -> Opcode.PRIMITIVE;
          case "RETURN" -> Opcode.RETURN;
          default -> throw new IllegalArgumentException(id + ": unsupported program opcode " + opcode);
        },
        first,
        second,
        third);
  }

  private static List<String> capabilityNames(Object value) {
    if (!(value instanceof Iterable<?> values)) return List.of();
    List<String> result = new ArrayList<>();
    for (Object item : values) {
      if (item instanceof Capability capability) result.add(capabilityName(capability));
    }
    Collections.sort(result);
    return result;
  }

  private static String capabilityName(Capability capability) {
    return capability.name().toLowerCase(java.util.Locale.ROOT).replace('_', '-');
  }

  private static String portableType(Object value) {
    if (value == null) return "nil";
    if (value instanceof Value guest) {
      if (guest.isNull()) return "nil";
      if (guest.isBoolean()) return "boolean";
      if (guest.isNumber()) {
        if (guest.fitsInLong()) return "long";
        if (guest.fitsInBigInteger()) return "bigint";
      }
      if (guest.isString()) return "string";
      value = guest;
    }
    Object raw = HaraBox.unwrap(value);
    if (raw instanceof Boolean) return "boolean";
    if (NumUtils.isLongValue(raw)) return "long";
    if (NumUtils.isBigIntegerValue(raw)) return "bigint";
    if (raw instanceof Float || raw instanceof Double) return "float";
    if (raw instanceof String) return "string";
    return "host-handle";
  }

  private static String portableDisplay(Object value) {
    if (value instanceof Value guest) return guest.toString();
    Object raw = HaraBox.unwrap(value);
    return String.valueOf(raw);
  }

  private static String normalizeStatus(String status) {
    return switch (status) {
      case "return" -> "returned";
      case "failure" -> "failed";
      default -> status;
    };
  }

  private static Map<String, Object> locationValue(EventLocation location) {
    if (location == null) return null;
    Map<String, Object> value = new LinkedHashMap<>();
    if (location.sourceId() != null) value.put("sourceId", location.sourceId());
    if (!location.formPath().isEmpty()) value.put("formPath", location.formPath());
    if (location.span() != null) value.put("span", List.of(location.span().start(), location.span().end()));
    if (location.function() != null) value.put("function", location.function());
    if (location.instructionPointer() != null) value.put("instructionPointer", location.instructionPointer());
    return value;
  }

  private static String eventName(EventKind event) {
    return switch (event) {
      case SEMANTIC_BOUNDARY -> "semantic-boundary";
      case INSTRUCTION_EXECUTE -> "instruction-execute";
      case CALL_ENTER -> "call-enter";
      case CALL_RETURN -> "call-return";
      case EXCEPTION_RAISE -> "exception-raise";
      case EXCEPTION_UNWIND -> "exception-unwind";
      case VAR_SET -> "var-set";
      case FIELD_SET -> "field-set";
      case PROMISE_SUSPEND -> "promise-suspend";
      case PROMISE_RESUME -> "promise-resume";
      case MACHINE_SUSPEND -> "machine-suspend";
      case MACHINE_RESUME -> "machine-resume";
      case PROTOCOL_CALL -> "protocol-call";
      case EXECUTION_TERMINAL -> "execution-terminal";
    };
  }

  private static EventKind eventKind(String value) {
    return switch (value) {
      case "semantic-boundary" -> EventKind.SEMANTIC_BOUNDARY;
      case "instruction-execute" -> EventKind.INSTRUCTION_EXECUTE;
      case "call-enter" -> EventKind.CALL_ENTER;
      case "call-return" -> EventKind.CALL_RETURN;
      case "exception-raise" -> EventKind.EXCEPTION_RAISE;
      case "exception-unwind" -> EventKind.EXCEPTION_UNWIND;
      case "var-set" -> EventKind.VAR_SET;
      case "field-set" -> EventKind.FIELD_SET;
      case "promise-suspend" -> EventKind.PROMISE_SUSPEND;
      case "promise-resume" -> EventKind.PROMISE_RESUME;
      case "machine-suspend" -> EventKind.MACHINE_SUSPEND;
      case "machine-resume" -> EventKind.MACHINE_RESUME;
      case "protocol-call" -> EventKind.PROTOCOL_CALL;
      case "execution-terminal" -> EventKind.EXECUTION_TERMINAL;
      default -> throw new IllegalArgumentException("unsupported event " + value);
    };
  }

  private static String phaseName(EventPhase phase) {
    return phase == EventPhase.LIVE ? "live" : "replay";
  }

  private static TargetKind targetKind(String value) {
    return switch (value) {
      case "interpreter" -> TargetKind.INTERPRETER;
      case "hbc" -> TargetKind.HBC;
      case "whole-wasm" -> TargetKind.WHOLE_WASM;
      default -> throw new IllegalArgumentException("unsupported target kind " + value);
    };
  }

  private static Map<String, Object> object(Object... fields) {
    if ((fields.length & 1) != 0) throw new IllegalArgumentException("object requires key/value pairs");
    Map<String, Object> value = new LinkedHashMap<>();
    for (int index = 0; index < fields.length; index += 2) value.put((String) fields[index], fields[index + 1]);
    return value;
  }

  private static JsonValue.Array asArray(JsonValue value) {
    if (!(value instanceof JsonValue.Array result)) throw new IllegalArgumentException("expected JSON array");
    return result;
  }

  private static List<JsonValue> array(JsonValue.Object value, String key) {
    return asArray(value(value, key)).values();
  }

  private static JsonValue.Object asObject(JsonValue value) {
    if (!(value instanceof JsonValue.Object result)) throw new IllegalArgumentException("expected JSON object");
    return result;
  }

  private static JsonValue.Object asObject(JsonValue.Object value, String key) {
    return asObject(value(value, key));
  }

  private static JsonValue.Object optionalObject(JsonValue.Object value, String key) {
    JsonValue result = expectedValue(value, key);
    return result == null || result instanceof JsonValue.Null ? null : asObject(result);
  }

  private static JsonValue value(JsonValue.Object value, String key) {
    JsonValue result = expectedValue(value, key);
    if (result == null) throw new IllegalArgumentException("missing field " + key);
    return result;
  }

  private static JsonValue expectedValue(JsonValue.Object value, String key) {
    return value.values().get(key);
  }

  private static String requiredString(JsonValue.Object value, String key) {
    JsonValue result = value(value, key);
    if (!(result instanceof JsonValue.String string)) throw new IllegalArgumentException("missing string field " + key);
    return string.value();
  }

  private static String optionalString(JsonValue.Object value, String key) {
    JsonValue result = expectedValue(value, key);
    return result instanceof JsonValue.String string ? string.value() : null;
  }

  private static String stringValue(JsonValue value, String label) {
    if (!(value instanceof JsonValue.String string)) throw new IllegalArgumentException(label + " must be a string");
    return string.value();
  }

  private static boolean bool(JsonValue.Object value, String key) {
    JsonValue result = expectedValue(value, key);
    return result instanceof JsonValue.Bool flag && flag.value();
  }

  private static long optionalNumber(JsonValue.Object value, String key, long fallback) {
    JsonValue result = expectedValue(value, key);
    return result == null ? fallback : number(result);
  }

  private static long number(JsonValue value) {
    if (value instanceof JsonValue.Integer integer) return integer.value();
    if (value instanceof JsonValue.BigIntegerValue integer) return integer.value().longValueExact();
    throw new IllegalArgumentException("expected JSON integer");
  }

  private static void expectString(JsonValue.Object value, String key, String actual, String id) {
    JsonValue expected = expectedValue(value, key);
    if (expected instanceof JsonValue.String string && !string.value().equals(actual)) {
      throw new IllegalStateException(id + ": expected " + key + " " + string.value() + ", got " + actual);
    }
  }

  private static void expectLong(JsonValue.Object value, String key, long actual, String id) {
    JsonValue expected = expectedValue(value, key);
    if (expected != null && number(expected) != actual) {
      throw new IllegalStateException(id + ": expected " + key + " " + number(expected) + ", got " + actual);
    }
  }

  private static Object toJava(JsonValue value) {
    if (value == null || value instanceof JsonValue.Null) return null;
    if (value instanceof JsonValue.Bool result) return result.value();
    if (value instanceof JsonValue.Integer result) return result.value();
    if (value instanceof JsonValue.BigIntegerValue result) return result.value();
    if (value instanceof JsonValue.String result) return result.value();
    if (value instanceof JsonValue.Array result) {
      return result.values().stream().map(InstrumentationConformance::toJava).toList();
    }
    Map<String, Object> result = new LinkedHashMap<>();
    for (Map.Entry<String, JsonValue> entry : ((JsonValue.Object) value).values().entrySet()) {
      result.put(entry.getKey(), toJava(entry.getValue()));
    }
    return result;
  }

  private static String message(Exception failure) {
    return failure.getMessage() == null ? failure.getClass().getSimpleName() : failure.getMessage();
  }
}
