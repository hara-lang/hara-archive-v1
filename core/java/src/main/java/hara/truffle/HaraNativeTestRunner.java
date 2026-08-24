package hara.truffle;

import hara.kernel.base.Parser;
import hara.lang.data.Keyword;
import hara.lang.data.TaggedLiteral;
import hara.lang.protocol.ILinearType;
import hara.lang.protocol.IMapType;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.List;
import java.util.regex.Matcher;
import java.util.regex.Pattern;
import org.graalvm.polyglot.Context;
import org.graalvm.polyglot.Value;
import org.graalvm.polyglot.io.IOAccess;

/** Runs Hara test files in isolated JVM runtime contexts. */
public final class HaraNativeTestRunner {
  private HaraNativeTestRunner() {}

  /** Host-neutral result extracted from a code.test summary or native Result vector. */
  public record Result(
      Path path,
      boolean passed,
      int facts,
      int checks,
      int passedChecks,
      int failedChecks,
      int errors,
      int timeouts,
      String rawSummary) {

    public String failureMessage() {
      return path + " failed: " + rawSummary;
    }
  }

  /** Runs one .hal test file in a fresh GraalVM Hara context. */
  public static Result runFile(Path projectRoot, Path testFile) throws IOException {
    Path root = projectRoot.toAbsolutePath().normalize();
    Path file = testFile.toAbsolutePath().normalize();
    if (!Files.isRegularFile(file)) {
      throw new HaraException("test file not found: " + file);
    }
    if (!file.toString().endsWith(".hal")) {
      throw new HaraException("test file must use the .hal extension: " + file);
    }

    String source = Files.readString(file, StandardCharsets.UTF_8);
    try (Context context =
        Context.newBuilder(HaraLanguage.ID)
            .currentWorkingDirectory(root)
            .allowIO(IOAccess.ALL)
            .allowCreateProcess(true)
            .option("hara.TestRunner", "native")
            .build()) {
      Value value;
      try {
        value = context.eval(HaraLanguage.ID, source);
      } catch (RuntimeException error) {
        throw HaraException.withCause(
            "Unable to load test file: " + file + " (" + error.getMessage() + ")", error);
      }
      Matcher namespace = TEST_NAMESPACE.matcher(source);
      if (namespace.find()
          && namespace.group(1).matches("[A-Za-z0-9_.-]+")
          && !value.hasArrayElements()) {
        try {
          value = context.eval(HaraLanguage.ID, testRunSource(namespace.group(1)));
        } catch (RuntimeException error) {
          throw HaraException.withCause(
              "Unable to execute test namespace: "
                  + namespace.group(1)
                  + " ("
                  + error.getMessage()
                  + ")",
              error);
        }
      }
      return parseResult(file, value);
    }
  }

  private static String testRunSource(String namespace) {
    String fact = System.getProperty("hara.xt.fact");
    String selection = fact == null || fact.isBlank() ? "" : " :name \"" + fact + "\"";
    return "(let [summary (code.test/run {:namespace \""
        + namespace
        + "\"" + selection + "}) failures (filter (fn [result] (not= :passed (:status result))) (:results summary)) diagnostic (map (fn [result] (let [check (first (filter (fn [item] (not (:pass item))) (:checks result))) error (or (:error result) (:error check))] {:name (:name result) :status (:status result) :error (if error (apply str (take 2000 error)) nil) :actual (:actual check) :expected (:expected check)})) failures)] (assoc (dissoc summary :results :report) :results (str diagnostic)))";
  }

  /** Discovers .hal test files from a project descriptor or explicit path. */
  public static List<Path> discover(Path start, Path requested) throws IOException {
    ArrayList<Path> files = new ArrayList<>();
    if (requested != null) {
      Path target = requested.toAbsolutePath().normalize();
      if (Files.isRegularFile(target)) {
        if (!target.toString().endsWith(".hal")) {
          throw new HaraException("test file must use the .hal extension: " + target);
        }
        files.add(target);
      } else if (Files.isDirectory(target)) {
        collect(target, files);
      } else {
        throw new HaraException("test path not found: " + target);
      }
    } else {
      HaraProject project = HaraProject.discover(start);
      if (project == null) {
        throw new HaraException("no project.edn found above " + start);
      }
      for (Path testPath : project.testPaths()) {
        if (Files.exists(testPath)) collect(testPath, files);
      }
    }
    files.sort(Comparator.naturalOrder());
    return List.copyOf(files);
  }

  public static List<Path> discover(Path start) throws IOException {
    return discover(start, null);
  }

  private static void collect(Path directory, List<Path> output) throws IOException {
    try (java.util.stream.Stream<Path> paths = Files.walk(directory)) {
      paths
          .filter(Files::isRegularFile)
          .filter(path -> path.toString().endsWith(".hal"))
          .forEach(output::add);
    }
  }

  @SuppressWarnings({"rawtypes", "unchecked"})
  static Result parseResult(Path path, Value value) {
    if (value.hasArrayElements()) {
      int passed = 0;
      int failed = 0;
      for (long index = 0; index < value.getArraySize(); index++) {
        String item = value.getArrayElement(index).toString();
        if (item.startsWith("#hara/Result[:success true")) passed++;
        else if (item.startsWith("#hara/Result[:success false")
            || item.startsWith("#hara/Result[:error")) failed++;
        else throw new HaraException("direct test result must be a native Result");
      }
      return new Result(
          path,
          failed == 0,
          passed + failed,
          passed + failed,
          passed,
          failed,
          0,
          0,
          value.toString());
    }
    return parseResult(path, value.isString() ? value.asString() : value.toString());
  }

