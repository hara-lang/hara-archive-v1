package hara.truffle;

import java.io.BufferedReader;
import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.InputStreamReader;
import java.io.PrintStream;
import java.net.Socket;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import hara.kernel.ReplConfig;
import hara.kernel.base.Parser;
import hara.lang.base.G;
import hara.lang.data.Keyword;
import hara.lang.protocol.ILinearType;
import hara.lang.protocol.IMapType;
import java.math.BigInteger;
import java.util.ArrayList;
import java.util.Collections;
import java.util.List;
import java.util.Iterator;
import java.util.Comparator;
import org.jline.reader.Candidate;
import org.jline.reader.Completer;
import org.jline.reader.EndOfFileException;
import org.jline.reader.History;
import org.jline.reader.Highlighter;
import org.jline.reader.LineReader;
import org.jline.reader.LineReaderBuilder;
import org.jline.reader.ParsedLine;
import org.jline.reader.UserInterruptException;
import org.jline.reader.impl.DefaultParser;
import org.jline.terminal.Terminal;
import org.jline.terminal.TerminalBuilder;
import org.jline.utils.InfoCmp;
import org.jline.utils.AttributedString;
import org.jline.widget.AutosuggestionWidgets;
import org.graalvm.polyglot.Context;
import org.graalvm.polyglot.HostAccess;
import org.graalvm.polyglot.PolyglotException;
import org.graalvm.polyglot.Value;
import org.graalvm.polyglot.io.IOAccess;
import hara.kernel.resp.RespConnection;

public final class Main {
  private Main() {}

  public static void main(String[] args) {
    ByteArrayOutputStream errorBytes = new ByteArrayOutputStream();
    PrintStream error = new PrintStream(errorBytes, true, StandardCharsets.UTF_8);
    int status = run(args, System.in, System.out, error);
    if (errorBytes.size() > 0)
      System.err.print("hara: " + errorBytes.toString(StandardCharsets.UTF_8));
    if (status != 0) {
      System.exit(status);
    }
  }

  static int run(String[] args, PrintStream output, PrintStream error) {
    return run(args, System.in, output, error);
  }

  static int run(String[] args, InputStream input, PrintStream output, PrintStream error) {
    if (args.length == 1 && ("--version".equals(args[0]) || "-V".equals(args[0]))) {
      output.println("hara truffle 0.1.0");
      return 0;
    }
    if (args.length == 1 && ("--help".equals(args[0]) || "-h".equals(args[0]))) {
      printUsage(output);
      return 0;
    }
    if (args.length > 0 && "compile-halc".equals(args[0])) {
      return HalcCompiler.run(
          java.util.Arrays.copyOfRange(args, 1, args.length), output, error);
    }
    if (args.length > 0 && "foundation-halc-benchmark".equals(args[0])) {
      return FoundationHalcBenchmark.run(
          java.util.Arrays.copyOfRange(args, 1, args.length), output, error);
    }
    if (args.length > 0 && "bytecode".equals(args[0])) {
      return HaraBytecodeTool.run(
          java.util.Arrays.copyOfRange(args, 1, args.length), output, error);
    }
    Capabilities capabilities;
    try {
      capabilities = parseCapabilities(args);
    } catch (IllegalArgumentException exception) {
      error.println(exception.getMessage());
      return 2;
    }
    args = capabilities.arguments;
    args = HaraCliRouter.instance().normalize(args);
    if (args.length == 0) {
      return runRepl(
          input,
          output,
          error,
          input == System.in && System.console() != null,
          capabilities,
          !capabilities.offline);
    }

    if ("help".equals(args[0])
        || "--help".equals(args[0])
        || java.util.Arrays.stream(args).skip(1).anyMatch(value -> "--help".equals(value) || "-h".equals(value))) {
      printUsage(output);
      return 0;
    }

    if ("repl".equals(args[0])) {
      return runRepl(
          input,
          output,
          error,
          input == System.in && System.console() != null,
          capabilities,
          !capabilities.offline);
    }

    if ("standalone".equalsIgnoreCase(args[0])) {
      return runRepl(
          input,
          output,
          error,
          input == System.in && System.console() != null,
          capabilities,
          false);
    }

    if ("server".equalsIgnoreCase(args[0]) || "headless".equalsIgnoreCase(args[0])) {
      return runServer(output, error, capabilities);
    }
    if ("remote".equalsIgnoreCase(args[0])) {
      if (args.length < 2) {
        error.println("remote requires HOST:PORT");
        return 2;
      }
      return runRemote(args[1], input, output, error);
    }
    if ("conformance".equals(args[0])) {
      return runConformance(output, error);
    }
    if ("benchmark".equals(args[0])) {
      return HaraRuntimeBenchmark.run(
          java.util.Arrays.copyOfRange(args, 1, args.length), output, error);
    }
    if ("spec".equals(args[0])) {
      return HaraSpecTool.run(
          java.util.Arrays.copyOfRange(args, 1, args.length), output, error);
    }
    if ("package".equals(args[0])) {
      return HaraPackageTool.run(
          java.util.Arrays.copyOfRange(args, 1, args.length), output, error);
    }
    if ("asset".equals(args[0])) {
      return HaraAssetTool.run(
          java.util.Arrays.copyOfRange(args, 1, args.length), output, error);
    }
    if ("id".equals(args[0])) {
      return HaraIdentityTool.run(
          java.util.Arrays.copyOfRange(args, 1, args.length), output, error);
    }
    if ("tap".equals(args[0])) {
      return HaraTapTool.run(
          java.util.Arrays.copyOfRange(args, 1, args.length), output, error);
    }

    try {
      if ("new".equals(args[0])) return newProject(args, output);
      if ("check".equals(args[0])) return checkProject(args, capabilities, output);
      if ("add".equals(args[0])) return editDependency(args, capabilities, output, true);
      if ("remove".equals(args[0])) return editDependency(args, capabilities, output, false);
      if ("sync".equals(args[0])) return syncProject(args, capabilities, output);
      if ("update".equals(args[0])) throw new HaraException("project update requires the reviewed registry client");
      if ("test".equals(args[0])) return testProject(args, capabilities, output);
      if ("run".equals(args[0]) && args.length == 1) return runProject(capabilities, output);
      String source;
      switch (args[0]) {
        case "eval":
          if (args.length < 2) {
            error.println("eval requires a Hara expression");
            return 2;
          }
          source = args[1];
          break;
        case "run":
          if (args.length < 2) {
            error.println("run requires a file path");
            return 2;
          }
          source = Files.readString(Path.of(args[1]), StandardCharsets.UTF_8);
          break;
        case "stdin":
          source = new String(input.readAllBytes(), StandardCharsets.UTF_8);
          break;
        default:
          error.println("Unknown command: " + args[0]);
          printUsage(error);
          return 2;
      }

      HaraProject project = capabilities.project == null ? null : runtimeProject(capabilities);
      try (HaraJvmProject ignored =
              project == null ? null : HaraJvmProject.prepare(project, capabilities.offline);
          Context context = context(capabilities, project)) {
        Value result = context.eval(HaraLanguage.ID, source);
        output.println(display(result));
      }
      return 0;
    } catch (PolyglotException exception) {
      error.println(exception.getMessage());
      return 1;
    } catch (IOException exception) {
      error.println(exception.getMessage());
      return 2;
    } catch (RuntimeException exception) {
      error.println(exception.getMessage());
      return 1;
    }
  }

  private static HaraProject cliProject(String[] args, Capabilities capabilities) {
    Path start =
        capabilities.project != null
            ? capabilities.project
            : args.length > 1 ? Path.of(args[1]) : Path.of(".");
    HaraProject project = HaraProject.discover(start);
    if (project == null) throw new HaraException("no project.edn found above " + start);
    project.validateCliProject();
    return project;
  }

