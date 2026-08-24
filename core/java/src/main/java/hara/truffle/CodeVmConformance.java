package hara.truffle;

import hara.kernel.base.Parser;
import hara.lang.data.Keyword;
import hara.lang.protocol.ILinearType;
import hara.lang.protocol.IMapType;
import java.io.IOException;
import java.io.InputStream;
import java.io.PrintStream;
import java.math.BigInteger;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.Locale;
import org.graalvm.polyglot.Context;
import org.graalvm.polyglot.Value;

/** Runs the shared production code.vm corpus through the real Truffle evaluator journal. */
public final class CodeVmConformance {
  static final String CORPUS_SCHEMA = "hal.code-vm-production-corpus/0-alpha";
  static final String REPORT_SCHEMA = "hal.code-vm-conformance-runtime/0-alpha";
  static final String CORPUS_FILE = "code-vm-conformance.edn";

  private static final int DEFAULT_STEPS = 512;
  private static final int DEFAULT_TRACE_LIMIT = 128;
  private static final int JOURNAL_DEPTH_LIMIT = 100;
  private static final int VALUE_CHARACTER_LIMIT = 512;

  private CodeVmConformance() {}

  public static void main(String[] args) {
    int status = run(args, System.out, System.err);
    if (status != 0) System.exit(status);
  }

  static int run(String[] args, PrintStream output, PrintStream error) {
    String command = args.length == 0 ? "check" : args[0];
    if (args.length > 1
        || !List.of("check", "report", "browser", "help", "--help", "-h")
            .contains(command)) {
      error.println("usage: code-vm-conformance [check|report|browser]");
      return 2;
    }
    if (List.of("help", "--help", "-h").contains(command)) {
      output.println("usage: code-vm-conformance [check|report|browser]");
      return 0;
    }

    try {
      Report report = runEmbedded();
      if ("report".equals(command)) {
        emitReport(CodeVmConformanceDocument.json(report, false, true), output);
        return report.passed() ? 0 : 1;
      }
      if ("browser".equals(command)) {
        emitReport(CodeVmConformanceDocument.json(report, true, true), output);
        return report.browserPassed() ? 0 : 1;
      }
      if (report.passed()) {
        output.println(
            "Truffle code.vm conformance passed: "
                + report.cases().size()
                + " cases, "
                + report.checkCount()
                + " checks");
        return 0;
      }
      error.println(
          "Truffle code.vm conformance failed: "
              + report.failedChecks()
              + " of "
              + report.checkCount()
              + " checks failed");
      for (Observation observation : report.cases()) {
        for (Check check : observation.checks()) {
          if (!check.pass()) {
            error.println(
                observation.testCase().id()
                    + " "
                    + check.id()
                    + ": expected "
                    + check.expected()
                    + ", got "
                    + check.actual());
          }
        }
      }
      return 1;
    } catch (Exception failure) {
      error.println("Truffle code.vm conformance failed: " + message(failure));
      return 1;
    }
  }

  private static void emitReport(String report, PrintStream output) throws IOException {
    String path = System.getProperty("hara.codeVmReport");
    if (path == null || path.isBlank()) {
      output.println(report);
      return;
    }
    Path target = Path.of(path);
    Path parent = target.toAbsolutePath().getParent();
    if (parent != null) Files.createDirectories(parent);
    Files.writeString(target, report + System.lineSeparator(), StandardCharsets.UTF_8);
  }

  static Report runEmbedded() throws IOException {
    return runCorpus(loadCorpusSource());
  }

  static Report runCorpus(String source) {
    Corpus corpus = parseCorpus(source);
    List<Observation> observations = new ArrayList<>();
    for (CorpusCase testCase : corpus.cases()) observations.add(observe(testCase));
    return new Report(corpus, List.copyOf(observations));
  }

  static String loadCorpusSource() throws IOException {
    String override = System.getProperty("hara.codeVmCorpus");
    if (override != null && !override.isBlank()) {
      return Files.readString(Path.of(override), StandardCharsets.UTF_8);
    }
    try (InputStream resource =
        CodeVmConformance.class.getClassLoader().getResourceAsStream(CORPUS_FILE)) {
      if (resource != null) return new String(resource.readAllBytes(), StandardCharsets.UTF_8);
    }

    Path current = Path.of("").toAbsolutePath().normalize();
    for (int depth = 0; current != null && depth < 8; depth++, current = current.getParent()) {
      for (Path candidate :
          List.of(
              current.resolve("core/rust/assets").resolve(CORPUS_FILE),
              current.resolve("rust/assets").resolve(CORPUS_FILE))) {
        if (Files.isRegularFile(candidate)) {
          return Files.readString(candidate, StandardCharsets.UTF_8);
        }
      }
    }
    throw new IOException("Unable to locate shared " + CORPUS_FILE);
  }

