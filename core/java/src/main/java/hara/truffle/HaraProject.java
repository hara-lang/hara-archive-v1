package hara.truffle;

import hara.kernel.base.Parser;
import hara.lang.base.G;
import hara.lang.data.Keyword;
import hara.lang.data.List;
import hara.lang.data.Symbol;
import hara.lang.protocol.ILinearType;
import hara.lang.protocol.IMapType;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Collections;
import java.util.LinkedHashMap;
import java.util.LinkedHashSet;
import java.util.Iterator;
import java.util.Map;
import java.util.Set;

/** Discovers project.edn (or legacy project.hal) and resolves namespace paths. */
final class HaraProject {
  private static final String PROJECT_FILE = "project.edn";
  private static final String LEGACY_PROJECT_FILE = "project.hal";

  private final Path root;
  private final Path descriptor;
  private final Symbol name;
  private final String version;
  private final Symbol main;
  private final java.util.List<Path> sourcePaths;
  private final java.util.List<Path> testPaths;
  private final java.util.List<Path> extensionPaths;
  private final Map<String, String> haraDependencies;
  private final java.util.List<JvmDependency> jvmDependencies;
  private final java.util.List<Path> jvmSourcePaths;
  private final Path jvmTargetPath;
  private final String jvmEntryPoint;
  private final Set<String> capabilities;

  record JvmDependency(String id, String version) {
    String coordinate() {
      return id.replace('/', ':') + ":" + version;
    }
  }

  private HaraProject(
      Path root,
      Path descriptor,
      Symbol name,
      String version,
      Symbol main,
      java.util.List<Path> sourcePaths,
      java.util.List<Path> testPaths,
      java.util.List<Path> extensionPaths,
      Map<String, String> haraDependencies,
      java.util.List<JvmDependency> jvmDependencies,
      java.util.List<Path> jvmSourcePaths,
      Path jvmTargetPath,
      String jvmEntryPoint,
      Set<String> capabilities) {
    this.root = root;
    this.descriptor = descriptor;
    this.name = name;
    this.version = version;
    this.main = main;
    this.sourcePaths = java.util.List.copyOf(sourcePaths);
    this.testPaths = java.util.List.copyOf(testPaths);
    this.extensionPaths = java.util.List.copyOf(extensionPaths);
    this.haraDependencies = Map.copyOf(haraDependencies);
    this.jvmDependencies = java.util.List.copyOf(jvmDependencies);
    this.jvmSourcePaths = java.util.List.copyOf(jvmSourcePaths);
    this.jvmTargetPath = jvmTargetPath;
    this.jvmEntryPoint = jvmEntryPoint;
    this.capabilities = Set.copyOf(capabilities);
  }

  static HaraProject discover(Path start) {
    Path current = start.toAbsolutePath().normalize();
    while (current != null) {
      Path descriptor = current.resolve(PROJECT_FILE);
      if (Files.isRegularFile(descriptor)) return read(descriptor);
      descriptor = current.resolve(LEGACY_PROJECT_FILE);
      if (Files.isRegularFile(descriptor)) return read(descriptor);
      current = current.getParent();
    }
    return null;
  }