  private static int newProject(String[] args, PrintStream output) throws IOException {
    if (args.length != 2) throw new HaraException("new requires a project name");
    String name = args[1];
    if (!name.matches("[a-z0-9-]+"))
      throw new HaraException("project name must contain only lowercase letters, numbers, and hyphens");
    Path root = Path.of(name);
    if (Files.exists(root)) throw new HaraException("destination already exists: " + root);
    String namespace = name.replace('-', '_');
    Files.createDirectories(root.resolve("src").resolve(namespace));
    Files.createDirectories(root.resolve("test").resolve(namespace));
    Files.createDirectories(root.resolve("extensions"));
    Files.createDirectories(root.resolve("src-java"));
    Files.writeString(
        root.resolve("project.edn"),
        "{:hara/type :project\n :hara/version \"1.0.0\"\n :project/id " + name
            + "\n :project/version \"0.1.0\"\n :project/source-paths [\"src\"]\n"
            + " :project/test-paths [\"test\"]\n :project/extension-paths [\"extensions\"]\n"
            + " :project/main " + namespace + ".main\n :project/capabilities #{}\n :project/dependencies {}\n"
            + " :jvm/dependencies []\n :jvm/source-paths [\"src-java\"]\n :jvm/target-path \"target/classes\"}\n");
    Files.writeString(root.resolve("workspace.edn"), "{:hara/type :workspace :hara/version \"1.0.0\"}\n");
    Files.writeString(root.resolve("src").resolve(namespace).resolve("main.hal"),
        "(ns " + namespace + ".main)\n\n(defn main []\n  \"Hello from " + name + "\")\n\n(main)\n");
    Files.writeString(root.resolve("test").resolve(namespace).resolve("main_test.hal"),
        "(ns " + namespace + ".main-test)\n\n"
            + "[(test-check \"starter project runs\" true true)]\n");
    output.println("created " + name);
    return 0;
  }

  private static int checkProject(String[] args, Capabilities capabilities, PrintStream output) {
    if (args.length > 2) throw new HaraException("check accepts at most one path");
    HaraProject project = cliProject(args, capabilities);
    output.println("project check: " + project.name().display() + " " + project.version());
    return 0;
  }

  private static int editDependency(String[] args, Capabilities capabilities, PrintStream output, boolean add)
      throws IOException {
    if (args.length != 2) throw new HaraException(add ? "add requires COORDINATE@RANGE" : "remove requires COORDINATE");
    String coordinate = args[1];
    String version = null;
    if (add) {
      int at = coordinate.lastIndexOf('@');
      if (at <= 0 || at == coordinate.length() - 1) throw new HaraException("add requires COORDINATE@RANGE");
      version = coordinate.substring(at + 1);
      coordinate = coordinate.substring(0, at);
      if (!version.matches("[0-9A-Za-z^~<>=.*+| -]+")) throw new HaraException("invalid dependency range " + version);
    }
    if (!coordinate.matches("[a-z0-9_.-]+/[a-z0-9_.-]+")) throw new HaraException("invalid package coordinate: " + coordinate);
    HaraProject project = cliProject(new String[] {add ? "add" : "remove"}, capabilities);
    rewriteDependency(project.descriptor(), coordinate, version);
    output.println((add ? "added " : "removed ") + coordinate);
    return 0;
  }

  private static void rewriteDependency(Path manifest, String coordinate, String version) throws IOException {
    String source = Files.readString(manifest);
    int key = source.indexOf(":project/dependencies");
    if (key < 0) throw new HaraException("project.edn is missing :project/dependencies");
    int open = source.indexOf('{', key);
    int close = matchingBrace(source, open);
    if (open < 0 || close < 0) throw new HaraException("project.edn :project/dependencies must be an EDN map");
    String body = source.substring(open + 1, close).trim();
    String entry = "\"" + coordinate + "\" {:version \"" + version + "\"}";
    String rewritten;
    if (version == null) {
      rewritten = body.replaceAll("\\s*\\\"" + java.util.regex.Pattern.quote(coordinate) + "\\\"\\s*\\{\\s*:version\\s*\\\"[^\\\"]+\\\"\\s*}\\s*", "").trim();
    } else if (body.isEmpty()) {
      rewritten = entry;
    } else if (body.matches("(?s).*\\\"" + java.util.regex.Pattern.quote(coordinate) + "\\\"\\s*\\{\\s*:version\\s*\\\"[^\\\"]+\\\"\\s*}.*")) {
      rewritten = body.replaceAll("\\\"" + java.util.regex.Pattern.quote(coordinate) + "\\\"\\s*\\{\\s*:version\\s*\\\"[^\\\"]+\\\"\\s*}", entry);
    } else {
      rewritten = body + " " + entry;
    }
    Files.writeString(manifest, source.substring(0, open + 1) + rewritten + source.substring(close));
  }

  private static int matchingBrace(String source, int open) {
    int depth = 0;
    boolean string = false;
    boolean escape = false;
    for (int i = open; i < source.length(); i++) {
      char ch = source.charAt(i);
      if (string) {
        if (escape) escape = false;
        else if (ch == '\\') escape = true;
        else if (ch == '"') string = false;
      } else if (ch == '"') string = true;
      else if (ch == '{') depth++;
      else if (ch == '}' && --depth == 0) return i;
    }
    return -1;
  }

  private static int syncProject(String[] args, Capabilities capabilities, PrintStream output)
      throws IOException {
    if (args.length > 2 || (args.length == 2 && !java.util.Set.of("--offline", "--locked", "--frozen").contains(args[1])))
      throw new HaraException("sync accepts at most one of --offline, --locked, or --frozen");
    HaraProject project = cliProject(new String[] {"sync"}, capabilities);
    String manifest = Files.readString(project.descriptor());
    int dependencyKey = manifest.indexOf(":project/dependencies");
    int dependencyOpen = dependencyKey < 0 ? -1 : manifest.indexOf('{', dependencyKey);
    int dependencyClose = dependencyOpen < 0 ? -1 : matchingBrace(manifest, dependencyOpen);
    if (dependencyOpen < 0 || dependencyClose < 0)
      throw new HaraException("project.edn :project/dependencies must be an EDN map");
    String dependencies = manifest.substring(dependencyOpen + 1, dependencyClose).trim();
    if (!dependencies.isEmpty()) {
      java.util.regex.Matcher coordinates =
          java.util.regex.Pattern.compile("\\\"[^\\\"]+\\\"\\s*\\{").matcher(dependencies);
      int count = 0;
      while (coordinates.find()) count++;
      throw new HaraException("project sync requires the reviewed registry client to resolve " + count + " declared dependencies");
    }
    Path lock = project.root().resolve("project.lock.edn");
    boolean locked = args.length == 2 && ("--locked".equals(args[1]) || "--frozen".equals(args[1]));
    if (locked && !Files.isRegularFile(lock)) throw new HaraException(args[1] + " requires an existing project.lock.edn");
    if (locked && !"{:lock/format \"0.0.0-alpha\" :packages {}}\n".equals(Files.readString(lock)))
      throw new HaraException(lock + " is not a lockfile written by this CLI");
    boolean offline =
        capabilities.offline
            || (args.length == 2
                && ("--offline".equals(args[1]) || "--frozen".equals(args[1])));
    java.util.List<Path> artifacts = HaraJvmProject.resolveDependencies(project, offline);
    if (!locked) Files.writeString(lock, "{:lock/format \"0.0.0-alpha\" :packages {}}\n");
    output.println("project sync: " + lock);
    if (!project.jvmDependencies().isEmpty()) {
      output.println(
          "jvm dependencies: "
              + project.jvmDependencies().size()
              + " direct, "
              + artifacts.size()
              + " artifacts");
    }
    return 0;
  }

  private static int runProject(Capabilities capabilities, PrintStream output) throws IOException {
    HaraProject project = cliProject(new String[] {"run"}, capabilities);
    try (HaraJvmProject ignored = HaraJvmProject.prepare(project, capabilities.offline);
        Context context = context(capabilities, project)) {
      output.println(display(context.eval(HaraLanguage.ID, Files.readString(project.mainFile()))));
    }
    return 0;
  }

