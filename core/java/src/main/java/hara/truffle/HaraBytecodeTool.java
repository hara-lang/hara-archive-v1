package hara.truffle;

import hara.truffle.bytecode.HbcCodec;
import hara.truffle.bytecode.HbcConformanceCorpus;
import hara.truffle.bytecode.HbcDisassembler;
import hara.truffle.bytecode.HbxBundleCodec;
import java.io.IOException;
import java.io.PrintStream;
import java.nio.file.Files;
import java.nio.file.Path;
import org.graalvm.polyglot.Context;
import org.graalvm.polyglot.Engine;
import org.graalvm.polyglot.Source;
import org.graalvm.polyglot.Value;
import org.graalvm.polyglot.io.ByteSequence;

final class HaraBytecodeTool {
  private HaraBytecodeTool() {}

  static int run(String[] arguments, PrintStream output, PrintStream error) {
    if (arguments.length != 2
        || !("run".equals(arguments[0])
            || "disassemble".equals(arguments[0])
            || "conformance".equals(arguments[0]))) {
      error.println(
          "usage: hara bytecode <run|disassemble> FILE.hbc|FILE.hbx\n"
              + "       hara bytecode conformance FILE.hcc");
      return 2;
    }
    try {
      byte[] artifact = Files.readAllBytes(Path.of(arguments[1]));
      if ("conformance".equals(arguments[0])) {
        return runConformance(artifact, output);
      }
      if (artifact.length >= 4
          && artifact[0] == 'H'
          && artifact[1] == 'B'
          && artifact[2] == 'X'
          && artifact[3] == '0') {
        return runBundle(artifact, arguments[0], output);
      }
      if ("disassemble".equals(arguments[0])) {
        output.print(HbcDisassembler.disassemble(HbcCodec.decode(artifact)));
        return 0;
      }
      Source source =
          Source.newBuilder(HaraLanguage.ID, ByteSequence.create(artifact), arguments[1])
              .mimeType(HaraLanguage.BYTECODE_MIME_TYPE)
              .build();
      try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
        Value result = context.eval(source);
        output.println(result.isNull() ? "nil" : result.toString());
      }
      return 0;
    } catch (IOException exception) {
      error.println(exception.getMessage());
      return 2;
    } catch (RuntimeException exception) {
      error.println(exception.getMessage());
      return 1;
    }
  }

  private static int runConformance(byte[] corpus, PrintStream output) throws IOException {
    java.util.List<HbcConformanceCorpus.Case> cases = HbcConformanceCorpus.decode(corpus);
    try (Engine engine =
        Engine.newBuilder().option("engine.WarnInterpreterOnly", "false").build()) {
      for (HbcConformanceCorpus.Case testCase : cases) {
        Source source =
            Source.newBuilder(
                    HaraLanguage.ID,
                    ByteSequence.create(testCase.artifact()),
                    testCase.id() + ".hbc")
                .mimeType(HaraLanguage.BYTECODE_MIME_TYPE)
                .build();
        // Each case gets isolated language state while sharing the immutable
        // engine/code cache.  This is both stricter than one shared namespace
        // and practical for native-image CI's complete opcode corpus.
        try (Context context = Context.newBuilder(HaraLanguage.ID).engine(engine).build()) {
          Value actual = context.eval(source);
          String display = HbcConformanceCorpus.display(actual);
          if (!testCase.expectedDisplay().equals(display)) {
            throw new HaraException(
                "HBC0 conformance failed for :"
                    + testCase.id()
                    + ": expected "
                    + testCase.expectedDisplay()
                    + ", got "
                    + display);
          }
        }
      }
    }
    output.println("HBC0 conformance passed: " + cases.size() + " cases");
    return 0;
  }

  private static int runBundle(byte[] bundle, String command, PrintStream output) throws IOException {
    java.util.List<HbxBundleCodec.Module> modules = HbxBundleCodec.decode(bundle);
    if ("disassemble".equals(command)) {
      for (HbxBundleCodec.Module module : modules) {
        output.println("module " + module.resource());
        output.print(HbcDisassembler.disassemble(HbcCodec.decode(module.artifact())));
      }
      return 0;
    }
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      Value result = null;
      for (HbxBundleCodec.Module module : modules) {
        if (!module.eager()) continue;
        context.eval(HaraLanguage.ID, module.namespaceForm());
        Source source =
            Source.newBuilder(
                    HaraLanguage.ID,
                    ByteSequence.create(module.artifact()),
                    module.resource() + ".hbc")
                .mimeType(HaraLanguage.BYTECODE_MIME_TYPE)
                .build();
        result = context.eval(source);
      }
      output.println(result == null || result.isNull() ? "nil" : result.toString());
    }
    return 0;
  }
}