  static HaraProject read(Path descriptor) {
    try {
      Object form =
          Parser.LispReader.readString(
              Files.readString(descriptor, StandardCharsets.UTF_8), null);
      if (PROJECT_FILE.equals(descriptor.getFileName().toString())) {
        if (!(form instanceof IMapType<?, ?> options))
          throw new HaraException("project.edn must be an EDN map");
        Object projectId = lookup(options, "project/id");
        Symbol projectName =
            projectId instanceof Symbol symbol
                ? symbol
                : projectId instanceof String string ? Symbol.create(string) : null;
        if (projectName == null)
          throw new HaraException("project.edn :project/id must be a string or symbol");
        rejectLegacyRuntimeKeys(options, PROJECT_FILE);
        Path root = descriptor.toAbsolutePath().normalize().getParent();
        java.util.List<Path> sharedSourcePaths =
            paths(
                root,
                lookup(options, "project/source-paths"),
                "project/source-paths",
                java.util.List.of("src"),
                PROJECT_FILE);
        java.util.List<Path> sharedTestPaths =
            paths(
                root,
                lookup(options, "project/test-paths"),
                "project/test-paths",
                java.util.List.of("test"),
                PROJECT_FILE);
        java.util.List<Path> sharedExtensionPaths =
            paths(
                root,
                lookup(options, "project/extension-paths"),
                "project/extension-paths",
                java.util.List.of("extensions"),
                PROJECT_FILE);
        RuntimeProfile runtime = runtimeProfile(root, options, "jvm", PROJECT_FILE);
        Map<String, String> sharedHara =
            haraDependencies(lookup(options, "project/dependencies"), PROJECT_FILE);
        Map<String, String> effectiveHara =
            mergeHaraDependencies(sharedHara, runtime.haraDependencies(), "jvm");
        String jvmEntryPoint = jvmEntryPoint(options, PROJECT_FILE);
        return new HaraProject(
            root,
            descriptor,
            projectName,
            lookup(options, "project/version") instanceof String value ? value : null,
            lookup(options, "project/main") instanceof Symbol value ? value : null,
            mergePaths(sharedSourcePaths, runtime.sourcePaths()),
            mergePaths(sharedTestPaths, runtime.testPaths()),
            mergePaths(sharedExtensionPaths, runtime.extensionPaths()),
            effectiveHara,
            runtime.mavenDependencies(),
            runtime.nativeSourcePaths(),
            runtime.targetPath() == null
                ? root.resolve("target/jvm/classes")
                : runtime.targetPath(),
            jvmEntryPoint,
            capabilities(lookup(options, "project/capabilities"), PROJECT_FILE));
      }
      if (!(form instanceof List<?> list)
          || list.count() != 3
          || !Symbol.create("defproject").equals(list.nth(0))
          || !(list.nth(1) instanceof Symbol projectName)
          || projectName.getNamespace() != null
          || !(list.nth(2) instanceof IMapType<?, ?> options)) {
        throw new HaraException(
            "project.hal expects (defproject unqualified-name options-map)");
      }
      Path root = descriptor.toAbsolutePath().normalize().getParent();
      return new HaraProject(
          root,
          descriptor,
          projectName,
          null,
          null,
          paths(
              root,
              lookup(options, "source-paths"),
              "source-paths",
              java.util.List.of("src"),
              LEGACY_PROJECT_FILE),
          paths(
              root,
              lookup(options, "test-paths"),
              "test-paths",
              java.util.List.of("test"),
              LEGACY_PROJECT_FILE),
          java.util.List.of(root.resolve("extensions")),
          Map.of(),
          java.util.List.of(),
          java.util.List.of(),
          root.resolve("target/classes"),
          null,
          Set.of());
    } catch (IOException error) {
      throw new HaraException(
          "Unable to read project descriptor " + descriptor + ": " + error.getMessage());
    }
  }

  Path resolve(String namespace, boolean includeTests) {
    String relative = namespace.replace('.', '/').replace('-', '_') + ".hal";
    for (Path sourcePath : sourcePaths) {
      Path candidate = sourcePath.resolve(relative).normalize();
      if (candidate.startsWith(root) && Files.isRegularFile(candidate)) return candidate;
    }
    if (includeTests) {
      for (Path testPath : testPaths) {
        Path candidate = testPath.resolve(relative).normalize();
        if (candidate.startsWith(root) && Files.isRegularFile(candidate)) return candidate;
      }
    }
    return null;
  }

  Symbol name() {
    return name;
  }

  Object extensionDeclaration(String namespace) {
    Object extensions = projectField("project/extensions");
    if (!(extensions instanceof IMapType<?, ?> map)) return null;
    Iterator<?> iterator = map.iterator();
    while (iterator.hasNext()) {
      Map.Entry<?, ?> entry = (Map.Entry<?, ?>) iterator.next();
      if (entry.getKey() instanceof Symbol symbol
          && namespace.equals(symbol.display())) return entry.getValue();
    }
    return null;
  }

  String extensionsEdn() {
    Object extensions = projectField("project/extensions");
    return extensions instanceof IMapType<?, ?> ? G.display(extensions) : "{}";
  }