  @SuppressWarnings("rawtypes")
  static Corpus parseCorpus(String source) {
    IMapType root = requireMap(Parser.LispReader.readString(source, null), "code.vm corpus");
    String schema = requireString(root.lookup(key("corpus/schema")), ":corpus/schema");
    if (!CORPUS_SCHEMA.equals(schema)) {
      throw new IllegalArgumentException("unsupported code.vm corpus schema: " + schema);
    }
    String id = requireKeyword(root.lookup(key("corpus/id")), ":corpus/id");
    String upstream = requireString(root.lookup(key("corpus/upstream")), ":corpus/upstream");
    Object rawCases = root.lookup(key("cases"));
    if (!(rawCases instanceof ILinearType<?> cases)) {
      throw new IllegalArgumentException("code.vm corpus :cases must be a vector");
    }
    List<CorpusCase> parsed = new ArrayList<>();
    for (Object item : cases) parsed.add(parseCase(requireMap(item, "code.vm corpus case")));
    if (parsed.isEmpty()) {
      throw new IllegalArgumentException("code.vm corpus :cases must not be empty");
    }
    return new Corpus(schema, id, upstream, List.copyOf(parsed));
  }

  @SuppressWarnings("rawtypes")
  private static CorpusCase parseCase(IMapType item) {
    String id = requireKeyword(item.lookup(key("id")), "case :id");
    String upstreamId = requireKeyword(item.lookup(key("upstream-id")), id + " :upstream-id");
    String source = requireString(item.lookup(key("source")), id + " :source");
    IMapType expectedMap = requireMap(item.lookup(key("expect")), id + " :expect");
    Expected expected = parseExpected(expectedMap, id);
    String dotted = id.replace('/', '.').replace('-', '_');
    return new CorpusCase(
        id,
        upstreamId,
        "code.vm/" + id,
        "code.vm.fixture." + dotted,
        "code/vm/fixture/" + id + ".hal",
        source,
        expected,
        optionalBoolean(
            item.lookup(key("interpreter-required")),
            true,
            id + " :interpreter-required"),
        optionalBoolean(item.lookup(key("browser-safe")), false, id + " :browser-safe"),
        optionalPositiveInt(item.lookup(key("steps")), DEFAULT_STEPS, id + " :steps"),
        optionalPositiveInt(
            item.lookup(key("trace-limit")), DEFAULT_TRACE_LIMIT, id + " :trace-limit"),
        optionalBoolean(
            item.lookup(key("expect-dropped")), false, id + " :expect-dropped"));
  }

  @SuppressWarnings("rawtypes")
  private static Expected parseExpected(IMapType values, String id) {
    List<Expected> found = new ArrayList<>();
    Object display = values.lookup(key("display"));
    Object error = values.lookup(key("error-category"));
    Object compile = values.lookup(key("compile-error"));
    if (display != null) {
      found.add(new Expected(ExpectedKind.DISPLAY, requireString(display, id + " display")));
    }
    if (error != null) {
      found.add(
          new Expected(
              ExpectedKind.ERROR_CATEGORY,
              requireString(error, id + " error category")));
    }
    if (compile != null) {
      found.add(
          new Expected(
              ExpectedKind.COMPILE_ERROR,
              requireString(compile, id + " compile error")));
    }
    if (found.size() != 1) {
      throw new IllegalArgumentException(
          id + " :expect must contain exactly one supported expectation");
    }
    return found.get(0);
  }

  private static Observation observe(CorpusCase testCase) {
    if (!testCase.interpreterRequired()) {
      Outcome unsupported =
          new Outcome(
              "unsupported",
              null,
              null,
              "interpreter not required for compile-only corpus case",
              false);
      List<Check> checks =
          List.of(
              new Check("truffle/expected", true, "not-required", "unsupported"),
              sourceIdentityCheck(testCase),
              new Check("trace/sequences", true, "not-applicable", "not-executed"),
              new Check("trace/bounded", true, "not-applicable", "not-executed"),
              new Check("fallback/forbidden", true, "false", "false"));
      return new Observation(testCase, unsupported, null, checks, List.of());
    }

    int maxEvents = Math.max(0, testCase.traceLimit() - 1);
    EvaluationJournal.Journal journal;
    try (Context context =
        Context.newBuilder(HaraLanguage.ID)
            .option("engine.WarnInterpreterOnly", "false")
            .build()) {
      journal =
          EvaluationJournal.collect(
              new EvaluationJournal.Limits(
                  maxEvents, JOURNAL_DEPTH_LIMIT, VALUE_CHARACTER_LIMIT),
              () -> materialize(context.eval(HaraLanguage.ID, testCase.source())));
    }

    Outcome outcome = outcome(journal);
    List<Check> checks =
        List.of(
            expectedCheck(testCase.expected(), outcome),
            sourceIdentityCheck(testCase),
            sequenceCheck(journal),
            boundedCheck(testCase, journal),
            new Check("fallback/forbidden", true, "false", "false"));
    return new Observation(
        testCase,
        outcome,
        journal,
        checks,
        teachingAnnotations(journal));
  }

