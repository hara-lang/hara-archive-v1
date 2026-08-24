package hara.truffle;

import hara.kernel.base.Parser;
import hara.lang.base.G;
import hara.lang.protocol.IMapType;
import java.io.IOException;
import java.io.PrintStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import org.graalvm.polyglot.Context;
import org.graalvm.polyglot.Value;

/** Portable HAL-backed specification commands for the Truffle CLI. */
final class HaraSpecTool {
  private static final String TEMPLATE =
      "{:document/id :example/metaspec :document/type :tool/metaspec "
          + ":document/version \"0.1.0\" :document/status :draft "
          + ":document/title \"Example Meta-Specification\" "
          + ":document/summary \"Describe the generated artifact contract.\" "
          + ":spec/conforms-to {:spec/id :tool/metaspec-metaspec :spec/version \"0.1.0\"} "
          + ":spec/artifact-kind :example/artifact "
          + ":meta/document-schema {:schema/id :example/document :schema/type :map} "
          + ":meta/schemas [] :meta/cross-references [] :meta/requirements [] "
          + ":metaspec/generation {:generation/input {} :generation/output {} "
          + ":generation/process [] :generation/acceptance {}}}";

  private HaraSpecTool() {}

  static int run(String[] arguments, PrintStream output, PrintStream error) {
    if (arguments.length == 0 || "--help".equals(arguments[0]) || "-h".equals(arguments[0])) {
      usage(output);
      return 0;
    }
    if ("template".equals(arguments[0])) {
      if (arguments.length != 1) {
        error.println("spec template accepts no file");
        return 2;
      }
      output.println(TEMPLATE);
      return 0;
    }
    if (!"lint".equals(arguments[0]) && !"verify".equals(arguments[0])) {
      error.println(
          "unavailable: spec "
              + arguments[0]
              + " is not yet implemented by the portable Truffle adapter");
      return 2;
    }
    if (arguments.length < 2) {
      error.println("spec " + arguments[0] + " requires FILE");
      return 2;
    }
    String format = format(arguments, error);
    if (format == null) return 2;
    Path path = Path.of(arguments[1]);
    try {
      String source = Files.readString(path, StandardCharsets.UTF_8);
      Object document = Parser.LispReader.readString(source, null);
      if (!(document instanceof IMapType<?, ?>)) {
        error.println(path + ": meta-spec root must be an EDN map");
        return 2;
      }
      String function = "lint".equals(arguments[0]) ? "lint-report" : "verify-report";
      try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
        context.eval(
            HaraLanguage.ID,
            "(ns tool.cli.spec-runner "
                + "(:require [tool.metaspec.core :as metaspec]))");
        Value value =
            context.eval(
                HaraLanguage.ID,
                "(metaspec/" + function + " " + G.display(document) + ")");
        String report = Main.display(value);
        boolean failed = report.contains(":report/status :fail");
        if ("edn".equals(format)) output.println(report);
        else
          output.println(
              "spec "
                  + arguments[0]
                  + ": "
                  + (failed ? "fail" : "pass")
                  + " "
                  + path);
        return failed ? 1 : 0;
      }
    } catch (IOException exception) {
      error.println("cannot read " + path + ": " + exception.getMessage());
      return 2;
    } catch (RuntimeException exception) {
      error.println(exception.getMessage());
      return 2;
    }
  }

  private static String format(String[] arguments, PrintStream error) {
    if (arguments.length == 2) return "text";
    if (arguments.length == 4
        && "--format".equals(arguments[2])
        && ("text".equals(arguments[3]) || "edn".equals(arguments[3]))) return arguments[3];
    error.println("spec format must be --format text or --format edn");
    return null;
  }

  private static void usage(PrintStream output) {
    output.println("hara spec lint FILE [--format text|edn]");
    output.println("hara spec verify FILE [--format text|edn]");
    output.println("hara spec template");
  }
}
