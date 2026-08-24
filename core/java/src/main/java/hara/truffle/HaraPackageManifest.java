package hara.truffle;

import hara.kernel.base.Parser;
import hara.lang.data.Keyword;
import hara.lang.data.Symbol;
import hara.lang.protocol.IMapType;
import hara.lang.protocol.ISetType;
import java.io.IOException;
import java.io.InputStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;
import java.util.ArrayList;
import java.util.HexFormat;
import java.util.Iterator;
import java.util.LinkedHashMap;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Map;
import java.util.Set;

/** Verified view of the generated package.edn index used by Wasm imports and JVM flavors. */
final class HaraPackageManifest {
  private static final String FORMAT = "0.0.0-alpha";
  private static final long MAX_MANIFEST_BYTES = 4 * 1024 * 1024;
  private static final long MAX_FILE_BYTES = 64 * 1024 * 1024;

  private final String identity;
  private final String version;
  private final String repository;
  private final String commit;
  private final Map<String, FileEntry> files;
  private final Map<String, WasmImport> wasmImports;
  private final Map<String, JvmFlavor> flavors;

  private HaraPackageManifest(
      String identity,
      String version,
      String repository,
      String commit,
      Map<String, FileEntry> files,
      Map<String, WasmImport> wasmImports,
      Map<String, JvmFlavor> flavors) {
    this.identity = identity;
    this.version = version;
    this.repository = repository;
    this.commit = commit;
    this.files = Map.copyOf(files);
    this.wasmImports = Map.copyOf(wasmImports);
    this.flavors = Map.copyOf(flavors);
  }

  static HaraPackageManifest read(Path root) {
    Path source = root.resolve("package.edn");
    try {
      if (!Files.isRegularFile(source)) return null;
      if (Files.size(source) > MAX_MANIFEST_BYTES) {
        throw new HaraException("package/manifest-too-large: " + source);
      }
      return parse(Files.readString(source), source.toString());
    } catch (IOException error) {
      throw new HaraException("package/manifest-unavailable: " + source + " (" + error + ")");
    }
  }