  Path extensionRoot(String namespace) {
    Object declaration = extensionDeclaration(namespace);
    if (!(declaration instanceof IMapType<?, ?> map)) return root;
    Object configured = lookup(map, "root");
    if (configured == null) return root;
    if (!(configured instanceof String relative))
      throw new HaraException("project.edn extension :root must be a string");
    Path resolved = root.resolve(relative).normalize();
    if (Path.of(relative).isAbsolute() || !resolved.startsWith(root))
      throw new HaraException("project.edn extension :root escapes the project");
    return resolved;
  }

  String extensionManifestSource(String namespace) {
    Object declaration = extensionDeclaration(namespace);
    if (!(declaration instanceof IMapType<?, ?> map)) return null;
    StringBuilder source = new StringBuilder("{:namespace ")
        .append(G.display(namespace))
        .append(" :identity ").append(G.display(name.display()))
        .append(" :version ").append(G.display(version));
    Iterator<?> iterator = map.iterator();
    while (iterator.hasNext()) {
      Map.Entry<?, ?> entry = (Map.Entry<?, ?>) iterator.next();
      if (entry.getKey() instanceof Keyword keyword && "root".equals(keyword.getName())) continue;
      source.append(' ').append(G.display(entry.getKey()))
          .append(' ').append(G.display(entry.getValue()));
    }
    return source.append('}').toString();
  }

  private Object projectField(String key) {
    if (!PROJECT_FILE.equals(descriptor.getFileName().toString())) return null;
    try {
      Object form = Parser.LispReader.readString(
          Files.readString(descriptor, StandardCharsets.UTF_8), null);
      return form instanceof IMapType<?, ?> map ? lookup(map, key) : null;
    } catch (IOException error) {
      throw new HaraException("Unable to read project descriptor " + descriptor + ": " + error.getMessage());
    }
  }

  Path descriptor() {
    return descriptor;
  }

  String version() {
    return version;
  }

  Symbol main() {
    return main;
  }

  void validateCliProject() {
    if (!PROJECT_FILE.equals(descriptor.getFileName().toString()))
      throw new HaraException("project CLI requires project.edn");
    try {
      Object form = Parser.LispReader.readString(Files.readString(descriptor, StandardCharsets.UTF_8), null);
      if (!(form instanceof IMapType<?, ?> options)
          || !(lookup(options, "hara/type") instanceof Keyword type)
          || !"project".equals(type.getName()))
        throw new HaraException("project.edn :hara/type must be :project");
      for (String key :
          java.util.List.of(
              "hara/version",
              "project/version",
              "project/source-paths",
              "project/test-paths",
              "project/extension-paths",
              "project/capabilities")) {
        if (lookup(options, key) == null) throw new HaraException("project.edn missing required key :" + key);
      }
      if (!(lookup(options, "hara/version") instanceof String))
        throw new HaraException("project.edn :hara/version must be a string");
      if (!(lookup(options, "project/version") instanceof String version)
          || !version.matches(
              "^(0|[1-9][0-9]*)\\.(0|[1-9][0-9]*)\\.(0|[1-9][0-9]*)(?:[-+][0-9A-Za-z.-]+)?$"))
        throw new HaraException("project.edn :project/version is not SemVer");
      rejectLegacyRuntimeKeys(options, PROJECT_FILE);
      RuntimeProfile runtime = runtimeProfile(root, options, "jvm", PROJECT_FILE);
      mergeHaraDependencies(
          haraDependencies(lookup(options, "project/dependencies"), PROJECT_FILE),
          runtime.haraDependencies(),
          "jvm");
      Object dependencies = lookup(options, "project/dependencies");
      if (dependencies != null && !(dependencies instanceof IMapType<?, ?>))
        throw new HaraException("project.edn :project/dependencies must be a map");
      paths(
          root,
          lookup(options, "project/artifact-paths"),
          "project/artifact-paths",
          java.util.List.of(),
          PROJECT_FILE);
    } catch (IOException error) {
      throw new HaraException("Unable to read project descriptor " + descriptor + ": " + error.getMessage());
    }
  }

  Path mainFile() {
    if (main == null) throw new HaraException("project.edn is missing :project/main");
    Path source = resolve(main.display(), false);
    if (source == null)
      throw new HaraException("cannot find :project/main " + main.display() + " in :project/source-paths");
    return source;
  }

