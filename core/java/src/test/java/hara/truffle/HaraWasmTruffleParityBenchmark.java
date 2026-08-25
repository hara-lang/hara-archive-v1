package hara.truffle;

import hara.kernel.base.Parser;
import hara.lang.data.Keyword;
import hara.lang.protocol.ILinearType;
import hara.lang.protocol.IMapType;
import hara.spec.SpecRegistry;
import java.io.BufferedWriter;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Collections;
import java.util.List;
import org.graalvm.polyglot.Context;

/** Emits per-case cold and warm timings for the Rust-WASM/Truffle parity corpus. */
public final class HaraWasmTruffleParityBenchmark {
  private static final Path CORPUS =
      SpecRegistry.resolve(
          "01-lang/001-language/draft/conformance/parity/wasm-truffle.edn");
  private static final Path ARTIFACT =
      Path.of("rust/raw/target/wasm32-unknown-unknown/browser-release/hara-wasm-vm.wasm");
  private static final String NAMESPACE = "test.rust.parity";
  private static final String REQUIRE = "(ns app (:require [test.rust.parity :as rust]))";

  private HaraWasmTruffleParityBenchmark() {}

  public static void main(String[] args) throws Exception {
    int coldSamples = args.length > 0 ? Integer.parseInt(args[0]) : 3;
    int warmup = args.length > 1 ? Integer.parseInt(args[1]) : 100;
    int iterations = args.length > 2 ? Integer.parseInt(args[2]) : 1000;
    Path output =
        args.length > 3 ? Path.of(args[3]) : Path.of("target/wasm-truffle-parity-speed.csv");
    int sliceIndex = args.length > 4 ? Integer.parseInt(args[4]) : 0;
    int sliceTotal = args.length > 5 ? Integer.parseInt(args[5]) : 1;
    if (coldSamples < 1 || warmup < 0 || iterations < 1) {
      throw new IllegalArgumentException(
          "expected positive cold samples/iterations and non-negative warmup");
    }
    if (sliceIndex < 0 || sliceIndex >= sliceTotal) {
      throw new IllegalArgumentException(
          "expected 0 <= slice index < slice total, got " + sliceIndex + " of " + sliceTotal);
    }
    if (!Files.isRegularFile(ARTIFACT)) throw new IllegalStateException("missing " + ARTIFACT);

    List<IMapType> cases = readCases();
    if (sliceTotal > 1) {
      // Round-robin keeps corpus order within a slice and balances the
      // curated/generated groups across slices.
      List<IMapType> sliced = new ArrayList<>();
      for (int i = 0; i < cases.size(); i++) {
        if (i % sliceTotal == sliceIndex) sliced.add(cases.get(i));
      }
      cases = sliced;
    }
    Path root = Files.createTempDirectory("hara-wasm-truffle-benchmark-");
    Path extension = root.resolve("test/rust/parity");
    Files.createDirectories(extension);
    HaraExtensionTestProject.write(
        extension,
        "{:namespace \"test.rust.parity\" :version \"0.2.0\" :provider :wasm "
            + ":module \"hara.wasm\" :abi :core.v1 "
            + ":exports {\"eval_i64\" {:args [:utf8] :returns :i64} "
            + "\"eval_error_code\" {:args [:utf8] :returns :i32}} :capabilities []}");
    Files.copy(ARTIFACT, extension.resolve("hara.wasm"));
    String previous = System.getProperty("hara.extensions.path");
    System.setProperty("hara.extensions.path", root.toString());
    Files.createDirectories(output.toAbsolutePath().getParent());
    try (BufferedWriter report = Files.newBufferedWriter(output)) {
      report.write(
          "id,group,origin,cold_truffle_ns,cold_wasm_ns,warm_truffle_ns_per_call,warm_wasm_ns_per_call,iterations,status");
      report.newLine();
      try (Context warmContext = Context.newBuilder(HaraLanguage.ID).build()) {
        warmContext.eval(HaraLanguage.ID, REQUIRE);
        for (IMapType testCase : cases) {
          String id = keywordText((Keyword) testCase.lookup(key("id")));
          String group = keywordText((Keyword) testCase.lookup(key("group")));
          String origin = keywordText((Keyword) testCase.lookup(key("origin")));
          String source = (String) testCase.lookup(key("source"));
          String setup = optionalString(testCase, "setup");
          String program = setup == null ? source : setup + "\n" + source;
          boolean error = isErrorCase((IMapType) testCase.lookup(key("expect")));

          long[] coldTruffleSamples = new long[coldSamples];
          long[] coldWasmSamples = new long[coldSamples];
          for (int sample = 0; sample < coldSamples; sample++) {
            coldTruffleSamples[sample] = coldTruffle(program, setup, source);
            coldWasmSamples[sample] = coldWasm(program, error);
          }
          safeWarmEval(warmContext, setup, source, program, error, warmup, false);
          long warmTruffleStart = System.nanoTime();
          for (int i = 0; i < iterations; i++)
            safeWarmEval(warmContext, setup, source, program, error, 1, false);
          long warmTruffle = (System.nanoTime() - warmTruffleStart) / iterations;
          safeWarmEval(warmContext, setup, source, program, error, warmup, true);
          long warmWasmStart = System.nanoTime();
          for (int i = 0; i < iterations; i++)
            safeWarmEval(warmContext, setup, source, program, error, 1, true);
          long warmWasm = (System.nanoTime() - warmWasmStart) / iterations;
          report.write(
              String.join(
                  ",",
                  csv(id),
                  csv(group),
                  csv(origin),
                  Long.toString(median(coldTruffleSamples)),
                  Long.toString(median(coldWasmSamples)),
                  Long.toString(warmTruffle),
                  Long.toString(warmWasm),
                  Integer.toString(iterations),
                  error ? "error" : "value"));
          report.newLine();
        }
      }
    } finally {
      if (previous == null) System.clearProperty("hara.extensions.path");
      else System.setProperty("hara.extensions.path", previous);
      Files.deleteIfExists(extension.resolve("hara.wasm"));
      Files.deleteIfExists(extension.resolve("project.edn"));
      Files.deleteIfExists(extension);
      Files.deleteIfExists(extension.getParent());
      Files.deleteIfExists(extension.getParent().getParent());
      Files.deleteIfExists(root);
    }
    System.out.printf(
        "wrote %d parity timings (slice %d of %d) to %s%n",
        cases.size(), sliceIndex, sliceTotal, output);
  }