  static HaraPackageManifest parse(String source, String origin) {
    final Object value;
    try {
      value = Parser.LispReader.readString(source, null);
    } catch (RuntimeException error) {
      throw new HaraException("package/invalid-manifest: " + origin + " (" + error + ")");
    }
    if (!(value instanceof IMapType<?, ?> root)) {
      throw invalid(origin, "manifest must be a map");
    }
    String format = requireString(root, "harp/format", origin);
    if (!FORMAT.equals(format)) throw invalid(origin, "unsupported :harp/format " + format);
    Object packageValue = lookup(root, "package");
    IMapType<?, ?> packageMap = requireMap(packageValue, "package", origin);
    String identity = requireString(packageMap, "identity", origin);
    String version = requireString(packageMap, "version", origin);
    String repository = null;
    String commit = null;
    Object provenanceValue = lookup(packageMap, "provenance");
    if (provenanceValue != null) {
      IMapType<?, ?> provenance = requireMap(provenanceValue, "provenance", origin);
      repository = requireString(provenance, "repository", origin);
      commit = requireString(provenance, "commit", origin);
    }
    IMapType<?, ?> fileMap = requireMap(lookup(root, "files"), "files", origin);
    Map<String, FileEntry> files = new LinkedHashMap<>();
    for (Map.Entry<?, ?> entry : entries(fileMap)) {
      String path = entry.getKey() instanceof String string
          ? string
          : entry.getKey() instanceof Keyword keyword ? keywordName(keyword) : "";
      if (!safeRelative(path)) throw invalid(origin, ":files keys must be safe relative strings");
      IMapType<?, ?> declaration = requireMap(entry.getValue(), "file " + path, origin);
      String sha256 = requireString(declaration, "sha256", origin);
      long size = requireNonNegativeLong(declaration, "size", origin);
      if (files.put(path, new FileEntry(sha256, size)) != null) {
        throw invalid(origin, "duplicate file " + path);
      }
    }
    Map<String, WasmImport> imports = new LinkedHashMap<>();
    Object wasmValue = lookup(root, "wasm-imports");
    if (wasmValue != null) {
      IMapType<?, ?> importMap = requireMap(wasmValue, "wasm-imports", origin);
      for (Map.Entry<?, ?> entry : entries(importMap)) {
        String logical = identifier(entry.getKey(), origin, "Wasm import name");
        if (imports.put(logical, parseImport(entry.getValue(), files, origin)) != null) {
          throw invalid(origin, "duplicate Wasm import " + logical);
        }
      }
    }
    Map<String, JvmFlavor> flavors = new LinkedHashMap<>();
    Object flavorsValue = lookup(root, "flavors");
    if (flavorsValue != null) {
      IMapType<?, ?> flavorMap = requireMap(flavorsValue, "flavors", origin);
      for (Map.Entry<?, ?> entry : entries(flavorMap)) {
        String flavor = identifier(entry.getKey(), origin, "flavor name");
        if ("wasm".equals(flavor)) throw invalid(origin, ":wasm is reserved and is not a host flavor");
        if (flavors.put(flavor, parseFlavor(entry.getValue(), files, origin)) != null) {
          throw invalid(origin, "duplicate flavor " + flavor);
        }
      }
    }
    if (!flavors.isEmpty() && (repository == null || commit == null)) {
      throw invalid(origin, "packages with host flavors require :package :provenance");
    }
    if (!flavors.isEmpty()
        && !commit.matches("(?:[0-9a-f]{40}|[0-9a-f]{64})")) {
      throw invalid(origin, ":package :provenance :commit must be a hexadecimal digest");
    }
    return new HaraPackageManifest(identity, version, repository, commit, files, imports, flavors);
  }

  String identity() {
    return identity;
  }

  String version() {
    return version;
  }

  JvmFlavor jvmFlavor() {
    return flavors.get("jvm");
  }

  List<Path> jvmDependencyFiles(Path root) {
    ArrayList<Path> result = new ArrayList<>();
    for (String path : files.keySet()) {
      if (path.startsWith("artifacts/jvm/dependencies/") && path.endsWith(".jar")) {
        result.add(root.resolve(path).normalize());
      }
    }
    return List.copyOf(result);
  }

  WasmImport wasmImport(String logical) {
    return wasmImports.get(logical);
  }

  /** Verifies every indexed file and returns the selected module path. */
  Path verifyImport(Path root, String logical) {
    WasmImport selected = wasmImports.get(logical);
    if (selected == null) throw new HaraException("package/missing-wasm-import: " + logical);
    if (!"wasm".equals(selected.artifactType)) {
      throw new HaraException("package/artifact-type-mismatch: expected :wasm, got :" + selected.artifactType);
    }
    if (!"core.v1".equals(selected.abi)) {
      throw new HaraException("package/abi-mismatch: direct Wasm import requires core.v1: " + logical);
    }
    if (!selected.requiredCapabilities.isEmpty()) {
      throw new HaraException("package/capability-denied: direct Wasm imports cannot request capabilities");
    }
    if (!selected.hostCalls.isEmpty()) {
      throw new HaraException("package/host-call-denied: direct Wasm imports cannot request host calls");
    }
    for (Map.Entry<String, FileEntry> entry : files.entrySet()) verifyFile(root, entry.getKey(), entry.getValue());
    return root.resolve(selected.path).normalize();
  }

