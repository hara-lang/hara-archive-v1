package hara.truffle;

import hara.kernel.base.Parser;
import hara.lang.base.G;
import hara.lang.data.Keyword;
import hara.lang.protocol.ILinearType;
import hara.lang.protocol.IMapType;
import java.io.IOException;
import java.io.PrintStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.HashSet;
import java.util.HexFormat;
import java.util.List;
import java.util.regex.Pattern;

/** Local deterministic asset collection commands for the Truffle CLI. */
final class HaraAssetTool {
  private static final Pattern VERSION =
      Pattern.compile("^[0-9]+\\.[0-9]+\\.[0-9]+(?:[-+][0-9A-Za-z.-]+)?$");

  private record Entry(String path, String mediaType) {}

  private record Collection(Path root, String coordinate, String version, List<Entry> entries) {}

  private HaraAssetTool() {}

  static int run(String[] arguments, PrintStream output, PrintStream error) {
    if (arguments.length == 0 || "--help".equals(arguments[0]) || "-h".equals(arguments[0])) {
      usage(output);
      return 0;
    }
    try {
      return switch (arguments[0]) {
        case "check" -> check(path(arguments, 1, Path.of(".")), output);
        case "build" -> build(arguments, output);
        case "inspect" -> inspect(required(arguments, 1, "asset inspect requires MANIFEST"), output);
        case "publish", "status", "search", "info", "pull", "sync", "yank" -> {
          error.println(
              "unavailable: hara asset "
                  + arguments[0]
                  + " requires the packages.hara-lang.org registry client");
          yield 2;
        }
        default -> {
          error.println("unknown asset command: " + arguments[0]);
          yield 2;
        }
      };
    } catch (HaraException | IOException exception) {
      error.println(exception.getMessage());
      return exception instanceof HaraException ? 1 : 2;
    }
  }

  private static int check(Path input, PrintStream output) throws IOException {
    Collection collection = read(input);
    verifyFiles(collection);
    output.println(
        "asset check: "
            + collection.coordinate()
            + " "
            + collection.version()
            + " files="
            + collection.entries().size());
    return 0;
  }

  private static int build(String[] arguments, PrintStream output) throws IOException {
    Collection collection = read(path(arguments, 1, Path.of(".")));
    Path destination = option(arguments, "--output");
    if (destination == null) destination = collection.root().resolve("target/asset-manifest.edn");
    String manifest = manifest(collection);
    if (destination.getParent() != null) Files.createDirectories(destination.getParent());
    Files.writeString(destination, manifest, StandardCharsets.UTF_8);
    output.println("asset build: " + destination);
    return 0;
  }

  private static int inspect(Path input, PrintStream output) throws IOException {
    output.print(Files.readString(input, StandardCharsets.UTF_8));
    return 0;
  }

  @SuppressWarnings({"rawtypes", "unchecked"})
  private static Collection read(Path input) throws IOException {
    Path descriptor = Files.isDirectory(input) ? input.resolve("asset.edn") : input;
    Object value =
        Parser.LispReader.readString(Files.readString(descriptor, StandardCharsets.UTF_8), null);
    if (!(value instanceof IMapType map)) throw new HaraException("asset.edn must be an EDN map");
    Object format = map.lookup(Keyword.create("asset/format"));
    if (!"0.0.0-alpha".equals(format))
      throw new HaraException("asset.edn requires alpha asset format");
    String coordinate = scalar(map, "asset/coordinate");
    coordinate = normalizeCoordinate(coordinate);
    String version = scalar(map, "asset/version");
    if (!VERSION.matcher(version).matches())
      throw new HaraException("asset.edn :asset/version must be semantic version text");
    Object entryValue = map.lookup(Keyword.create("asset/entries"));
    if (!(entryValue instanceof ILinearType entries) || entries.count() == 0)
      throw new HaraException("asset.edn :asset/entries must be a non-empty vector");
    ArrayList<Entry> parsed = new ArrayList<>();
    HashSet<String> paths = new HashSet<>();
    for (Object entryValueItem : entries) {
      if (!(entryValueItem instanceof IMapType entry))
        throw new HaraException("asset entry must be a map");
      String path = scalar(entry, "entry/path");
      safePath(path);
      if (!paths.add(path)) throw new HaraException("asset.edn contains duplicate :entry/path values");
      parsed.add(new Entry(path.replace('\\', '/'), scalar(entry, "entry/media-type")));
    }
    return new Collection(descriptor.toAbsolutePath().getParent(), coordinate, version, List.copyOf(parsed));
  }