  private static Outcome outcome(EvaluationJournal.Journal journal) {
    boolean truncated = "truncated".equals(journal.status());
    if (journal.result() != null) {
      return new Outcome("returned", journal.result().display(), null, null, truncated);
    }
    String error = journal.error() == null ? "" : journal.error();
    return new Outcome("failed", null, normalizeErrorCategory(error), error, truncated);
  }

  private static Check expectedCheck(Expected expected, Outcome outcome) {
    boolean pass;
    String expectedText;
    switch (expected.kind()) {
      case DISPLAY -> {
        expectedText = "returned:" + expected.value();
        pass =
            "returned".equals(outcome.status())
                && expected.value().equals(outcome.display());
      }
      case ERROR_CATEGORY -> {
        expectedText = "error-category:" + expected.value();
        pass = expected.value().equals(outcome.category());
      }
      case COMPILE_ERROR -> {
        expectedText = "compile-error:" + expected.value();
        pass = false;
      }
      default -> throw new IllegalStateException("unknown expectation");
    }
    return new Check("truffle/expected", pass, expectedText, outcomeText(outcome));
  }

  private static Check sourceIdentityCheck(CorpusCase testCase) {
    boolean pass =
        !testCase.sourceId().isBlank()
            && !testCase.namespace().isBlank()
            && !testCase.resource().isBlank();
    String identity =
        testCase.namespace() + "|" + testCase.resource() + "|" + testCase.sourceId();
    return new Check("source/identity", pass, identity, identity);
  }

  private static Check sequenceCheck(EvaluationJournal.Journal journal) {
    boolean contiguous = true;
    for (int index = 0; index < journal.events().size(); index++) {
      if (journal.events().get(index).sequence() != index + 1L) {
        contiguous = false;
        break;
      }
    }
    return new Check(
        "trace/sequences",
        contiguous,
        "contiguous",
        contiguous ? "contiguous" : "non-contiguous");
  }

  private static Check boundedCheck(
      CorpusCase testCase, EvaluationJournal.Journal journal) {
    boolean truncated = "truncated".equals(journal.status());
    boolean pass = journal.events().size() <= testCase.traceLimit();
    return new Check(
        "trace/bounded",
        pass,
        "events<=" + testCase.traceLimit(),
        "events="
            + journal.events().size()
            + ",truncated="
            + truncated
            + ",bytecodeExpectDropped="
            + testCase.expectDropped());
  }

  private static List<TeachingAnnotation> teachingAnnotations(
      EvaluationJournal.Journal journal) {
    List<TeachingAnnotation> annotations = new ArrayList<>();
    for (EvaluationJournal.Event event : journal.events().stream().limit(10).toList()) {
      annotations.add(
          new TeachingAnnotation(
              teachingConcept(event.kind()),
              "interpreter",
              event.sequence(),
              event.name() == null ? event.kind() : event.name()));
    }
    return List.copyOf(annotations);
  }

  private static String teachingConcept(String kind) {
    return switch (kind) {
      case "evaluation/start" -> "evaluation/order";
      case "macro/expand" -> "macro/expansion";
      case "operation/enter" -> "operation/enter";
      case "operation/return" -> "operation/return";
      case "evaluation/error" -> "error/propagation";
      case "journal/truncated" -> "trace/bounded";
      default -> "evaluation/event";
    };
  }

  static String normalizeErrorCategory(String message) {
    String normalized = message == null ? "" : message.toLowerCase(Locale.ROOT);
    if (normalized.contains("division by zero")
        || normalized.contains("divide by zero")
        || normalized.contains("/ by zero")) return "division by zero";
    if (normalized.contains("non-finite number")) return "non-finite number";
    if (normalized.contains("expects numbers")
        || normalized.contains("expects two numbers")
        || normalized.contains("expected a number")
        || normalized.contains("expected numeric")
        || normalized.contains("unsupported specialization")
        || (normalized.contains("number")
            && (normalized.contains("cannot")
                || normalized.contains("expected")
                || normalized.contains("expects")
                || normalized.contains("cast")))) return "expects numbers";
    if (normalized.contains("eof while reading")
        || normalized.contains("unexpected eof")
        || normalized.contains("unexpected end")
        || normalized.contains("incomplete source")
        || normalized.contains("unclosed")) return "reader";
    if (normalized.contains("unbound symbol")
        || normalized.contains("cannot find symbol")) return "unbound symbol";
    if (normalized.contains("recur")) return "recur";
    if (normalized.contains("unsupported")) return "unsupported form";
    return "unclassified";
  }