  Path verifyJvmFlavor(Path root) {
    JvmFlavor selected = jvmFlavor();
    if (selected == null) throw new HaraException("package/missing-flavor: :jvm");
    if (!"jar".equals(selected.artifactType())) {
      throw new HaraException("package/artifact-type-mismatch: expected :jar");
    }
    if (!"java-21".equals(selected.target())) {
      throw new HaraException("package/target-mismatch: JVM flavor requires java-21");
    }
    if (!JvmPackageProvider.ABI.equals(selected.abi())) {
      throw new HaraException("package/abi-mismatch: " + selected.abi());
    }
    for (Map.Entry<String, FileEntry> entry : files.entrySet()) verifyFile(root, entry.getKey(), entry.getValue());
    return root.resolve(selected.path()).normalize();
  }

  void verifyFiles(Path root) {
    for (Map.Entry<String, FileEntry> entry : files.entrySet()) {
      verifyFile(root, entry.getKey(), entry.getValue());
    }
  }

  void verifyArchiveEntries(Path root) {
    try (var paths = Files.walk(root)) {
      Set<String> actual =
          paths
              .filter(Files::isRegularFile)
              .map(path -> root.relativize(path).toString().replace('\\', '/'))
              .collect(java.util.stream.Collectors.toSet());
      Set<String> expected = new java.util.HashSet<>(files.keySet());
      expected.add("package.edn");
      if (!actual.equals(expected)) {
        throw new HaraException("package/archive-files-mismatch: declared files differ from archive");
      }
    } catch (IOException error) {
      throw new HaraException("package/archive-unavailable: " + root + " (" + error + ")");
    }
  }

  private void verifyFile(Path root, String relative, FileEntry expected) {
    Path path = root.resolve(relative).normalize();
    if (!path.startsWith(root.normalize()) || !Files.isRegularFile(path)) {
      throw new HaraException("package/missing-artifact: " + relative);
    }
    try {
      long size = Files.size(path);
      if (size != expected.size) {
        throw new HaraException("package/size-mismatch: " + relative);
      }
      if (size > MAX_FILE_BYTES) throw new HaraException("package/artifact-too-large: " + relative);
      String actual = digest(path);
      if (!expected.sha256.equals(actual)) {
        throw new HaraException("package/digest-mismatch: " + relative);
      }
    } catch (IOException error) {
      throw new HaraException("package/missing-artifact: " + relative + " (" + error + ")");
    }
  }

  static List<Path> installedRoots() {
    String configured = System.getProperty("hara.dist.home", "");
    if (configured.isBlank()) configured = System.getenv().getOrDefault("HARA_DIST_HOME", "");
    Path dist = configured.isBlank()
        ? Path.of(System.getProperty("user.home"), ".hara", "dist")
        : Path.of(configured);
    Path roots = dist.resolve("roots/sha256");
    if (!Files.isDirectory(roots)) return List.of();
    try (var entries = Files.list(roots)) {
      return entries.filter(Files::isDirectory).sorted().toList();
    } catch (IOException error) {
      throw new HaraException("package/roots-unavailable: " + roots + " (" + error + ")");
    }
  }

  private static WasmImport parseImport(Object value, Map<String, FileEntry> files, String origin) {
    IMapType<?, ?> variant = requireMap(value, "Wasm import", origin);
    IMapType<?, ?> artifact = requireMap(lookup(variant, "variant/artifact"), "variant/artifact", origin);
    String artifactType = requireKeyword(artifact, "artifact/type", origin);
    String path = requireString(artifact, "artifact/path", origin);
    if (!safeRelative(path) || !path.endsWith(".wasm")) throw invalid(origin, "artifact path must be a relative .wasm file");
    String sha256 = requireString(artifact, "artifact/sha256", origin);
    String target = requireString(artifact, "artifact/target", origin);
    String abi = requireString(artifact, "artifact/abi", origin);
    String entryPoint = requireString(artifact, "artifact/entry-point", origin);
    FileEntry file = files.get(path);
    if (file == null) throw invalid(origin, "artifact path is not declared in :files: " + path);
    if (!file.sha256.equals(sha256)) throw invalid(origin, "artifact digest differs from :files: " + path);
    Set<String> capabilities = identifiers(lookup(variant, "variant/required-capabilities"), origin, "capabilities");
    Set<String> hostCalls = identifiers(lookup(variant, "variant/host-calls"), origin, "host calls");
    Set<String> exports = identifiers(lookup(variant, "variant/exports"), origin, "exports");
    return new WasmImport(artifactType, path, sha256, target, abi, entryPoint, capabilities, hostCalls, exports);
  }