  @SuppressWarnings("rawtypes")
  private static int testProject(String[] args, Capabilities capabilities, PrintStream output) throws IOException {
    if (args.length > 2) throw new HaraException("test accepts at most one path");
    HaraProject project = cliProject(args, capabilities);
    ArrayList<Path> files = new ArrayList<>();
    Path requested =
        args.length == 2 ? Path.of(args[1]).toAbsolutePath().normalize() : null;
    if (requested != null && Files.isRegularFile(requested)) {
      if (!requested.toString().endsWith(".hal"))
        throw new HaraException("test file must use the .hal extension");
      files.add(requested);
    } else if (requested != null && Files.isDirectory(requested)) {
      try (java.util.stream.Stream<Path> paths = Files.walk(requested)) {
        paths.filter(path -> path.toString().endsWith(".hal")).forEach(files::add);
      }
    } else if (requested != null) {
      throw new HaraException("test path does not exist: " + requested);
    } else {
      for (Path root : project.testPaths()) {
        if (Files.exists(root)) try (java.util.stream.Stream<Path> paths = Files.walk(root)) {
          paths.filter(path -> path.toString().endsWith(".hal")).forEach(files::add);
        }
      }
    }
    files.sort(Comparator.naturalOrder());
    if (files.isEmpty()) throw new HaraException("project has no .hal files under :project/test-paths");
    int passed = 0;
    int failed = 0;
    try (HaraJvmProject ignored = HaraJvmProject.prepare(project, capabilities.offline)) {
      for (Path file : files) {
        try (Context context = context(capabilities, project)) {
          Value value = context.eval(HaraLanguage.ID, Files.readString(file));
          HaraNativeTestRunner.Result result = HaraNativeTestRunner.parseResult(file, value);
          int filePassed = result.passedChecks();
          int fileFailed = result.failedChecks() + result.errors() + result.timeouts();
          if (!result.passed() && fileFailed == 0) fileFailed = 1;
          passed += filePassed;
          failed += fileFailed;
          output.println("test " + file + ": " + filePassed + " passed, " + fileFailed + " failed");
        } catch (RuntimeException failure) {
          failed++;
          output.println("test " + file + ": " + failure.getMessage());
        }
      }
    }
    output.println("test result: " + passed + " passed, " + failed + " failed");
    if (failed > 0) throw new HaraException("test failures");
    return 0;
  }

  private static int runServer(PrintStream output, PrintStream error, Capabilities capabilities) {
    if (capabilities.offline) {
      error.println("--offline cannot be used with headless");
      return 2;
    }
    try (SessionKernel kernel =
            new SessionKernel(
            capabilities.file, capabilities.network, capabilities.process);
        HaraServer server =
            new HaraServer(
                kernel,
                capabilities.host,
                capabilities.port,
                capabilities.logRequests,
                currentProjectRoot())) {
      server.start();
      output.println("HARA RESP " + capabilities.host + ":" + server.port() + " · session ROOT");
      output.flush();
      while (server.isRunning()) {
        try {
          Thread.sleep(1000L);
        } catch (InterruptedException interrupted) {
          Thread.currentThread().interrupt();
          return 0;
        }
      }
      return 0;
    } catch (IOException exception) {
      error.println(exception.getMessage());
      return 1;
    }
  }

  private static int runRemote(
      String endpoint, InputStream input, PrintStream output, PrintStream error) {
    int separator = endpoint.lastIndexOf(':');
    if (separator <= 0 || separator == endpoint.length() - 1) {
      error.println("remote expects HOST:PORT");
      return 2;
    }
    String host = endpoint.substring(0, separator);
    try (Socket socket = new Socket(host, Integer.parseInt(endpoint.substring(separator + 1)))) {
      RespConnection conn = new RespConnection(socket);
      conn.write("HELLO", "4", "CLIENT", "HARA-REMOTE");
      output.println(remoteText(conn.read()));
      java.io.BufferedReader reader =
          new java.io.BufferedReader(new InputStreamReader(input, StandardCharsets.UTF_8));
      String line;
      long request = 0;
      while ((line = reader.readLine()) != null) {
        String command = line.strip();
        if (command.isEmpty()) continue;
        if ("/quit".equals(command) || ":quit".equals(command)) {
          conn.write("QUIT");
          output.println(remoteText(conn.read()));
          return 0;
        }
        if (command.startsWith("/session use ")) {
          conn.write("SESSION", "ATTACH", command.substring(13).strip());
          output.println(remoteText(conn.read()));
          continue;
        }
        if (command.startsWith("/session ")) {
          String[] parts = command.substring(9).strip().split("\\s+");
          List<Object> requestArgs = new ArrayList<>();
          requestArgs.add("SESSION");
          for (String part : parts) requestArgs.add(part);
          conn.write(requestArgs);
          output.println(remoteText(conn.read()));
          continue;
        }
        String id = "REMOTE-" + (++request);
        conn.write("EVAL", id, command);
        output.println(remoteText(conn.read()));
        output.println(remoteText(conn.read()));
      }
      return 0;
    } catch (Exception exception) {
      error.println(exception.getMessage());
      return 1;
    }
  }

  private static String remoteText(Object value) {
    if (value instanceof byte[]) return new String((byte[]) value, StandardCharsets.UTF_8);
    if (value instanceof List<?>) {
      StringBuilder result = new StringBuilder("[");
      boolean first = true;
      for (Object item : (List<?>) value) {
        if (!first) result.append(' ');
        first = false;
        result.append(remoteText(item));
      }
      return result.append(']').toString();
    }
    return String.valueOf(value);
  }

  private static Path currentProjectRoot() {
    HaraProject project = HaraProject.discover(Path.of("."));
    return project == null ? null : project.root();
  }

  private static HaraProject runtimeProject(Capabilities capabilities) {
    Path start = capabilities.project == null ? Path.of(".") : capabilities.project;
    HaraProject project = HaraProject.discover(start);
    if (project != null
        && !"project.edn".equals(project.descriptor().getFileName().toString())) return null;
    if (project != null) project.validateCliProject();
    return project;
  }

