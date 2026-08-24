package hara.spec;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;

/** Resolves the workspace's canonical hara-specs-registry checkout for tests. */
public final class SpecRegistry {
  private static final String PROPERTY = "hara.specs.registry";
  private static final String REGISTRY_ENV = "HARA_SPECS_REGISTRY";
  private static final String WORKSPACE_ENV = "HARA_WORKSPACE_ROOT";

  private SpecRegistry() {}

  /** Returns the validated absolute registry root. */
  public static Path root() {
    String configured = System.getProperty(PROPERTY);
    if (configured != null && !configured.isBlank()) {
      return validate(Path.of(configured), "system property -D" + PROPERTY);
    }

    configured = System.getenv(REGISTRY_ENV);
    if (configured != null && !configured.isBlank()) {
      return validate(Path.of(configured), REGISTRY_ENV);
    }

    String workspace = System.getenv(WORKSPACE_ENV);
    if (workspace != null && !workspace.isBlank()) {
      return validate(Path.of(workspace).resolve("technology/hara-specs-registry"), WORKSPACE_ENV);
    }

    Path start = Path.of(System.getProperty("user.dir", ".")).toAbsolutePath().normalize();
    for (Path cursor = start; cursor != null; cursor = cursor.getParent()) {
      for (Path candidate :
          List.of(cursor.resolve("hara-specs-registry"), cursor.resolve("technology/hara-specs-registry"))) {
        if (isRegistry(candidate)) return candidate.toAbsolutePath().normalize();
      }
    }

    throw new IllegalStateException(
        "Cannot locate hara-specs-registry. Set "
            + REGISTRY_ENV
            + " or "
            + WORKSPACE_ENV
            + ", or pass -D"
            + PROPERTY
            + "=/absolute/path/to/hara-specs-registry; working directory was "
            + start);
  }

  /** Returns whether the canonical registry can be found without throwing. */
  public static boolean available() {
    try {
      root();
      return true;
    } catch (IllegalStateException ex) {
      return false;
    }
  }

  /** Resolves a registry-relative file and fails with a useful diagnostic if it is absent. */
  public static Path require(String relative) {
    Path path = resolve(relative);
    if (!Files.isRegularFile(path)) {
      throw new IllegalStateException("Missing specification file: " + path);
    }
    return path;
  }

  /** Resolves a registry-relative file without requiring it to exist. */
  public static Path resolve(String relative) {
    Path requested = Path.of(relative);
    if (requested.isAbsolute() || relative.startsWith("..")) {
      throw new IllegalArgumentException("Specification path must be registry-relative: " + relative);
    }
    Path root = root();
    Path path = root.resolve(requested).normalize();
    if (!path.startsWith(root)) {
      throw new IllegalArgumentException("Specification path escapes the registry: " + relative);
    }
    return path;
  }

  private static Path validate(Path candidate, String source) {
    Path root = candidate.toAbsolutePath().normalize();
    if (!isRegistry(root)) {
      throw new IllegalStateException(
          "Configured hara-specs-registry from " + source + " is not valid: " + root);
    }
    return root;
  }

  private static boolean isRegistry(Path candidate) {
    return Files.isDirectory(candidate)
        && (Files.isRegularFile(candidate.resolve("spec-manifest.json"))
            || Files.isRegularFile(candidate.resolve("registry-index.json")));
  }
}