  Path root() {
    return root;
  }

  java.util.List<Path> sourcePaths() {
    return sourcePaths;
  }

  java.util.List<Path> testPaths() {
    return testPaths;
  }

  Map<String, String> haraDependencies() {
    return haraDependencies;
  }

  java.util.List<JvmDependency> jvmDependencies() {
    return jvmDependencies;
  }

  java.util.List<Path> jvmSourcePaths() {
    return jvmSourcePaths;
  }

  Path jvmTargetPath() {
    return jvmTargetPath;
  }

  String jvmEntryPoint() {
    return jvmEntryPoint;
  }

  boolean hasCapability(String capability) {
    return capabilities.contains(capability);
  }

  java.util.List<Path> extensionRoots() {
    return extensionPaths;
  }

  Path extensionRoot() {
    return extensionPaths.isEmpty() ? root.resolve("extensions") : extensionPaths.get(0);
  }


  private record RuntimeProfile(
      java.util.List<Path> sourcePaths,
      java.util.List<Path> testPaths,
      java.util.List<Path> extensionPaths,
      java.util.List<Path> nativeSourcePaths,
      Path targetPath,
      Map<String, String> haraDependencies,
      java.util.List<JvmDependency> mavenDependencies) {}

  private static RuntimeProfile runtimeProfile(
      Path root, IMapType<?, ?> project, String runtime, String descriptor) {
    Object declaredProfiles = lookup(project, "project/runtime-profiles");
    if (declaredProfiles == null) {
      return new RuntimeProfile(
          java.util.List.of(),
          java.util.List.of(),
          java.util.List.of(),
          java.util.List.of(),
          null,
          Map.of(),
          java.util.List.of());
    }
    if (!(declaredProfiles instanceof IMapType<?, ?> profiles)) {
      throw new HaraException(descriptor + " :project/runtime-profiles must be a map");
    }
    Object declaredProfile = lookup(profiles, runtime);
    if (declaredProfile == null) {
      return new RuntimeProfile(
          java.util.List.of(),
          java.util.List.of(),
          java.util.List.of(),
          java.util.List.of(),
          null,
          Map.of(),
          java.util.List.of());
    }
    if (!(declaredProfile instanceof IMapType<?, ?> profile)) {
      throw new HaraException(
          descriptor + " :project/runtime-profiles :" + runtime + " must be a map");
    }
    Object declaredDependencies = lookup(profile, "runtime/dependencies");
    IMapType<?, ?> dependencyGroups;
    if (declaredDependencies == null) {
      dependencyGroups = null;
    } else if (declaredDependencies instanceof IMapType<?, ?> map) {
      dependencyGroups = map;
    } else {
      throw new HaraException(
          descriptor + " :runtime/dependencies for :" + runtime + " must be a map");
    }
    Object target = lookup(profile, "runtime/target-path");
    return new RuntimeProfile(
        paths(
            root,
            lookup(profile, "runtime/source-paths"),
            "runtime/source-paths",
            java.util.List.of(),
            descriptor),
        paths(
            root,
            lookup(profile, "runtime/test-paths"),
            "runtime/test-paths",
            java.util.List.of(),
            descriptor),
        paths(
            root,
            lookup(profile, "runtime/extension-paths"),
            "runtime/extension-paths",
            java.util.List.of(),
            descriptor),
        paths(
            root,
            lookup(profile, "runtime/native-source-paths"),
            "runtime/native-source-paths",
            java.util.List.of(),
            descriptor),
        target == null
            ? null
            : path(root, target, "runtime/target-path", null, descriptor),
        haraDependencies(
            dependencyGroups == null ? null : lookup(dependencyGroups, "hara"), descriptor),
        mavenDependencies(
            dependencyGroups == null ? null : lookup(dependencyGroups, "maven"), descriptor));
  }