  @SuppressWarnings({"rawtypes", "unchecked"})
  private static int runConformance(PrintStream output, PrintStream error) {
    try (InputStream resource =
        Main.class
            .getClassLoader()
            .getResourceAsStream("01-lang/001-language/draft/conformance/core.edn")) {
      if (resource == null) {
        error.println("Missing packaged Core-language conformance manifest");
        return 1;
      }
      String source = new String(resource.readAllBytes(), StandardCharsets.UTF_8);
      IMapType manifest = (IMapType) Parser.LispReader.readString(source, null);
      ILinearType<?> cases = (ILinearType<?>) manifest.lookup(Keyword.create("cases"));
      int completed = 0;
      for (Object item : cases) {
        IMapType testCase = (IMapType) item;
        String id = ((Keyword) testCase.lookup(Keyword.create("id"))).getName();
        String form = (String) testCase.lookup(Keyword.create("source"));
        String className = ((Keyword) testCase.lookup(Keyword.create("class"))).getName();
        IMapType expected = (IMapType) testCase.lookup(Keyword.create("expect"));
        if ("reader".equals(className)) {
          Object expectedError = expected.lookup(Keyword.create("error"));
          Object expectedMessage = expected.lookup(Keyword.create("message"));
          try {
            Object actual = Parser.LispReader.readString(form, null);
            if (expectedError != null) {
              throw new IllegalStateException(id + " expected a reader error");
            }
            Object expectedForm = expected.lookup(Keyword.create("form"));
            if (expectedForm != null && !expectedForm.toString().equals(G.display(actual))) {
              throw new IllegalStateException(
                  id + " expected " + expectedForm + ", got " + G.display(actual));
            }
          } catch (RuntimeException readerError) {
            if (expectedError == null) throw readerError;
            Throwable readerCause =
                readerError.getCause() == null ? readerError : readerError.getCause();
            if (expectedMessage != null
                && !readerCause.getMessage().contains(expectedMessage.toString())) {
              throw new IllegalStateException(
                  id + " reader error mismatch: expected message containing " + expectedMessage);
            }
          }
        } else {
          String setup = (String) testCase.lookup(Keyword.create("setup"));
          Object expectedError = expected.lookup(Keyword.create("error"));
          Object expectedMessage = expected.lookup(Keyword.create("message"));
          try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
            if (setup != null) context.eval(HaraLanguage.ID, setup);
            Value actual = context.eval(HaraLanguage.ID, form);
            if (expectedError != null) {
              throw new IllegalStateException(id + " expected an error");
            }
            assertConformanceValue(id, actual, expected.lookup(Keyword.create("value")));
          } catch (PolyglotException guestError) {
            if (expectedError == null) throw guestError;
            if (expectedMessage != null
                && !guestError.getMessage().contains(expectedMessage.toString())) {
              throw new IllegalStateException(
                  id + " error mismatch: expected message containing " + expectedMessage);
            }
          }
        }
        completed++;
      }
      output.println("Core-language conformance passed: " + completed + " cases");
      return 0;
    } catch (Exception failure) {
      error.println("Core-language conformance failed: " + failure.getMessage());
      return 1;
    }
  }

  private static void assertConformanceValue(String id, Value actual, Object expected) {
    if (expected == null) {
      if (!actual.isNull()) throw new IllegalStateException(id + " expected nil");
    } else if (expected instanceof Boolean) {
      if (actual.asBoolean() != (Boolean) expected)
        throw new IllegalStateException(id + " boolean mismatch");
    } else if (expected instanceof String) {
      if (!actual.asString().equals(expected))
        throw new IllegalStateException(id + " string mismatch");
    } else if (expected instanceof BigInteger) {
      if (!actual.as(BigInteger.class).equals(expected))
        throw new IllegalStateException(id + " bigint mismatch");
    } else if (expected instanceof Keyword) {
      if (!actual.toString().equals(G.display(expected)))
        throw new IllegalStateException(id + " keyword mismatch");
    } else if (expected instanceof Number) {
      if (actual.asLong() != ((Number) expected).longValue())
        throw new IllegalStateException(id + " number mismatch");
    } else {
      throw new IllegalStateException(id + " has unsupported expected value " + expected);
    }
  }

  private static int runRepl(
      InputStream input,
      PrintStream output,
      PrintStream error,
      boolean interactive,
      Capabilities capabilities,
      boolean enableResp) {
    HaraProject project;
    try {
      project = runtimeProject(capabilities);
    } catch (RuntimeException errorValue) {
      error.println(errorValue.getMessage());
      return 1;
    }
    try (HaraJvmProject ignored =
            project == null ? null : HaraJvmProject.prepare(project, capabilities.offline);
        SessionKernel kernel =
            new SessionKernel(
                capabilities.file,
                capabilities.network,
                capabilities.process,
                project)) {
      RespController resp =
          new RespController(
              kernel, capabilities.host, capabilities.port, capabilities.logRequests);
      if (enableResp) {
        try {
          resp.start();
        } catch (IOException exception) {
          error.println("Unable to start RESP listener: " + exception.getMessage());
          return 1;
        }
      }
      try {
        if (interactive)
          return runJLineRepl(kernel.root(), resp, output, error, capabilities);
        try (BufferedReader reader =
            new BufferedReader(new InputStreamReader(input, StandardCharsets.UTF_8))) {
          StringBuilder source = new StringBuilder();
          List<String> history = new ArrayList<>();
          String line;
          while ((line = reader.readLine()) != null) {
            if (source.length() == 0 && ":quit".equals(line.trim())) return 0;
            if (source.length() == 0 && ":help".equals(line.trim())) {
              output.println(":quit          exit the REPL");
              output.println(":help          show this help");
              output.println(":history       show evaluated forms");
              continue;
            }
            if (source.length() == 0 && ":history".equals(line.trim())) {
              for (int i = 0; i < history.size(); i++)
                output.println((i + 1) + ": " + history.get(i));
              continue;
            }
            source.append(line).append('\n');
            if (!isComplete(source)) continue;
            history.add(source.toString().stripTrailing());
            try {
              output.println("=> " + display(kernel.root().eval(source.toString())));
            } catch (RuntimeException exception) {
              error.println(exception.getMessage());
            }
            source.setLength(0);
          }
          if (source.length() != 0) {
            error.println("Incomplete source");
            return 1;
          }
          return 0;
        }
      } finally {
        resp.close();
      }
    } catch (IOException exception) {
      error.println(exception.getMessage());
      return 1;
    } catch (RuntimeException exception) {
      error.println(exception.getMessage());
      return 1;
    }
  }

  private static Context context(Capabilities capabilities) {
    return context(capabilities, null, null);
  }

  private static Context context(Capabilities capabilities, HaraProject project) {
    return context(capabilities, project == null ? null : project.root(), project);
  }

  private static Context context(
      Capabilities capabilities, Path projectRoot, HaraProject project) {
    IOAccess access =
        IOAccess.newBuilder()
            .allowHostFileAccess(capabilities.file || projectRoot != null)
            .allowHostSocketAccess(capabilities.network)
            .build();
    Context.Builder builder = Context.newBuilder(HaraLanguage.ID)
        .allowCreateProcess(capabilities.process)
        .allowIO(access);
    if (project != null && project.hasCapability("jvm/reflection")) {
      builder.allowHostAccess(HostAccess.ALL).allowHostClassLookup(name -> true);
    }
    if (projectRoot != null) builder.currentWorkingDirectory(projectRoot);
    return builder.build();
  }

  private static Capabilities parseCapabilities(String[] arguments) {
    ArrayList<String> positional = new ArrayList<>();
    boolean file = false;
    boolean network = false;
    boolean process = false;
    boolean offline = false;
    boolean logRequests = false;
    boolean noHistory = false;
    boolean noSplash = false;
    boolean noColor = false;
    Path historyFile = null;
    Path project = null;
    String host = HaraServer.DEFAULT_HOST;
    int port = HaraServer.DEFAULT_PORT;
    boolean options = true;
    for (int index = 0; index < arguments.length; index++) {
      String argument = arguments[index];
      if (options && "--".equals(argument)) {
        options = false;
      } else if (options && "--allow-file".equals(argument)) file = true;
      else if (options && ("--allow-net".equals(argument) || "--native-sockets".equals(argument)))
        network = true;
      else if (options && "--allow-process".equals(argument)) process = true;
      else if (options && "--offline".equals(argument)) offline = true;
      else if (options && "--log-requests".equals(argument)) logRequests = true;
      else if (options && "--no-history".equals(argument)) noHistory = true;
      else if (options && "--no-splash".equals(argument)) noSplash = true;
      else if (options && "--no-color".equals(argument)) noColor = true;
      else if (options && argument.startsWith("--project="))
        project = Path.of(requiredOption("--project", argument.substring("--project=".length())));
      else if (options && "--project".equals(argument))
        project = Path.of(nextOption(arguments, ++index, "--project"));
      else if (options && argument.startsWith("--root="))
        project = Path.of(requiredOption("--root", argument.substring("--root=".length())));
      else if (options && "--root".equals(argument))
        project = Path.of(nextOption(arguments, ++index, "--root"));
      else if (options && argument.startsWith("--host="))
        host = requiredOption("--host", argument.substring("--host=".length()));
      else if (options && "--host".equals(argument))
        host = requiredOption("--host", nextOption(arguments, ++index, "--host"));
      else if (options && argument.startsWith("--port="))
        port = parsePort(argument.substring("--port=".length()));
      else if (options && "--port".equals(argument))
        port = parsePort(nextOption(arguments, ++index, "--port"));
      else if (options && argument.startsWith("--history="))
        historyFile = Path.of(requiredOption("--history", argument.substring("--history=".length())));
      else if (options && "--history".equals(argument))
        historyFile = Path.of(nextOption(arguments, ++index, "--history"));
      else if (options && argument.startsWith("--"))
        throw new IllegalArgumentException("Unknown option: " + argument);
      else {
        positional.add(argument);
        options = false;
      }
    }
    return new Capabilities(
        file,
        network,
        process,
        offline,
        host,
        port,
        logRequests,
        noHistory,
        noSplash,
        noColor,
        historyFile,
        project,
        positional.toArray(new String[0]));
  }

  private static String nextOption(String[] arguments, int index, String option) {
    if (index >= arguments.length) throw new IllegalArgumentException(option + " requires a value");
    return requiredOption(option, arguments[index]);
  }

  private static String requiredOption(String option, String value) {
    if (value == null || value.isBlank())
      throw new IllegalArgumentException(option + " requires a value");
    return value;
  }

  private static int parsePort(String value) {
    try {
      int port = Integer.parseInt(requiredOption("--port", value));
      if (port < 0 || port > 65535) throw new NumberFormatException();
      return port;
    } catch (NumberFormatException exception) {
      throw new IllegalArgumentException("--port must be between 0 and 65535");
    }
  }

  private static final class Capabilities {
    private final boolean file;
    private final boolean network;
    private final boolean process;
    private final boolean offline;
    private final String host;
    private final int port;
    private final boolean logRequests;
    private final boolean noHistory;
    private final boolean noSplash;
    private final boolean noColor;
    private final Path historyFile;
    private final Path project;
    private final String[] arguments;

    private Capabilities(
        boolean file,
        boolean network,
        boolean process,
        boolean offline,
        String host,
        int port,
        boolean logRequests,
        boolean noHistory,
        boolean noSplash,
        boolean noColor,
        Path historyFile,
        Path project,
        String[] arguments) {
      this.file = file;
      this.network = network;
      this.process = process;
      this.offline = offline;
      this.host = host;
      this.port = port;
      this.logRequests = logRequests;
      this.noHistory = noHistory;
      this.noSplash = noSplash;
      this.noColor = noColor;
      this.historyFile = historyFile;
      this.project = project;
      this.arguments = arguments;
    }
  }

  private static int runJLineRepl(
      SessionKernel.Session session,
      RespController resp,
      PrintStream output,
      PrintStream error,
      Capabilities capabilities)
      throws IOException {
    try (Terminal terminal = TerminalBuilder.builder().system(true).build()) {
      ReplConfig baseConfig = ReplConfig.defaults(".hara_history");
      if (capabilities.historyFile != null)
        baseConfig = baseConfig.withHistoryFile(capabilities.historyFile);
      if (capabilities.noSplash) baseConfig = baseConfig.withSplash("");
      ReplConfig config =
          baseConfig.withColor(
              baseConfig.color()
                  && !capabilities.noColor
                  && !"dumb".equalsIgnoreCase(terminal.getType()));
      LineReaderBuilder builder =
          LineReaderBuilder.builder()
              .terminal(terminal)
              .parser(new LispLineParser())
              .highlighter(new SlashCommandHighlighter())
              .option(LineReader.Option.AUTO_LIST, true)
              .option(LineReader.Option.AUTO_MENU, true)
              .completer(new HaraCompleter(session));
      if (!capabilities.noHistory) {
        builder
            .variable(LineReader.HISTORY_FILE, config.historyFile())
            .option(LineReader.Option.HISTORY_INCREMENTAL, true);
      }
      LineReader reader = builder.build();
      AutosuggestionWidgets autosuggestions = new AutosuggestionWidgets(reader);
      autosuggestions.enable();
      reader.setAutosuggestion(LineReader.SuggestionType.TAIL_TIP);
      clearTerminal(terminal);
      printHeader(terminal, config, resp, true);
      reader
          .getWidgets()
          .put(
              "show-doc",
              () -> {
                showDocumentation(session, reader, terminal);
                return true;
              });
      reader
          .getKeyMaps()
          .get(LineReader.MAIN)
          .bind(new org.jline.reader.Reference("show-doc"), "\033OP");
      reader
          .getKeyMaps()
          .get(LineReader.MAIN)
          .bind(new org.jline.reader.Reference("show-doc"), "\033q");
      StringBuilder source = new StringBuilder();
      long lastElapsedNanos = -1L;
      String namespace = session.currentNamespace();
      int walkthroughStep = 0;
      while (true) {
        try {
          String prompt =
              source.length() == 0
                  ? config.sessionPrompt(namespace)
                  : config.continuationPrompt();
          String line = reader.readLine(prompt, "", (Character) null, null);
          if (source.length() == 0) {
            String command = line.strip();
            if ("/quit".equals(command) || "/exit".equals(command) || ":quit".equals(command))
              return 0;
            if ("/help".equals(command) || ":help".equals(command)) {
              printInteractiveHelp(terminal);
              continue;
            }
            if (command.equals("/docs") || command.startsWith("/docs ")) {
              printDocs(terminal, command.length() == 5 ? "" : command.substring(6).strip());
              continue;
            }
            if (command.equals("/walkthrough") || command.startsWith("/walkthrough ")) {
              String action = command.length() == 12 ? "" : command.substring(13).strip();
              walkthroughStep = printWalkthrough(terminal, action, walkthroughStep);
              continue;
            }
            if (command.startsWith("/history") || command.startsWith(":history")) {
              String query = command.replaceFirst("^[/:]history\\s*", "").strip();
              for (History.Entry entry : reader.getHistory()) {
                if (query.isEmpty() || fuzzyScore(query, entry.line()) < Integer.MAX_VALUE)
                  terminal.writer().println((entry.index() + 1) + ": " + entry.line());
              }
              terminal.writer().println();
              terminal.writer().flush();
              continue;
            }
            if (command.startsWith("/doc ")) {
              showDocumentation(session, command.substring(5).strip(), terminal);
              continue;
            }
            if (command.startsWith("/apropos ")) {
              printApropos(session, command.substring(9).strip(), terminal);
              continue;
            }
            if ("/time".equals(command)) {
              terminal
                  .writer()
                  .println(
                      lastElapsedNanos < 0
                          ? "No evaluation yet."
                          : formatElapsed(lastElapsedNanos));
              terminal.writer().println();
              terminal.writer().flush();
              continue;
            }
            if ("/status".equals(command)) {
              printHeader(terminal, config, resp, false);
              continue;
            }
            if ("/clear".equals(command)) {
              clearTerminal(terminal);
              printHeader(terminal, config, resp, true);
              continue;
            }
            if ("/splash".equals(command)) {
              printHeader(terminal, config, resp, true);
              continue;
            }
            if ("/ns".equals(command)) {
              terminal.writer().println(namespace);
              terminal.writer().println();
              terminal.writer().flush();
              continue;
            }
            if (command.equals("/resp") || command.startsWith("/resp ")) {
              terminal.writer().println(resp.command(command));
              terminal.writer().println();
              terminal.writer().flush();
              continue;
            }
            if (line.startsWith("/")) {
              terminal.writer().println("Unknown command: " + command + ". Try /help.");
              terminal.writer().println();
              terminal.writer().flush();
              continue;
            }
          }
          source.append(line).append('\n');
          if (!isComplete(source)) continue;
          long started = System.nanoTime();
          try {
            output.println("=> " + display(session.eval(source.toString())));
            output.println();
            namespace = session.currentNamespace();
          } catch (RuntimeException exception) {
            error.println(exception.getMessage());
            error.println();
          } finally {
            lastElapsedNanos = System.nanoTime() - started;
          }
          source.setLength(0);
        } catch (UserInterruptException exception) {
          source.setLength(0);
          output.println("^C");
          output.println();
        } catch (EndOfFileException exception) {
          return 0;
        }
      }
    }
  }

  private static void printHeader(
      Terminal terminal, ReplConfig config, RespController resp, boolean includeSplash) {
    if (includeSplash && !config.splash().isBlank()) {
      terminal.writer().println(config.renderedSplash());
      terminal.writer().println();
    }
    terminal.writer().println("HARA · TRUFFLE                                      SESSION ROOT");
    terminal.writer().println(config.tagline("JOURNEY WITHIN"));
    terminal.writer().println("────────────────────────────────────────────────────────────────");
    terminal.writer().println();
    terminal
        .writer()
        .println("  /docs  Docs       /walkthrough  Tour");
    terminal
        .writer()
        .println("  /help  Help       /history      History");
    terminal
        .writer()
        .println("  /status Status    /resp         Listener");
    terminal
        .writer()
        .println("  /clear Clear      /quit         Exit");
    terminal.writer().println();
    terminal.writer().println("RESP  " + resp.status());
    terminal.writer().println();
    terminal.writer().flush();
  }

  private static final String[][] DOC_TOPICS = {
    {"language", "forms, literals, and evaluation"},
    {"collections", "vectors, maps, sets, and sequences"},
    {"functions", "bindings, functions, and metadata"},
    {"namespaces", ".hal files, :require, and :import"},
    {"interop", "platform-neutral member access"},
    {"repl", "discovery and terminal commands"}
  };

  private static final String[][] WALKTHROUGH_ACTIONS = {
    {"next", "continue to the next page"},
    {"prev", "return to the previous page"},
    {"1", "expressions"}, {"2", "data"}, {"3", "bindings and functions"},
    {"4", "namespaces and packages"}, {"5", "discover and connect"},
    {"stop", "close the walkthrough"}
  };

  private static final String[][] RESP_ACTIONS = {
    {"start", "start the listener"},
    {"stop", "stop the listener and keep ROOT"},
    {"restart", "restart, optionally at HOST:PORT"}
  };

  private static final String[][] REPL_COMMANDS = {
    {"/docs", "browse built-in language documentation"},
    {"/walkthrough", "start the guided Hara walkthrough"},
    {"/help", "show REPL commands"},
    {"/history", "show persistent input history"},
    {"/clear", "clear the terminal and redraw the header"},
    {"/splash", "show the Hara splash and header"},
    {"/status", "show ROOT and RESP status"},
    {"/resp", "show or control the RESP listener"},
    {"/ns", "show the current namespace"},
    {"/doc", "show documentation for a symbol"},
    {"/apropos", "search documented symbols"},
    {"/time", "show the last evaluation time"},
    {"/quit", "exit the REPL"},
    {"/exit", "exit the REPL"}
  };


  private static int fuzzyScore(String query, String value) {
    if (query == null || query.isEmpty()) return 0;
    if (value.equals(query)) return 0;
    String q = query.toLowerCase(java.util.Locale.ROOT);
    String v = value.toLowerCase(java.util.Locale.ROOT);
    if (v.startsWith(q)) return 10 + value.length() - query.length();
    if (v.contains(q)) return 100 + v.indexOf(q);
    int index = 0;
    int gaps = 0;
    for (int i = 0; i < v.length() && index < q.length(); i++) {
      if (v.charAt(i) == q.charAt(index)) index++;
      else if (index > 0) gaps++;
    }
    return index == q.length() ? 200 + gaps : Integer.MAX_VALUE;
  }

  private static String formatElapsed(long nanos) {
    if (nanos < 1_000_000L) return nanos + " ns";
    if (nanos < 1_000_000_000L)
      return String.format(java.util.Locale.ROOT, "%.2f ms", nanos / 1_000_000.0);
    return String.format(java.util.Locale.ROOT, "%.2f s", nanos / 1_000_000_000.0);
  }

  private static void clearTerminal(Terminal terminal) {
    terminal.puts(InfoCmp.Capability.clear_screen);
    terminal.flush();
  }

  private static void showDocumentation(
      SessionKernel.Session session, LineReader reader, Terminal terminal) {
    String symbol =
        HaraCompleter.extractWord(reader.getBuffer().toString(), reader.getBuffer().cursor());
    showDocumentation(session, symbol, terminal);
  }

  private static void showDocumentation(
      SessionKernel.Session session, String symbol, Terminal terminal) {
    if (symbol == null || symbol.isEmpty() || symbol.startsWith("/")) return;
    try {
      Value doc = session.eval("(get (meta #'" + symbol + ") :doc)");
      Value arglists = session.eval("(get (meta #'" + symbol + ") :arglists)");
      terminal.writer().println();
      terminal.writer().println("Documentation: " + symbol);
      if (arglists != null && !arglists.isNull())
        terminal.writer().println("  Arglists: " + arglists);
      if (doc != null && !doc.isNull()) terminal.writer().println("  " + doc.asString());
      terminal.writer().println();
      terminal.writer().flush();
    } catch (RuntimeException ignored) {
      terminal.writer().println("No documentation for " + symbol);
      terminal.writer().println();
      terminal.writer().flush();
    }
  }

  private static void printApropos(
      SessionKernel.Session session, String query, Terminal terminal) {
    try {
      for (String name : session.currentSymbols()) {
        if (fuzzyScore(query, name) == Integer.MAX_VALUE) continue;
        Value doc = session.eval("(get (meta #'" + name + ") :doc)");
        if (doc != null && !doc.isNull()) terminal.writer().println(name + " — " + doc.asString());
      }
      terminal.writer().println();
      terminal.writer().flush();
    } catch (RuntimeException ignored) {
      terminal.writer().println("Unable to search documentation.");
      terminal.writer().println();
      terminal.writer().flush();
    }
  }

  private static final String[][] WALKTHROUGH = {
    {
      "Expressions",
      "Hara evaluates data-shaped expressions from the inside out.",
      "Try: (+ 1 2 3)"
    },
    {
      "Data",
      "Vectors, maps, sets, keywords, strings, and numbers are literal values.",
      "Try: (get {:name \"Hara\" :kind :language} :name)"
    },
    {
      "Bindings and functions",
      "Use let for local names and fn for reusable behavior.",
      "Try: (let [double (fn [x] (* x 2))] (double 21))"
    },
    {
      "Namespaces and packages",
      "Hara source files use .hal; ns :require loads packaged interfaces.",
      "Example: (ns app.core (:require [demo.000-answer-42 :as answer]))"
    },
    {
      "Discover and connect",
      "Use /doc SYMBOL and /apropos QUERY to explore; /resp controls the listener.",
      "Try: /doc map"
    }
  };

  private static void printDocs(Terminal terminal, String requestedTopic) {
    String topic = requestedTopic.toLowerCase(java.util.Locale.ROOT);
    terminal.writer().println();
    switch (topic) {
      case "":
        terminal.writer().println("HARA DOCS");
        terminal.writer().println("  /docs language      forms, literals, and evaluation");
        terminal.writer().println("  /docs collections   vectors, maps, sets, and sequences");
        terminal.writer().println("  /docs functions     bindings, functions, and metadata");
        terminal.writer().println("  /docs namespaces    .hal files, ns, :require, and :import");
        terminal.writer().println("  /docs interop       platform-neutral member access");
        terminal.writer().println("  /docs repl          discovery and terminal commands");
        terminal.writer().println();
        terminal.writer().println("Use /doc SYMBOL for live API metadata or /walkthrough to learn by doing.");
        break;
      case "language":
      case "basics":
        printDocSection(
            terminal,
            "LANGUAGE",
            "Lists are calls: (+ 1 2) evaluates to 3.",
            "Literal data includes nil, booleans, numbers, strings, keywords, vectors, maps, and sets.",
            "Core forms include quote, if, do, let, fn, def, and ns.");
        break;
      case "collections":
        printDocSection(
            terminal,
            "COLLECTIONS",
            "Vectors: [1 2 3]    Maps: {:name \"Hara\"}    Sets: #{:a :b}",
            "Use get, assoc, dissoc, conj, count, first, rest, map, filter, and reduce.",
            "Collections are persistent values; updates return a new value.");
        break;
      case "functions":
        printDocSection(
            terminal,
            "FUNCTIONS",
            "Anonymous: (fn [x] (* x x))",
            "Named: (def square (fn [x] (* x x)))",
            "Functions support fixed and variadic parameter lists; /doc shows arglists and metadata.");
        break;
      case "namespaces":
      case "packages":
        printDocSection(
            terminal,
            "NAMESPACES · PACKAGES",
            "Hara source files use the .hal extension.",
            "Declare dependencies in ns with :require and aliases with :as.",
            "JVM-specific capabilities belong behind packaged modules and explicit :import declarations.");
        break;
      case "interop":
        printDocSection(
            terminal,
            "INTEROP",
            "The . form is the platform-neutral member operation.",
            "(. value member args...) invokes a member; (. value field field) reads a field.",
            "The active runtime decides how those operations map onto JVM, Truffle, or packaged values.");
        break;
      case "repl":
      case "terminal":
        printDocSection(
            terminal,
            "REPL",
            "Tab completes slash commands and visible symbols at the cursor.",
            "Use /doc SYMBOL, /apropos QUERY, /history, /time, /status, and /resp.",
            "Use /walkthrough next and /walkthrough prev to navigate the guided tour.");
        break;
      default:
        terminal.writer().println("Unknown docs topic: " + requestedTopic);
        terminal.writer().println("Try /docs for the topic index.");
    }
    terminal.writer().println();
    terminal.writer().flush();
  }

  private static void printDocSection(Terminal terminal, String title, String... lines) {
    terminal.writer().println(title);
    for (String line : lines) terminal.writer().println("  " + line);
  }

  private static int printWalkthrough(Terminal terminal, String action, int currentStep) {
    String value = action.toLowerCase(java.util.Locale.ROOT);
    if ("stop".equals(value) || "exit".equals(value)) {
      terminal.writer().println();
      terminal.writer().println("Walkthrough closed. Use /walkthrough to begin again.");
      terminal.writer().println();
      terminal.writer().flush();
      return 0;
    }
    int step;
    if (value.isEmpty() || "start".equals(value)) step = 1;
    else if ("next".equals(value)) step = Math.min(WALKTHROUGH.length, Math.max(1, currentStep + 1));
    else if ("prev".equals(value) || "previous".equals(value))
      step = Math.max(1, currentStep <= 1 ? 1 : currentStep - 1);
    else {
      try {
        step = Integer.parseInt(value);
      } catch (NumberFormatException error) {
        terminal.writer().println("Usage: /walkthrough [next|prev|1-5|stop]");
        terminal.writer().println();
        terminal.writer().flush();
        return currentStep;
      }
      if (step < 1 || step > WALKTHROUGH.length) {
        terminal.writer().println("Walkthrough step must be between 1 and " + WALKTHROUGH.length + ".");
        terminal.writer().println();
        terminal.writer().flush();
        return currentStep;
      }
    }
    String[] page = WALKTHROUGH[step - 1];
    terminal.writer().println();
    terminal.writer().println("WALKTHROUGH " + step + "/" + WALKTHROUGH.length + " · " + page[0]);
    terminal.writer().println("  " + page[1]);
    terminal.writer().println("  " + page[2]);
    terminal.writer().println();
    if (step < WALKTHROUGH.length) terminal.writer().println("  Continue: /walkthrough next");
    else terminal.writer().println("  Complete. Use /docs to keep exploring.");
    terminal.writer().println("  Navigate: /walkthrough prev · /walkthrough 1-5 · /walkthrough stop");
    terminal.writer().println();
    terminal.writer().flush();
    return step;
  }

  private static void printInteractiveHelp(Terminal terminal) {
    terminal.writer().println();
    terminal.writer().println("REPL");
    printHelpEntry(terminal, "/help", "show this command guide");
    printHelpEntry(terminal, "/docs [TOPIC]", "browse built-in Hara documentation");
    printHelpEntry(terminal, "/walkthrough [next|prev|1-5|stop]", "navigate the guided tour");
    printHelpEntry(terminal, "/history [QUERY]", "search persistent input history");
    printHelpEntry(terminal, "/clear", "clear the terminal and redraw the menu");
    printHelpEntry(terminal, "/splash", "redraw the splash and menu");
    printHelpEntry(terminal, "/time", "show the last evaluation time");
    terminal.writer().println();
    terminal.writer().println("SESSION · ROOT");
    printHelpEntry(terminal, "/status", "show session and listener status");
    printHelpEntry(terminal, "/ns", "show the current namespace");
    printHelpEntry(terminal, "/doc SYMBOL", "show symbol documentation");
    printHelpEntry(terminal, "/apropos QUERY", "search documented symbols");
    terminal.writer().println();
    terminal.writer().println("RESP LISTENER");
    printHelpEntry(terminal, "/resp", "show listener status");
    printHelpEntry(terminal, "/resp start [PORT|HOST:PORT]", "start the listener");
    printHelpEntry(terminal, "/resp stop", "stop the listener; keep ROOT");
    printHelpEntry(terminal, "/resp restart [PORT|HOST:PORT]", "restart the listener; keep ROOT");
    terminal.writer().println();
    terminal.writer().println("EXIT");
    printHelpEntry(terminal, "/quit", "leave Hara");
    printHelpEntry(terminal, "/exit", "leave Hara");
    terminal.writer().println();
    terminal.writer().println("Tab completes commands and visible Hara symbols.");
    terminal.writer().println();
    terminal.writer().flush();
  }

  private static void printHelpEntry(Terminal terminal, String command, String description) {
    terminal.writer().printf("  %-36s %s%n", command, description);
  }

  static String display(Value result) {
    if (result.isNull()) return "nil";
    if (result.hasHashEntries()) return result.toString();
    if (result.isHostObject() && result.asHostObject() instanceof Iterator<?>) {
      return "#<lazy-iterator>";
    }
    if (result.isString()) return G.display(result.asString());
    return result.toString();
  }

  private static boolean isComplete(CharSequence source) {
    int depth = 0;
    boolean string = false;
    boolean escape = false;
    boolean comment = false;
    for (int i = 0; i < source.length(); i++) {
      char ch = source.charAt(i);
      if (comment) {
        if (ch == '\n' || ch == '\r') {
          comment = false;
        }
        continue;
      }
      if (string) {
        if (escape) {
          escape = false;
        } else if (ch == '\\') {
          escape = true;
        } else if (ch == '"') {
          string = false;
        }
        continue;
      }
      if (ch == ';') {
        comment = true;
      } else if (ch == '"') {
        string = true;
      } else if (ch == '(' || ch == '[' || ch == '{') {
        depth++;
      } else if (ch == ')' || ch == ']' || ch == '}') {
        depth--;
      }
    }
    return depth == 0 && !string && !escape;
  }

  private static void printUsage(PrintStream output) {
    output.println("Hara CLI · Truffle runtime");
    output.println();
    output.println("Usage:");
    output.println("  hara [OPTIONS] repl");
    output.println("  hara eval EXPRESSION | run FILE | stdin");
    output.println("  hara server | remote HOST:PORT");
    output.println("  hara project <new|check|run|test|add|remove|sync|update> ...");
    output.println("  hara package <COMMAND> ...");
    output.println("  hara id <login|enroll|status|key|namespace> ...");
    output.println("  hara asset <check|build|inspect|publish|status|search|info|pull|sync|yank> ...");
    output.println("  hara tap <bootstrap|init|add|remove|list|verify|mirror> ...");
    output.println("  hara spec <COMMAND> ...");
    output.println("  hara extension <check|build|install|test> ...");
    output.println("  hara bytecode <run|disassemble> FILE.hbc|FILE.hbx");
    output.println("  hara bytecode conformance FILE.hcc");
    output.println();
    output.println("Compatibility aliases:");
    output.println("  new check test add remove sync update headless standalone");
    output.println();
    output.println("Global options:");
    output.println("  --project PATH, --root PATH, --offline");
    output.println("  --allow-file, --allow-net, --allow-process");
    output.println("  --host HOST, --port PORT, --history PATH");
    output.println("  --no-history, --no-splash, --no-color, --log-requests");
  }


  static final class RespController implements AutoCloseable {
    private final SessionKernel kernel;
    private final boolean logRequests;
    private String host;
    private int port;
    private HaraServer server;

    RespController(SessionKernel kernel, String host, int port, boolean logRequests) {
      this.kernel = kernel;
      this.host = host;
      this.port = port;
      this.logRequests = logRequests;
    }

    synchronized void start() throws IOException {
      if (isRunning()) return;
      HaraServer candidate = new HaraServer(kernel, host, port, logRequests);
      try {
        candidate.start();
        server = candidate;
        port = candidate.port();
      } catch (IOException exception) {
        candidate.close();
        throw exception;
      }
    }

    synchronized void stop() {
      if (server != null) server.close();
      server = null;
    }

    synchronized boolean isRunning() {
      return server != null && server.isRunning();
    }

    synchronized String endpoint() {
      return host + ":" + (isRunning() ? server.port() : port);
    }

    synchronized String status() {
      return isRunning() ? "● " + endpoint() : "○ offline";
    }

    synchronized String command(String line) {
      String[] parts = line.strip().split("\\s+");
      if (parts.length == 1) return "RESP " + status();
      String action = parts[1].toLowerCase(java.util.Locale.ROOT);
      try {
        if ("stop".equals(action)) {
          stop();
          return "RESP ○ offline";
        }
        if ("start".equals(action) || "restart".equals(action)) {
          if (parts.length > 3)
            return "Usage: /resp " + action + " [PORT|HOST:PORT]";
          String nextHost = host;
          int nextPort = port;
          if (parts.length == 3) {
            Endpoint endpoint = Endpoint.parse(parts[2], host);
            nextHost = endpoint.host;
            nextPort = endpoint.port;
          }
          if ("start".equals(action) && isRunning())
            return "RESP already running at " + endpoint();
          stop();
          host = nextHost;
          port = nextPort;
          start();
          return "RESP ● " + endpoint();
        }
        return "Usage: /resp [start [PORT|HOST:PORT]|stop|restart [PORT|HOST:PORT]]";
      } catch (IOException | IllegalArgumentException exception) {
        return "RESP error: " + exception.getMessage();
      }
    }

    @Override
    public synchronized void close() {
      stop();
    }
  }

  static final class Endpoint {
    private final String host;
    private final int port;

    private Endpoint(String host, int port) {
      this.host = host;
      this.port = port;
    }

    static Endpoint parse(String value, String defaultHost) {
      int separator = value.lastIndexOf(':');
      if (separator < 0) return new Endpoint(defaultHost, parsePort(value));
      String host = requiredOption("host", value.substring(0, separator));
      return new Endpoint(host, parsePort(value.substring(separator + 1)));
    }
  }

  private static final class SlashCommandHighlighter implements Highlighter {

    public AttributedString highlight(LineReader reader, String buffer) {
      String suggestion = argumentHint(buffer);
      if (suggestion.isEmpty()
          && reader.getBuffer().cursor() == buffer.length()
          && buffer.startsWith("/")
          && buffer.indexOf(' ') < 0) {
        for (String[] command : REPL_COMMANDS) {
          if (command[0].startsWith(buffer) && command[0].length() > buffer.length()) {
            suggestion = command[0].substring(buffer.length());
            break;
          }
        }
      }
      reader.setTailTip(suggestion);
      if (!buffer.startsWith("/")) return new AttributedString(buffer);
      int separator = buffer.indexOf(' ');
      String command = separator < 0 ? buffer : buffer.substring(0, separator);
      boolean known = false;
      for (String[] candidate : REPL_COMMANDS) {
        if (candidate[0].equals(command)
            || (separator < 0 && candidate[0].startsWith(command))) {
          known = true;
          break;
        }
      }
      String color = known ? "\u001b[36;1m" : "\u001b[31;1m";
      if (separator < 0) return AttributedString.fromAnsi(color + buffer + "\u001b[0m");
      return AttributedString.fromAnsi(
          color
              + command
              + "\u001b[0m"
              + buffer.substring(command.length(), separator + 1)
              + "\u001b[35m"
              + buffer.substring(separator + 1)
              + "\u001b[0m");
    }

    private String argumentHint(String buffer) {
      if ("/docs".equals(buffer))
        return "  [language · collections · functions · namespaces · interop · repl]";
      if ("/walkthrough".equals(buffer)) return "  [next · prev · 1-5 · stop]";
      if ("/resp".equals(buffer)) return "  [start · stop · restart]";
      if ("/doc".equals(buffer)) return "  SYMBOL";
      if ("/apropos".equals(buffer)) return "  QUERY";
      if ("/history".equals(buffer)) return "  [QUERY]";
      String hint = matchingHint(buffer, "/docs ", DOC_TOPICS);
      if (!hint.isEmpty()) return hint;
      hint = matchingHint(buffer, "/walkthrough ", WALKTHROUGH_ACTIONS);
      if (!hint.isEmpty()) return hint;
      return matchingHint(buffer, "/resp ", RESP_ACTIONS);
    }

    private String matchingHint(String buffer, String command, String[][] choices) {
      if (!buffer.startsWith(command)) return "";
      String prefix = buffer.substring(command.length());
      for (String[] choice : choices) {
        if (choice[0].startsWith(prefix))
          return choice[0].substring(prefix.length()) + "  — " + choice[1];
      }
      return "";
    }

    @Override
    public void setErrorPattern(java.util.regex.Pattern pattern) {}

    @Override
    public void setErrorIndex(int index) {}
  }

  private static final class LispLineParser extends DefaultParser {
    @Override
    public boolean isDelimiterChar(CharSequence buffer, int pos) {
      char c = buffer.charAt(pos);
      return c == '('
          || c == ')'
          || c == '['
          || c == ']'
          || c == '{'
          || c == '}'
          || Character.isWhitespace(c)
          || c == '"'
          || c == '\'';
    }
  }

  private static final class HaraCompleter implements Completer {
    private final SessionKernel.Session session;

    private HaraCompleter(SessionKernel.Session session) {
      this.session = session;
    }

    @Override
    public void complete(LineReader reader, ParsedLine line, List<Candidate> candidates) {
      String buffer = line.line();
      int cursor = Math.min(line.cursor(), buffer.length());
      int start = wordStart(buffer, cursor);
      String prefix = buffer.substring(start, cursor);
      if (buffer.startsWith("/docs ")) {
        addValueCandidates(prefix, DOC_TOPICS, "Documentation topics", candidates);
        return;
      }
      if (buffer.startsWith("/walkthrough ")) {
        addValueCandidates(prefix, WALKTHROUGH_ACTIONS, "Walkthrough navigation", candidates);
        return;
      }
      if (prefix.startsWith("/")) {
        addCommandCandidates(prefix, candidates);
        return;
      }
      try {
        List<ScoredCandidate> matches = new ArrayList<>();
        for (String name : session.currentSymbols()) {
          int score = fuzzyScore(prefix, name);
          if (score < Integer.MAX_VALUE)
            matches.add(new ScoredCandidate(name, score, describe(name)));
        }
        matches.sort(
            (left, right) -> {
              int result = Integer.compare(left.score, right.score);
              return result == 0 ? left.name.compareTo(right.name) : result;
            });
        for (ScoredCandidate match : matches)
          candidates.add(
              new Candidate(
                  match.name, match.name, "Hara symbols", match.description, null, null, true));
      } catch (RuntimeException ignored) {
        // Completion must never disrupt the REPL when the session is unavailable.
      }
    }

    private void addValueCandidates(
        String prefix, String[][] values, String group, List<Candidate> candidates) {
      for (String[] value : values) {
        if (fuzzyScore(prefix, value[0]) != Integer.MAX_VALUE)
          candidates.add(
              new Candidate(value[0], value[0], group, value[1], null, null, true));
      }
    }

    private void addCommandCandidates(String prefix, List<Candidate> candidates) {
      for (String[] command : REPL_COMMANDS) {
        if (fuzzyScore(prefix, command[0]) != Integer.MAX_VALUE)
          candidates.add(
              new Candidate(
                  command[0],
                  command[0],
                  "REPL commands",
                  command[1],
                  commandTakesArgument(command[0]) ? " " : null,
                  null,
                  !commandTakesArgument(command[0])));
      }
    }

    private boolean commandTakesArgument(String command) {
      return "/docs".equals(command)
          || "/walkthrough".equals(command)
          || "/resp".equals(command)
          || "/doc".equals(command)
          || "/apropos".equals(command)
          || "/history".equals(command);
    }

    private String describe(String name) {
      try {
        String escaped = name.replace("\\", "\\\\").replace("\"", "\\\"");
        Value doc = session.eval("(get (meta #'" + escaped + ") :doc)");
        Value arglists = session.eval("(get (meta #'" + escaped + ") :arglists)");
        StringBuilder result = new StringBuilder();
        if (arglists != null && !arglists.isNull()) result.append(arglists);
        if (doc != null && !doc.isNull()) {
          if (result.length() > 0) result.append(" — ");
          result.append(doc.asString().replace('\n', ' '));
        }
        return result.toString();
      } catch (RuntimeException ignored) {
        return "";
      }
    }

    public static String extractWord(String buffer, int cursor) {
      if (buffer == null || buffer.isEmpty() || cursor <= 0) return "";
      int start = wordStart(buffer, Math.min(cursor, buffer.length()));
      return buffer.substring(start, Math.min(cursor, buffer.length()));
    }

    private static int wordStart(String buffer, int cursor) {
      if (buffer == null || buffer.isEmpty() || cursor <= 0) return 0;
      cursor = Math.min(cursor, buffer.length());
      int start = Math.max(0, cursor - 1);
      while (start >= 0) {
        char c = buffer.charAt(start);
        if (c == '('
            || c == ')'
            || c == '['
            || c == ']'
            || c == '{'
            || c == '}'
            || Character.isWhitespace(c)
            || c == '"'
            || c == '\'')
          return start + 1;
        start--;
      }
      return 0;
    }

    private static final class ScoredCandidate {
      private final String name;
      private final int score;
      private final String description;

      private ScoredCandidate(String name, int score, String description) {
        this.name = name;
        this.score = score;
        this.description = description;
      }
    }
  }

}
