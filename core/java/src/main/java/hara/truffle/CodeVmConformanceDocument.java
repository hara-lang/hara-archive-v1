package hara.truffle;

import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/** JSON-safe report projection shared with the Rust/browser code.vm conformance contract. */
final class CodeVmConformanceDocument {
  private CodeVmConformanceDocument() {}

  static String json(CodeVmConformance.Report report, boolean browserOnly, boolean pretty) {
    return Json.write(value(report, browserOnly), pretty);
  }

  static Map<String, Object> value(CodeVmConformance.Report report, boolean browserOnly) {
    List<CodeVmConformance.Observation> selected =
        report.cases().stream()
            .filter(observation -> !browserOnly || observation.testCase().browserSafe())
            .toList();
    int checks = selected.stream().mapToInt(value -> value.checks().size()).sum();
    int passed =
        (int)
            selected.stream()
                .flatMap(value -> value.checks().stream())
                .filter(CodeVmConformance.Check::pass)
                .count();
    int failed = checks - passed;

    return object(
        "schema", CodeVmConformance.REPORT_SCHEMA,
        "view", browserOnly ? "browser" : "complete",
        "status", failed == 0 ? "passed" : "failed",
        "terminalNeutral", true,
        "corpus",
            object(
                "schema", report.corpus().schema(),
                "id", report.corpus().id(),
                "upstream", report.corpus().upstream()),
        "summary",
            object(
                "cases", selected.size(),
                "checks", checks,
                "passed", passed,
                "failed", failed),
        "runtimeMatrix", runtimeMatrix(),
        "cases", selected.stream().map(CodeVmConformanceDocument::caseValue).toList());
  }

  private static Map<String, Object> runtimeMatrix() {
    return object(
        "rust",
            object(
                "supported", false,
                "status", "not-executed",
                "reason", "truffle-runner-scope"),
        "wasm",
            object(
                "supported", false,
                "status", "not-executed",
                "reason", "truffle-runner-scope"),
        "truffle",
            object(
                "supported", true,
                "scope", "interpreter",
                "interpreter", "production-evaluation-journal",
                "corpus", "shared-production-corpus",
                "halc", "rust-owned-production-trace",
                "bytecode", "rust-owned-production-trace"));
  }

  private static Map<String, Object> caseValue(
      CodeVmConformance.Observation observation) {
    CodeVmConformance.CorpusCase testCase = observation.testCase();
    return object(
        "id", testCase.id(),
        "upstreamId", testCase.upstreamId(),
        "sourceId", testCase.sourceId(),
        "namespace", testCase.namespace(),
        "resource", testCase.resource(),
        "source", testCase.source(),
        "browserSafe", testCase.browserSafe(),
        "expected", expectedValue(testCase.expected()),
        "passed", observation.passed(),
        "stages",
            object(
                "interpreter",
                    object(
                        "required", testCase.interpreterRequired(),
                        "outcome", outcomeValue(observation.outcome()),
                        "trace", journalValue(observation.journal(), testCase.sourceId())),
                "halc", unsupportedStage("rust-production-runner"),
                "bytecode", unsupportedStage("rust-production-runner")),
        "checks",
            observation.checks().stream()
                .map(CodeVmConformanceDocument::checkValue)
                .toList(),
        "teaching",
            observation.teaching().stream()
                .map(CodeVmConformanceDocument::annotationValue)
                .toList());
  }

  private static Map<String, Object> unsupportedStage(String owner) {
    return object(
        "supported", false,
        "status", "unsupported",
        "owner", owner,
        "fallback", false,
        "trace", null);
  }

  private static Map<String, Object> expectedValue(
      CodeVmConformance.Expected expected) {
    return switch (expected.kind()) {
      case DISPLAY -> object("status", "returned", "display", expected.value());
      case ERROR_CATEGORY ->
          object("status", "error", "category", expected.value());
      case COMPILE_ERROR ->
          object("status", "compile-error", "marker", expected.value());
    };
  }

  private static Map<String, Object> outcomeValue(
      CodeVmConformance.Outcome outcome) {
    return object(
        "status", outcome.status(),
        "display", outcome.display(),
        "category", outcome.category(),
        "message", outcome.message(),
        "truncated", outcome.truncated());
  }

  private static Object journalValue(
      EvaluationJournal.Journal journal, String sourceId) {
    if (journal == null) return null;
    return object(
        "schema", journal.schema(),
        "id", journal.id(),
        "sourceId", sourceId,
        "status", journal.status(),
        "events",
            journal.events().stream()
                .map(CodeVmConformanceDocument::eventValue)
                .toList(),
        "result", previewValue(journal.result()),
        "error", journal.error());
  }