  private static String jvmEntryPoint(IMapType<?, ?> project, String descriptor) {
    Object packageValue = lookup(project, "project/package");
    if (packageValue == null) return null;
    if (!(packageValue instanceof IMapType<?, ?> packageOptions)) {
      throw new HaraException(descriptor + " :project/package must be a map");
    }
    Object entries = lookup(packageOptions, "entry-points");
    if (entries == null) return null;
    if (!(entries instanceof ILinearType<?> values)) {
      throw new HaraException(descriptor + " :project/package :entry-points must be a vector");
    }
    if (values.count() != 1) {
      throw new HaraException(
          descriptor + " :project/package :entry-points must contain exactly one JVM entry point");
    }
    Object entry = values.nth(0);
    String className =
        entry instanceof Symbol symbol && symbol.getNamespace() == null
            ? symbol.display()
            : entry instanceof String string ? string : null;
    if (className != null
        && className.matches("[A-Za-z_$][A-Za-z0-9_$]*(\\.[A-Za-z_$][A-Za-z0-9_$]*)+")) {
      return className;
    }
    throw new HaraException(
        descriptor
            + " :project/package :entry-points must contain one fully-qualified JVM class name");
  }

  private static java.util.List<Path> mergePaths(
      java.util.List<Path> shared, java.util.List<Path> runtime) {
    ArrayList<Path> paths = new ArrayList<>(shared);
    paths.addAll(runtime);
    return java.util.List.copyOf(paths);
  }

  private static Map<String, String> mergeHaraDependencies(
      Map<String, String> shared, Map<String, String> runtime, String profile) {
    LinkedHashMap<String, String> dependencies = new LinkedHashMap<>(shared);
    for (Map.Entry<String, String> entry : runtime.entrySet()) {
      String existing = dependencies.get(entry.getKey());
      if (existing != null && !existing.equals(entry.getValue())) {
        throw new HaraException(
            "Conflicting Hara dependency requirements for "
                + entry.getKey()
                + " in :"
                + profile
                + ": "
                + existing
                + " and "
                + entry.getValue());
      }
      dependencies.put(entry.getKey(), entry.getValue());
    }
    return Map.copyOf(dependencies);
  }

  private static Map<String, String> haraDependencies(Object value, String descriptor) {
    if (value == null) return Map.of();
    if (!(value instanceof IMapType<?, ?> entries)) {
      throw new HaraException(descriptor + " Hara dependencies must be a map");
    }
    LinkedHashMap<String, String> dependencies = new LinkedHashMap<>();
    Iterator<?> iterator = entries.iterator();
    while (iterator.hasNext()) {
      Map.Entry<?, ?> entry = (Map.Entry<?, ?>) iterator.next();
      String coordinate = haraCoordinate(entry.getKey(), descriptor);
      if (!(entry.getValue() instanceof IMapType<?, ?> declaration)
          || !(lookup(declaration, "version") instanceof String version)
          || version.isBlank()) {
        throw new HaraException(
            descriptor + " Hara dependency " + coordinate + " requires :version");
      }
      if (dependencies.put(coordinate, version) != null) {
        throw new HaraException(descriptor + " duplicate Hara dependency " + coordinate);
      }
    }
    return Map.copyOf(dependencies);
  }

  private static String haraCoordinate(Object value, String descriptor) {
    String coordinate;
    if (value instanceof Symbol symbol) {
      coordinate = symbol.display();
    } else if (value instanceof String text) {
      coordinate = text;
    } else {
      throw new HaraException(descriptor + " Hara dependency coordinates must be symbols or strings");
    }
    if (coordinate.startsWith("official:")) {
      coordinate = "hara:" + coordinate.substring("official:".length());
    } else if (!coordinate.contains(":")) {
      coordinate = "hara:" + coordinate;
    }
    if (!coordinate.matches("[a-z0-9_.-]+:[a-z0-9_.-]+/[a-z0-9_.-]+")) {
      throw new HaraException(descriptor + " invalid Hara dependency coordinate " + coordinate);
    }
    return coordinate;
  }

  private static void rejectLegacyRuntimeKeys(IMapType<?, ?> options, String descriptor) {
    for (Map.Entry<String, String> legacy :
        Map.of(
                "jvm/source-paths",
                ":project/runtime-profiles :jvm :runtime/native-source-paths",
                "jvm/dependencies",
                ":project/runtime-profiles :jvm :runtime/dependencies :maven",
                "jvm/target-path",
                ":project/runtime-profiles :jvm :runtime/target-path")
            .entrySet()) {
      if (lookup(options, legacy.getKey()) != null) {
        throw new HaraException(
            descriptor
                + " :"
                + legacy.getKey()
                + " is no longer supported; use "
                + legacy.getValue());
      }
    }
  }

