package hara.truffle;

import hara.truffle.InstrumentationModel.Capability;
import hara.truffle.InstrumentationModel.EventDelivery;
import hara.truffle.InstrumentationModel.DeliveredEvent;
import hara.truffle.InstrumentationModel.EventKind;
import hara.truffle.InstrumentationModel.EventLocation;
import hara.truffle.InstrumentationModel.EventPhase;
import hara.truffle.InstrumentationModel.InstrumentFilter;
import hara.truffle.InstrumentationModel.InstrumentHandle;
import hara.truffle.InstrumentationModel.InstrumentMode;
import hara.truffle.InstrumentationModel.InstrumentRegistration;
import hara.truffle.InstrumentationModel.ProjectionRequest;
import hara.truffle.InstrumentationModel.RuntimeBackend;
import hara.truffle.InstrumentationModel.SourceSpan;
import hara.truffle.InstrumentationModel.TargetDescriptor;
import hara.truffle.InstrumentationModel.TargetHandle;
import hara.truffle.InstrumentationModel.TargetKind;
import java.io.PrintStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.TreeSet;
import org.graalvm.polyglot.Context;

/** Produces the shared machine-readable instrumentation conformance report. */
public final class InstrumentationConformance {
  private static final String CORPUS_SCHEMA = "hara.instrumentation.conformance-corpus/0-alpha";
  private static final String REPORT_SCHEMA = "hara.instrumentation.conformance-report/0-alpha";

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
      List<Object> cases = new ArrayList<>();
      for (JsonValue value : array(corpus, "cases")) cases.add(observe(asObject(value)));
      Map<String, Object> report = object(
          "schema", REPORT_SCHEMA,
          "corpus", object("schema", requiredString(corpus, "schema"), "id", requiredString(corpus, "id")),
          "runtime", "java",
          "cases", cases);
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
    if ("state".equals(requiredString(testCase, "kind"))) {
      return observeState(testCase, id);
    }
    TargetKind targetKind = targetKind(requiredString(testCase, "targetKind"));
    List<JsonValue> sourceEvents = array(testCase, "events");
    TreeSet<EventKind> events = new TreeSet<>();
    TreeSet<Capability> capabilities = new TreeSet<>();
    for (JsonValue sourceEvent : sourceEvents) {
      EventKind event = eventKind(requiredString(asObject(sourceEvent), "event"));
      events.add(event);
      capabilities.add(event.requiredCapability());
    }
    capabilities.add(Capability.INSPECT_SOURCE_LOCATION);
    String session = "instrum-alpha";
    try (InstrumentationHub hub = new InstrumentationHub()) {
      String targetId = id + "/target";
      TargetDescriptor descriptor = new TargetDescriptor(
          targetId, session, targetKind, new RuntimeBackend("java"), capabilities);
      TargetHandle target = hub.registerTarget(descriptor);
      InstrumentRegistration registration = new InstrumentRegistration(
          id + "/instrument", session, InstrumentMode.PASSIVE, capabilities, events,
          InstrumentFilter.all(),
          new ProjectionRequest(true, null, null, null, null, null, null),
          EventDelivery.queue(32));
      InstrumentHandle instrument = hub.registerInstrument(registration);
      hub.attach(instrument, target);
      for (JsonValue sourceEvent : sourceEvents) {
        JsonValue.Object event = asObject(sourceEvent);
        hub.publish(target,
            eventKind(requiredString(event, "event")),
            phase(requiredString(event, "phase")),
            location(event.values().get("location")),
            data(event.values().get("data")));
      }
      List<Map<String, Object>> actual = hub.drain(instrument).events().stream()
          .map(InstrumentationConformance::eventValue)
          .toList();
      List<Map<String, Object>> expected = sourceEvents.stream()
          .map(value -> canonicalEvent(asObject(value)))
          .toList();
      if (!actual.equals(expected)) throw new IllegalStateException(id + ": produced events differ from corpus");
      return object("id", id, "kind", "events", "targetKind", targetKind.toString(), "events", actual);
    }
  }

  @SuppressWarnings("unchecked")
  private static Map<String, Object> observeState(JsonValue.Object testCase, String id) {
    Map<String, Object> state = new LinkedHashMap<>((Map<String, Object>) toJava(testCase.values().get("initial")));
    for (JsonValue value : array(testCase, "operations")) {
      Map<String, Object> operation = (Map<String, Object>) toJava(value);
      String name = (String) operation.get("operation");
      switch (name) {
        case "run", "evaluate" -> {
          state.put("status", operation.get("status"));
          state.put("eventSequence", operation.get("eventSequence"));
          if (operation.containsKey("source")) {
            String observed = evaluateStateSource((String) operation.get("source"));
            String expected = (String) operation.get("result");
            if (!expected.equals(observed)) {
              throw new IllegalStateException(
                  id + ": bytecode result mismatch: expected " + expected + ", got " + observed);
            }
            state.put("result", observed);
          } else if (operation.containsKey("result")) {
            state.put("result", operation.get("result"));
          }
        }
        case "reset" -> {
          long generation = ((Number) state.get("generation")).longValue();
          long delta = operation.containsKey("generationDelta")
              ? ((Number) operation.get("generationDelta")).longValue()
              : 1L;
          state.put("generation", generation + delta);
          state.put("status", operation.get("status"));
          state.put("eventSequence", operation.get("eventSequence"));
          if (Boolean.TRUE.equals(operation.get("removeResult"))) state.remove("result");
        }
        default -> throw new IllegalArgumentException(id + ": unsupported state operation " + name);
      }
    }
    Map<String, Object> expected = (Map<String, Object>) toJava(testCase.values().get("state"));
    if (!state.equals(expected)) {
      throw new IllegalStateException(id + ": state transitions differ from corpus");
    }
    return object("id", id, "kind", "state", "state", state);
  }

  private static String evaluateStateSource(String source) {
    try (Context context =
        Context.newBuilder(HaraLanguage.ID)
            .option("engine.WarnInterpreterOnly", "false")
            .build()) {
      EvaluationJournal.Journal journal =
          EvaluationJournal.collect(
              new EvaluationJournal.Limits(128, 100, 512),
              () -> context.eval(HaraLanguage.ID, source));
      if (journal.result() == null) {
        throw new IllegalStateException("state source did not return: " + journal.error());
      }
      return journal.result().display();
    }
  }

  private static Map<String, Object> eventValue(DeliveredEvent delivered) {
    var event = delivered.envelope();
    return object(
        "event", eventName(event.event()),
        "phase", phaseName(event.phase()),
        "generation", event.generation(),
        "sequence", event.sequence(),
        "location", locationValue(event.location()),
        "data", event.data());
  }

  private static Map<String, Object> canonicalEvent(JsonValue.Object event) {
    return object(
        "event", requiredString(event, "event"),
        "phase", requiredString(event, "phase"),
        "generation", requiredLong(event, "generation"),
        "sequence", requiredLong(event, "sequence"),
        "location", locationValue(location(event.values().get("location"))),
        "data", data(event.values().get("data")));
  }

  private static EventLocation location(JsonValue value) {
    if (value == null || value instanceof JsonValue.Null) return null;
    JsonValue.Object source = asObject(value);
    List<Integer> formPath = new ArrayList<>();
    JsonValue formPathValue = source.values().get("formPath");
    if (formPathValue != null) {
      for (JsonValue item : asArray(formPathValue).values()) formPath.add(Math.toIntExact(number(item)));
    }
    JsonValue spanValue = source.values().get("span");
    SourceSpan span = null;
    if (spanValue != null) {
      List<JsonValue> values = asArray(spanValue).values();
      span = new SourceSpan(Math.toIntExact(number(values.get(0))), Math.toIntExact(number(values.get(1))));
    }
    JsonValue instruction = source.values().get("instructionPointer");
    return new EventLocation(
        optionalString(source, "sourceId"),
        formPath,
        span,
        optionalString(source, "function"),
        instruction == null || instruction instanceof JsonValue.Null ? null : Math.toIntExact(number(instruction)));
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

  private static Map<String, String> data(JsonValue value) {
    if (value == null || value instanceof JsonValue.Null) return Map.of();
    Map<String, String> result = new LinkedHashMap<>();
    for (Map.Entry<String, JsonValue> entry : asObject(value).values().entrySet()) {
      if (!(entry.getValue() instanceof JsonValue.String string)) {
        throw new IllegalArgumentException("event data values must be strings");
      }
      result.put(entry.getKey(), string.value());
    }
    return result;
  }

  private static JsonValue.Array asArray(JsonValue value) {
    if (!(value instanceof JsonValue.Array result)) throw new IllegalArgumentException("expected JSON array");
    return result;
  }

  private static List<JsonValue> array(JsonValue.Object value, String key) {
    JsonValue source = value.values().get(key);
    return asArray(source).values();
  }

  private static JsonValue.Object asObject(JsonValue value) {
    if (!(value instanceof JsonValue.Object result)) throw new IllegalArgumentException("expected JSON object");
    return result;
  }

  private static String requiredString(JsonValue.Object value, String key) {
    JsonValue source = value.values().get(key);
    if (!(source instanceof JsonValue.String result)) throw new IllegalArgumentException("missing string field " + key);
    return result.value();
  }

  private static String optionalString(JsonValue.Object value, String key) {
    JsonValue source = value.values().get(key);
    return source instanceof JsonValue.String result ? result.value() : null;
  }

  private static long requiredLong(JsonValue.Object value, String key) {
    return number(value.values().get(key));
  }

  private static long number(JsonValue value) {
    if (value instanceof JsonValue.Integer result) return result.value();
    if (value instanceof JsonValue.BigIntegerValue result) return result.value().longValueExact();
    throw new IllegalArgumentException("expected JSON integer");
  }

  private static TargetKind targetKind(String value) {
    return switch (value) {
      case "interpreter" -> TargetKind.INTERPRETER;
      case "hbc" -> TargetKind.HBC;
      case "whole-wasm" -> TargetKind.WHOLE_WASM;
      default -> throw new IllegalArgumentException("unsupported target kind " + value);
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

  private static EventPhase phase(String value) {
    return switch (value) {
      case "live" -> EventPhase.LIVE;
      case "replay" -> EventPhase.REPLAY;
      default -> throw new IllegalArgumentException("unsupported event phase " + value);
    };
  }

  private static String phaseName(EventPhase phase) {
    return phase == EventPhase.LIVE ? "live" : "replay";
  }

  private static Map<String, Object> object(Object... fields) {
    if ((fields.length & 1) != 0) throw new IllegalArgumentException("object requires key/value pairs");
    Map<String, Object> value = new LinkedHashMap<>();
    for (int index = 0; index < fields.length; index += 2) value.put((String) fields[index], fields[index + 1]);
    return value;
  }

  private static Object toJava(JsonValue value) {
    if (value == null || value instanceof JsonValue.Null) return null;
    if (value instanceof JsonValue.Bool result) return result.value();
    if (value instanceof JsonValue.Integer result) return result.value();
    if (value instanceof JsonValue.BigIntegerValue result) return result.value();
    if (value instanceof JsonValue.String result) return result.value();
    if (value instanceof JsonValue.Array result) return result.values().stream().map(InstrumentationConformance::toJava).toList();
    Map<String, Object> result = new LinkedHashMap<>();
    for (Map.Entry<String, JsonValue> entry : ((JsonValue.Object) value).values().entrySet()) result.put(entry.getKey(), toJava(entry.getValue()));
    return result;
  }

  private static String message(Exception failure) {
    return failure.getMessage() == null ? failure.getClass().getSimpleName() : failure.getMessage();
  }
}