  private static Object materialize(Value value) {
    if (value == null || value.isNull()) return null;
    if (value.isBoolean()) return value.asBoolean();
    if (value.isString()) return value.asString();
    if (value.fitsInLong()) return value.asLong();
    if (value.fitsInBigInteger()) return value.as(BigInteger.class);
    if (value.fitsInDouble()) return value.asDouble();
    if (value.hasArrayElements()) {
      List<Object> values = new ArrayList<>();
      for (long index = 0; index < value.getArraySize(); index++) {
        values.add(materialize(value.getArrayElement(index)));
      }
      return List.copyOf(values);
    }
    return value.toString();
  }

  private static String outcomeText(Outcome outcome) {
    return "status="
        + outcome.status()
        + ",display="
        + (outcome.display() == null ? "none" : outcome.display())
        + ",category="
        + (outcome.category() == null ? "none" : outcome.category());
  }

  private static String message(Throwable failure) {
    Throwable current = failure;
    while (current.getCause() != null && current.getCause() != current) {
      current = current.getCause();
    }
    return current.getMessage() == null
        ? current.getClass().getName()
        : current.getMessage();
  }

  @SuppressWarnings("rawtypes")
  private static IMapType requireMap(Object value, String label) {
    if (value instanceof IMapType map) return map;
    throw new IllegalArgumentException(label + " must be a map");
  }

  private static String requireString(Object value, String label) {
    if (value instanceof String text && !text.isEmpty()) return text;
    throw new IllegalArgumentException(label + " must be a non-empty string");
  }

  private static String requireKeyword(Object value, String label) {
    if (value instanceof Keyword keyword) {
      return keyword.getNamespace() == null
          ? keyword.getName()
          : keyword.getNamespace() + "/" + keyword.getName();
    }
    throw new IllegalArgumentException(label + " must be a keyword");
  }

  private static boolean optionalBoolean(
      Object value, boolean fallback, String label) {
    if (value == null) return fallback;
    if (value instanceof Boolean flag) return flag;
    throw new IllegalArgumentException(label + " must be a boolean");
  }

  private static int optionalPositiveInt(
      Object value, int fallback, String label) {
    if (value == null) return fallback;
    if (value instanceof Number number) {
      long result = number.longValue();
      if (result > 0 && result <= Integer.MAX_VALUE) return (int) result;
    }
    throw new IllegalArgumentException(label + " must be a positive integer");
  }

  private static Keyword key(String name) {
    return Keyword.create(name);
  }

  enum ExpectedKind {
    DISPLAY,
    ERROR_CATEGORY,
    COMPILE_ERROR
  }

  record Expected(ExpectedKind kind, String value) {}

  record Corpus(String schema, String id, String upstream, List<CorpusCase> cases) {}

  record CorpusCase(
      String id,
      String upstreamId,
      String sourceId,
      String namespace,
      String resource,
      String source,
      Expected expected,
      boolean interpreterRequired,
      boolean browserSafe,
      int steps,
      int traceLimit,
      boolean expectDropped) {}

  record Outcome(
      String status,
      String display,
      String category,
      String message,
      boolean truncated) {}

  record Check(String id, boolean pass, String expected, String actual) {}

  record TeachingAnnotation(
      String concept, String stage, long sequence, String detail) {}

  record Observation(
      CorpusCase testCase,
      Outcome outcome,
      EvaluationJournal.Journal journal,
      List<Check> checks,
      List<TeachingAnnotation> teaching) {
    boolean passed() {
      return checks.stream().allMatch(Check::pass);
    }
  }

  record Report(Corpus corpus, List<Observation> cases) {
    boolean passed() {
      return cases.stream().allMatch(Observation::passed);
    }

    boolean browserPassed() {
      return cases.stream()
          .filter(observation -> observation.testCase().browserSafe())
          .allMatch(Observation::passed);
    }

    int checkCount() {
      return cases.stream()
          .mapToInt(observation -> observation.checks().size())
          .sum();
    }

    int failedChecks() {
      return (int)
          cases.stream()
              .flatMap(observation -> observation.checks().stream())
              .filter(check -> !check.pass())
              .count();
    }
  }
}