  private static String manifest(Collection collection) throws IOException {
    verifyFiles(collection);
    ArrayList<Entry> entries = new ArrayList<>(collection.entries());
    entries.sort(Comparator.comparing(Entry::path));
    StringBuilder output =
        new StringBuilder(
            "{:asset/format \"0.0.0-alpha\"\n :asset/coordinate "
                + G.display(collection.coordinate())
                + "\n :asset/version "
                + G.display(collection.version())
                + "\n :asset/entries [\n");
    for (Entry entry : entries) {
      byte[] bytes = Files.readAllBytes(collection.root().resolve(entry.path()));
      output
          .append("  {:entry/path ")
          .append(G.display(entry.path()))
          .append(" :entry/media-type ")
          .append(G.display(entry.mediaType()))
          .append(" :entry/size ")
          .append(bytes.length)
          .append(" :entry/sha256 \"sha256:")
          .append(sha256(bytes))
          .append("\"}\n");
    }
    return output.append(" ]}\n").toString();
  }

  private static void verifyFiles(Collection collection) {
    for (Entry entry : collection.entries()) {
      Path path = collection.root().resolve(entry.path()).normalize();
      if (!path.startsWith(collection.root()) || !Files.isRegularFile(path))
        throw new HaraException("asset entry does not exist: " + path);
    }
  }

  private static void safePath(String value) {
    Path path = Path.of(value);
    if (value.isEmpty() || path.isAbsolute() || path.normalize().startsWith(".."))
      throw new HaraException("unsafe asset path: " + value);
    for (Path part : path) {
      if (".".equals(part.toString()) || "..".equals(part.toString()))
        throw new HaraException("unsafe asset path: " + value);
    }
  }

  private static String normalizeCoordinate(String value) {
    if (value.startsWith("official:")) value = "hara:" + value.substring("official:".length());
    else if (!value.contains(":")) value = "hara:" + value;
    if (!value.matches("^[a-z0-9_.-]+:[a-z0-9_.-]+/[a-z0-9_.-]+$"))
      throw new HaraException("invalid asset coordinate: " + value);
    return value;
  }

  @SuppressWarnings("rawtypes")
  private static String scalar(IMapType map, String key) {
    Object value = map.lookup(Keyword.create(key));
    if (value == null) throw new HaraException("asset.edn is missing :" + key);
    if (value instanceof String text) return text;
    return G.display(value);
  }

  private static String sha256(byte[] value) {
    try {
      return HexFormat.of().formatHex(MessageDigest.getInstance("SHA-256").digest(value));
    } catch (NoSuchAlgorithmException impossible) {
      throw new IllegalStateException(impossible);
    }
  }

  private static Path path(String[] arguments, int index, Path fallback) {
    if (arguments.length <= index || arguments[index].startsWith("--")) return fallback;
    return Path.of(arguments[index]);
  }

  private static Path required(String[] arguments, int index, String message) {
    if (arguments.length <= index) throw new HaraException(message);
    return Path.of(arguments[index]);
  }

  private static Path option(String[] arguments, String name) {
    for (int index = 0; index < arguments.length; index++) {
      if (name.equals(arguments[index])) {
        if (index + 1 >= arguments.length) throw new HaraException(name + " requires a value");
        return Path.of(arguments[index + 1]);
      }
    }
    return null;
  }

  private static void usage(PrintStream output) {
    output.println("hara asset check [PATH]");
    output.println("hara asset build [PATH] [--output PATH]");
    output.println("hara asset inspect MANIFEST");
    output.println("hara asset <publish|status|search|info|pull|sync|yank>");
  }
}