  private static JvmFlavor parseFlavor(
      Object value, Map<String, FileEntry> files, String origin) {
    IMapType<?, ?> variant = requireMap(value, "JVM flavor", origin);
    IMapType<?, ?> artifact = requireMap(lookup(variant, "variant/artifact"), "variant/artifact", origin);
    String artifactType = requireKeyword(artifact, "artifact/type", origin);
    String path = requireString(artifact, "artifact/path", origin);
    if (!safeRelative(path) || !path.endsWith(".jar")) {
      throw invalid(origin, "JVM flavor artifact must be a relative .jar file");
    }
    String sha256 = requireString(artifact, "artifact/sha256", origin);
    String target = requireString(artifact, "artifact/target", origin);
    String abi = requireString(artifact, "artifact/abi", origin);
    String entryPoint = requireString(artifact, "artifact/entry-point", origin);
    FileEntry file = files.get(path);
    if (file == null) throw invalid(origin, "artifact path is not declared in :files: " + path);
    if (!file.sha256.equals(sha256)) throw invalid(origin, "artifact digest differs from :files: " + path);
    Object capabilityValue = lookup(variant, "variant/required-capabilities");
    if (capabilityValue == null) {
      throw invalid(origin, "JVM flavor requires :variant/required-capabilities");
    }
    Set<String> capabilities = identifiers(capabilityValue, origin, "capabilities");
    Set<String> hostCalls = identifiers(lookup(variant, "variant/host-calls"), origin, "host calls");
    Set<String> exports = identifiers(lookup(variant, "variant/exports"), origin, "exports");
    Map<String, String> dependencies = mavenDependencies(lookup(variant, "variant/dependencies"), origin);
    return new JvmFlavor(
        artifactType,
        path,
        sha256,
        target,
        abi,
        entryPoint,
        capabilities,
        hostCalls,
        exports,
        dependencies);
  }

  private static Map<String, String> mavenDependencies(Object value, String origin) {
    if (value == null) return Map.of();
    IMapType<?, ?> dependencyGroups = requireMap(value, "variant/dependencies", origin);
    Object mavenValue = lookup(dependencyGroups, "maven");
    if (mavenValue == null) return Map.of();
    IMapType<?, ?> maven = requireMap(mavenValue, "variant/dependencies :maven", origin);
    Map<String, String> result = new LinkedHashMap<>();
    for (Map.Entry<?, ?> entry : entries(maven)) {
      String coordinate;
      if (entry.getKey() instanceof Symbol symbol) coordinate = symbol.display();
      else if (entry.getKey() instanceof String string && !string.isBlank()) coordinate = string;
      else throw invalid(origin, "Maven dependency coordinates must be symbols or strings");
      IMapType<?, ?> declaration = requireMap(entry.getValue(), "Maven dependency", origin);
      String version = requireString(declaration, "version", origin);
      if (result.put(coordinate, version) != null) {
        throw invalid(origin, "duplicate Maven dependency " + coordinate);
      }
    }
    return Map.copyOf(result);
  }

  private static Set<String> identifiers(Object value, String origin, String field) {
    if (value == null) return Set.of();
    if (!(value instanceof ISetType<?> set)) throw invalid(origin, field + " must be an EDN set");
    LinkedHashSet<String> result = new LinkedHashSet<>();
    for (Object item : set) result.add(identifier(item, origin, field));
    return Set.copyOf(result);
  }