  @SuppressWarnings({"rawtypes", "unchecked"})
  private static List<IMapType> readCases() throws Exception {
    IMapType manifest = (IMapType) Parser.LispReader.readString(Files.readString(CORPUS), null);
    ILinearType<?> raw = (ILinearType<?>) manifest.lookup(key("cases"));
    List<IMapType> cases = new ArrayList<>();
    for (Object item : raw) cases.add((IMapType) item);
    return cases;
  }

  private static long coldTruffle(String program, String setup, String source) {
    long start = System.nanoTime();
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      evaluateTruffle(context, setup, source);
    } catch (Throwable ignored) {
      // Error cases are timed through the same path and recorded as status=error.
    }
    return System.nanoTime() - start;
  }

  private static long coldWasm(String program, boolean error) {
    long start = System.nanoTime();
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      context.eval(HaraLanguage.ID, REQUIRE);
      evaluateWasm(context, program, error);
    } catch (Throwable ignored) {
      // Error cases return a stable error code and are intentionally still timed.
    }
    return System.nanoTime() - start;
  }

  private static void warmEval(
      Context context,
      String setup,
      String source,
      String program,
      boolean error,
      int count,
      boolean wasm) {
    for (int i = 0; i < count; i++) {
      if (wasm) evaluateWasm(context, program, error);
      else evaluateTruffle(context, setup, source);
    }
  }

  private static void safeWarmEval(
      Context context,
      String setup,
      String source,
      String program,
      boolean error,
      int count,
      boolean wasm) {
    try {
      warmEval(context, setup, source, program, error, count, wasm);
    } catch (Throwable failure) {
      if (!error) throw new RuntimeException(failure);
    }
  }

  private static void evaluateTruffle(Context context, String setup, String source) {
    if (setup != null) context.eval(HaraLanguage.ID, setup);
    context.eval(HaraLanguage.ID, source);
  }

  private static void evaluateWasm(Context context, String program, boolean error) {
    context.eval(
        HaraLanguage.ID,
        "(rust/" + (error ? "eval_error_code" : "eval_i64") + " " + quote(program) + ")");
  }

  @SuppressWarnings("rawtypes")
  private static boolean isErrorCase(IMapType expect) {
    return expect.lookup(key("error")) != null;
  }

  @SuppressWarnings("rawtypes")
  private static String optionalString(IMapType map, String name) {
    Object value = map.lookup(key(name));
    return value == null ? null : (String) value;
  }

  private static Keyword key(String name) {
    return Keyword.create(null, name);
  }

  private static String keywordText(Keyword keyword) {
    return keyword.getNamespace() == null
        ? keyword.getName()
        : keyword.getNamespace() + "/" + keyword.getName();
  }

  private static String quote(String source) {
    return "\"" + source.replace("\\", "\\\\").replace("\"", "\\\"") + "\"";
  }

  private static String csv(String value) {
    return "\"" + value.replace("\"", "\"\"") + "\"";
  }

  private static long median(long[] values) {
    List<Long> sorted = new ArrayList<>();
    for (long value : values) sorted.add(value);
    Collections.sort(sorted);
    return sorted.get(sorted.size() / 2);
  }
}