  private static Map<String, Object> eventValue(EvaluationJournal.Event event) {
    return object(
        "id", event.id(),
        "sequence", event.sequence(),
        "kind", event.kind(),
        "operation", event.operation(),
        "parentOperation", event.parent(),
        "depth", event.depth(),
        "function", event.name(),
        "values",
            event.values().stream()
                .map(CodeVmConformanceDocument::previewValue)
                .toList(),
        "message", event.message());
  }

  private static Object previewValue(EvaluationJournal.ValuePreview preview) {
    if (preview == null) return null;
    return object(
        "type", preview.type(),
        "display", preview.display(),
        "truncated", preview.truncated());
  }

  private static Map<String, Object> checkValue(CodeVmConformance.Check check) {
    return object(
        "id", check.id(),
        "pass", check.pass(),
        "expected", check.expected(),
        "actual", check.actual());
  }

  private static Map<String, Object> annotationValue(
      CodeVmConformance.TeachingAnnotation annotation) {
    return object(
        "concept", annotation.concept(),
        "stage", annotation.stage(),
        "sequence", annotation.sequence(),
        "detail", annotation.detail());
  }

  private static Map<String, Object> object(Object... fields) {
    if ((fields.length & 1) != 0) {
      throw new IllegalArgumentException("object requires key/value pairs");
    }
    Map<String, Object> value = new LinkedHashMap<>();
    for (int index = 0; index < fields.length; index += 2) {
      String key = (String) fields[index];
      if (value.containsKey(key)) {
        throw new IllegalArgumentException("duplicate JSON field: " + key);
      }
      value.put(key, fields[index + 1]);
    }
    return value;
  }

  static final class Json {
    private Json() {}

    static String write(Object value, boolean pretty) {
      StringBuilder output = new StringBuilder();
      append(output, value, 0, pretty);
      return output.toString();
    }

    private static void append(
        StringBuilder output, Object value, int depth, boolean pretty) {
      if (value == null) {
        output.append("null");
      } else if (value instanceof Boolean
          || value instanceof Byte
          || value instanceof Short
          || value instanceof Integer
          || value instanceof Long) {
        output.append(value);
      } else if (value instanceof String text) {
        string(output, text);
      } else if (value instanceof Map<?, ?> map) {
        map(output, map, depth, pretty);
      } else if (value instanceof Iterable<?> values) {
        array(output, values, depth, pretty);
      } else {
        throw new IllegalArgumentException(
            "unsupported report JSON value: " + value.getClass());
      }
    }

    private static void map(
        StringBuilder output, Map<?, ?> values, int depth, boolean pretty) {
      output.append('{');
      int index = 0;
      for (Map.Entry<?, ?> entry : values.entrySet()) {
        if (!(entry.getKey() instanceof String key)) {
          throw new IllegalArgumentException(
              "report JSON object keys must be strings");
        }
        if (index++ > 0) output.append(',');
        if (pretty) newline(output, depth + 1);
        string(output, key);
        output.append(pretty ? ": " : ":");
        append(output, entry.getValue(), depth + 1, pretty);
      }
      if (pretty && !values.isEmpty()) newline(output, depth);
      output.append('}');
    }

    private static void array(
        StringBuilder output, Iterable<?> values, int depth, boolean pretty) {
      List<Object> retained = new ArrayList<>();
      for (Object value : values) retained.add(value);
      output.append('[');
      for (int index = 0; index < retained.size(); index++) {
        if (index > 0) output.append(',');
        if (pretty) newline(output, depth + 1);
        append(output, retained.get(index), depth + 1, pretty);
      }
      if (pretty && !retained.isEmpty()) newline(output, depth);
      output.append(']');
    }

    private static void newline(StringBuilder output, int depth) {
      output.append('\n').append("  ".repeat(depth));
    }

    private static void string(StringBuilder output, String value) {
      output.append('"');
      for (int index = 0; index < value.length(); index++) {
        char character = value.charAt(index);
        switch (character) {
          case '"' -> output.append("\\\"");
          case '\\' -> output.append("\\\\");
          case '\b' -> output.append("\\b");
          case '\f' -> output.append("\\f");
          case '\n' -> output.append("\\n");
          case '\r' -> output.append("\\r");
          case '\t' -> output.append("\\t");
          default -> {
            if (character < 0x20) {
              output.append(String.format("\\u%04x", (int) character));
            } else {
              output.append(character);
            }
          }
        }
      }
      output.append('"');
    }
  }
}