  @SuppressWarnings("rawtypes")
  private static Object lookup(IMapType<?, ?> map, String key) {
    return ((IMapType) map).lookup(Keyword.create(key));
  }

  private static java.util.List<Path> paths(
      Path root,
      Object value,
      String option,
      java.util.List<String> defaults,
      String descriptor) {
    Iterable<?> entries;
    if (value == null) {
      entries = defaults;
    } else if (value instanceof ILinearType<?>) {
      entries = (ILinearType<?>) value;
    } else {
      throw new HaraException(descriptor + " :" + option + " expects a sequential collection");
    }
    ArrayList<Path> paths = new ArrayList<>();
    for (Object entry : entries) {
      if (!(entry instanceof String) || ((String) entry).isBlank()) {
        throw new HaraException(descriptor + " :" + option + " expects non-empty path strings");
      }
      Path path = root.resolve((String) entry).normalize();
      if (!path.startsWith(root)) {
        throw new HaraException(descriptor + " :" + option + " cannot escape the project root");
      }
      paths.add(path);
    }
    return Collections.unmodifiableList(paths);
  }

  private static Path path(
      Path root, Object value, String option, String defaultValue, String descriptor) {
    Object selected = value == null ? defaultValue : value;
    if (!(selected instanceof String entry) || entry.isBlank()) {
      throw new HaraException(descriptor + " :" + option + " expects a non-empty path string");
    }
    Path path = root.resolve(entry).normalize();
    if (!path.startsWith(root)) {
      throw new HaraException(descriptor + " :" + option + " cannot escape the project root");
    }
    return path;
  }

  private static java.util.List<JvmDependency> mavenDependencies(
      Object value, String descriptor) {
    if (value == null) return java.util.List.of();
    if (!(value instanceof IMapType<?, ?> entries)) {
      throw new HaraException(descriptor + " runtime Maven dependencies must be a map");
    }
    ArrayList<JvmDependency> dependencies = new ArrayList<>();
    LinkedHashSet<String> ids = new LinkedHashSet<>();
    Iterator<?> iterator = entries.iterator();
    while (iterator.hasNext()) {
      Map.Entry<?, ?> entry = (Map.Entry<?, ?>) iterator.next();
      Object idValue = entry.getKey();
      String id;
      if (idValue instanceof Symbol symbol) {
        id = symbol.display();
      } else if (idValue instanceof String text) {
        id = text.replace(':', '/');
      } else {
        throw new HaraException(
            descriptor + " Maven dependency coordinates must be symbols or strings");
      }
      if (!id.matches("[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+")) {
        throw new HaraException(descriptor + " invalid Maven dependency coordinate " + id);
      }
      if (!(entry.getValue() instanceof IMapType<?, ?> declaration)
          || !(lookup(declaration, "version") instanceof String version)
          || !version.matches("[A-Za-z0-9][A-Za-z0-9._+-]*")) {
        throw new HaraException(
            descriptor + " Maven dependency " + id + " requires an exact version");
      }
      if (!ids.add(id)) {
        throw new HaraException(descriptor + " duplicate Maven dependency " + id);
      }
      dependencies.add(new JvmDependency(id, version));
    }
    return java.util.List.copyOf(dependencies);
  }

  private static Set<String> capabilities(Object value, String descriptor) {
    if (value == null) return Set.of();
    if (!(value instanceof Iterable<?> entries)) {
      throw new HaraException(descriptor + " :project/capabilities expects a collection");
    }
    LinkedHashSet<String> capabilities = new LinkedHashSet<>();
    for (Object entry : entries) {
      if (!(entry instanceof Keyword capability)) {
        throw new HaraException(descriptor + " :project/capabilities expects keywords");
      }
      String name =
          capability.getNamespace() == null
              ? capability.getName()
              : capability.getNamespace() + "/" + capability.getName();
      capabilities.add(name);
    }
    return Set.copyOf(capabilities);
  }
}