  @SuppressWarnings({"rawtypes", "unchecked"})
  static Result parseResult(Path path, String transfer) {
    Object parsed;
    String summaryText = transfer;
    try {
      parsed = Parser.LispReader.readString(transfer, null);
      if (parsed instanceof String encoded) {
        summaryText = encoded;
        parsed = Parser.LispReader.readString(encoded, null);
      }
    } catch (RuntimeException unreadableDiagnostic) {
      return parsePrintedSummary(path, summaryText, transfer, unreadableDiagnostic);
    }

    if (parsed instanceof IMapType summary) {
      Object status = lookup(summary, "status");
      Object countsValue = lookup(summary, "counts");
      if (!(status instanceof Keyword) || !(countsValue instanceof IMapType counts)) {
        throw new HaraException("code.test/run result is missing :status or :counts");
      }
      int passedFacts = number(counts, "passed", 0);
      int failedFacts = number(counts, "failed", 0);
      int errors = number(counts, "error", 0);
      int timeouts = number(counts, "timeout", 0);
      int skipped = number(counts, "skipped", 0);
      int cancelled = number(counts, "cancelled", 0);
      int facts =
          number(
              summary,
              "facts",
              passedFacts + failedFacts + errors + timeouts + skipped + cancelled);
      int passedChecks = number(summary, "passed", passedFacts);
      int failedChecks = number(summary, "failed", failedFacts);
      int checks = number(summary, "checks", passedChecks + failedChecks);
      boolean passing = Keyword.create("passed").equals(status);
      return new Result(
          path,
          passing,
          facts,
          checks,
          passedChecks,
          failedChecks,
          errors,
          timeouts,
          transfer);
    }

    if (parsed instanceof ILinearType<?> items) {
      int passed = 0;
      int failed = 0;
      for (Object item : items) {
        if (!(item instanceof TaggedLiteral tagged)
            || !"hara/Result".equals(tagged.tag().display())
            || !(tagged.form() instanceof ILinearType<?> fields)
            || fields.count() != 4
            || !(fields.nth(0) instanceof Keyword status)) {
          throw new HaraException("direct test result must be a native Result");
        }
        Object data = fields.nth(1);
        if (Keyword.create("success").equals(status) && Boolean.TRUE.equals(data)) passed++;
        else if ((Keyword.create("success").equals(status) && Boolean.FALSE.equals(data))
            || Keyword.create("error").equals(status)) failed++;
        else throw new HaraException("test Result must contain a boolean success value or error");
      }
      return new Result(
          path,
          failed == 0,
          passed + failed,
          passed + failed,
          passed,
          failed,
          0,
          0,
          transfer);
    }

    throw new HaraException(
        "test file must return a code.test/run summary or test result vector");
  }

  private static final Pattern PRINTED_SUMMARY =
      Pattern.compile(
          "^\\s*\\{:status\\s+:([a-z-]+).*?"
              + ":files\\s+\\d+\\s+:facts\\s+(\\d+)\\s+:checks\\s+(\\d+)"
              + "\\s+:passed\\s+(\\d+)\\s+:failed\\s+(\\d+)"
              + "\\s+:throw\\s+(\\d+)\\s+:timeout\\s+(\\d+)",
          Pattern.DOTALL);
  private static final Pattern TEST_NAMESPACE =
      Pattern.compile("(?m)^\\s*\\(ns\\s+([^\\s()]+)");

  /**
   * Extracts the stable code.test summary prefix when diagnostic values in :results are purposely
   * non-readable (for example #&lt;ThrownValue ...&gt;). The pass/fail decision and all reported counts
   * precede :results, so host reporting does not need to deserialize those guest diagnostics.
   */
  private static Result parsePrintedSummary(
      Path path, String summaryText, String transfer, RuntimeException unreadableDiagnostic) {
    Matcher matcher = PRINTED_SUMMARY.matcher(summaryText);
    if (!matcher.find()) {
      throw unreadableDiagnostic;
    }
    boolean passed = "passed".equals(matcher.group(1));
    int facts = Integer.parseInt(matcher.group(2));
    int checks = Integer.parseInt(matcher.group(3));
    int passedChecks = Integer.parseInt(matcher.group(4));
    int failedChecks = Integer.parseInt(matcher.group(5));
    int errors = Integer.parseInt(matcher.group(6));
    int timeouts = Integer.parseInt(matcher.group(7));
    return new Result(
        path,
        passed,
        facts,
        checks,
        passedChecks,
        failedChecks,
        errors,
        timeouts,
        transfer);
  }

  @SuppressWarnings({"rawtypes", "unchecked"})
  private static Object lookup(IMapType map, String key) {
    return map.lookup(Keyword.create(key));
  }

  @SuppressWarnings("rawtypes")
  private static int number(IMapType map, String key, int fallback) {
    Object value = lookup(map, key);
    return value instanceof Number number ? Math.toIntExact(number.longValue()) : fallback;
  }
}