  private static String identifier(Object value, String origin, String field) {
    if (value instanceof Keyword keyword) return keyword.getName();
    if (value instanceof String string && !string.isBlank()) return string;
    throw invalid(origin, field + " must contain identifiers");
  }

  private static boolean safeRelative(String value) {
    if (value.isBlank() || value.startsWith("/") || value.contains("\\") || value.contains(":")) return false;
    for (String part : value.split("/", -1)) if (part.isBlank() || ".".equals(part) || "..".equals(part)) return false;
    return true;
  }

  private static String digest(Path path) throws IOException {
    try {
      MessageDigest digest = MessageDigest.getInstance("SHA-256");
      try (InputStream input = Files.newInputStream(path)) {
        byte[] buffer = new byte[64 * 1024];
        for (int count; (count = input.read(buffer)) >= 0; ) if (count > 0) digest.update(buffer, 0, count);
      }
      return "sha256:" + HexFormat.of().formatHex(digest.digest());
    } catch (NoSuchAlgorithmException impossible) {
      throw new IllegalStateException(impossible);
    }
  }

  private static List<Map.Entry<?, ?>> entries(IMapType<?, ?> map) {
    List<Map.Entry<?, ?>> result = new ArrayList<>();
    Iterator<?> iterator = map.iterator();
    while (iterator.hasNext()) result.add((Map.Entry<?, ?>) iterator.next());
    return result;
  }

  private static Object lookup(IMapType<?, ?> map, String field) {
    for (Map.Entry<?, ?> entry : entries(map)) {
      Object key = entry.getKey();
      if (key instanceof Keyword keyword
          && (field.equals(keyword.getName()) || field.equals(keywordName(keyword)))) {
        return entry.getValue();
      }
      if (key instanceof String string && field.equals(string)) return entry.getValue();
    }
    return null;
  }

  private static String keywordName(Keyword keyword) {
    return keyword.getNamespace() == null || keyword.getNamespace().isEmpty()
        ? keyword.getName()
        : keyword.getNamespace() + "/" + keyword.getName();
  }

  private static IMapType<?, ?> requireMap(Object value, String field, String origin) {
    if (!(value instanceof IMapType<?, ?> map)) throw invalid(origin, field + " must be a map");
    return map;
  }

  private static String requireString(IMapType<?, ?> map, String field, String origin) {
    Object value = lookup(map, field);
    if (!(value instanceof String string) || string.isBlank()) {
      throw invalid(origin, field + " must be a non-empty string");
    }
    return string;
  }

  private static String requireKeyword(IMapType<?, ?> map, String field, String origin) {
    Object value = lookup(map, field);
    if (!(value instanceof Keyword keyword)) throw invalid(origin, field + " must be a keyword");
    return keyword.getName();
  }

  private static long requireNonNegativeLong(IMapType<?, ?> map, String field, String origin) {
    Object value = lookup(map, field);
    if (!(value instanceof Number number) || number.longValue() < 0) throw invalid(origin, field + " must be non-negative");
    return number.longValue();
  }

  private static HaraException invalid(String origin, String detail) {
    return new HaraException("package/invalid-manifest: " + origin + " (" + detail + ")");
  }

  record FileEntry(String sha256, long size) {}

  record JvmFlavor(
      String artifactType,
      String path,
      String sha256,
      String target,
      String abi,
      String entryPoint,
      Set<String> requiredCapabilities,
      Set<String> hostCalls,
      Set<String> exports,
      Map<String, String> mavenDependencies) {
    JvmFlavor {
      requiredCapabilities = Set.copyOf(requiredCapabilities);
      hostCalls = Set.copyOf(hostCalls);
      exports = Set.copyOf(exports);
      mavenDependencies = Map.copyOf(mavenDependencies);
    }
  }

  record WasmImport(
      String artifactType,
      String path,
      String sha256,
      String target,
      String abi,
      String entryPoint,
      Set<String> requiredCapabilities,
      Set<String> hostCalls,
      Set<String> exports) {}
}
