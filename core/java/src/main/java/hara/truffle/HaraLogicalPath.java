package hara.truffle;

import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;

/** Portable path algebra for the logical filesystem mounted at {@code /}. */
public final class HaraLogicalPath {
  public static final class Error extends IllegalArgumentException {
    private final String code;

    public Error(String code, String message) {
      super(message);
      this.code = code;
    }

    public String code() {
      return code;
    }
  }

  private HaraLogicalPath() {}

  public static String normalise(String input) {
    if (input == null) throw invalid("logical path must be a string");
    if (input.indexOf('\0') >= 0) throw invalid("logical path contains NUL");
    if (input.indexOf('\\') >= 0) {
      throw invalid("logical paths use '/' rather than host separators");
    }
    ArrayList<String> segments = new ArrayList<>();
    for (String segment : input.split("/", -1)) {
      if (segment.isEmpty() || ".".equals(segment)) continue;
      if ("..".equals(segment)) {
        if (segments.isEmpty()) {
          throw new Error("outside-root", "logical path escapes above the mounted root");
        }
        segments.remove(segments.size() - 1);
        continue;
      }
      if (isDrivePrefix(segment)) {
        throw invalid("logical paths do not accept host drive prefixes");
      }
      segments.add(segment);
    }
    return segments.isEmpty() ? "/" : "/" + String.join("/", segments);
  }

  public static String join(String base, String path) {
    String canonicalBase = normalise(base);
    String relative = path == null ? null : path;
    if (relative == null) throw invalid("logical path must be a string");
    while (relative.startsWith("/")) relative = relative.substring(1);
    return normalise(("/".equals(canonicalBase) ? "" : canonicalBase) + "/" + relative);
  }

  public static String resolve(String base, String path) {
    if (path == null) throw invalid("logical path must be a string");
    return path.startsWith("/") ? normalise(path) : join(base, path);
  }

  public static String parent(String path) {
    String canonical = normalise(path);
    if ("/".equals(canonical)) return null;
    int separator = canonical.lastIndexOf('/');
    return separator == 0 ? "/" : canonical.substring(0, separator);
  }

  public static String fileName(String path) {
    String canonical = normalise(path);
    if ("/".equals(canonical)) return "";
    return canonical.substring(canonical.lastIndexOf('/') + 1);
  }

  public static List<String> segments(String path) {
    String canonical = normalise(path);
    if ("/".equals(canonical)) return List.of();
    return List.of(canonical.substring(1).split("/"));
  }

  public static Path toHost(Path root, String logical) {
    Path canonicalRoot = root.toAbsolutePath().normalize();
    Path output = canonicalRoot;
    for (String segment : segments(logical)) output = output.resolve(segment);
    output = output.normalize();
    if (!output.startsWith(canonicalRoot)) {
      throw new Error("outside-root", "logical path escapes above the mounted root");
    }
    return output;
  }

  public static String fromHost(Path root, Path host) {
    Path canonicalRoot = root.toAbsolutePath().normalize();
    Path canonicalHost = host.toAbsolutePath().normalize();
    if (!canonicalHost.startsWith(canonicalRoot)) {
      throw new Error("outside-root", "host path is outside the mounted root");
    }
    Path relative = canonicalRoot.relativize(canonicalHost);
    ArrayList<String> segments = new ArrayList<>();
    for (Path segment : relative) segments.add(segment.toString());
    return segments.isEmpty() ? "/" : normalise("/" + String.join("/", segments));
  }

  public static void validateFragment(String value, String label) {
    if (value == null) throw invalid(label + " must be a string");
    if (value.indexOf('/') >= 0 || value.indexOf('\\') >= 0 || value.indexOf('\0') >= 0) {
      throw invalid(label + " must be one logical path fragment");
    }
    if (".".equals(value) || "..".equals(value) || isDrivePrefix(value)) {
      throw invalid(label + " must not contain host or traversal syntax");
    }
  }

  private static boolean isDrivePrefix(String value) {
    return value.length() >= 2
        && Character.isLetter(value.charAt(0))
        && value.charAt(1) == ':';
  }

  private static Error invalid(String message) {
    return new Error("invalid-path", message);
  }
}
