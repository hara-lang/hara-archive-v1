package hara.truffle;

import java.io.IOException;
import java.net.URL;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Collections;
import java.util.List;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;

/** Discovers extension manifests from the classpath and configured project roots. */
final class HaraExtensionRegistry {
  record JvmFlavorPackage(Path root, HaraPackageManifest manifest) {}

  private final List<Path> roots;
  private final Map<String, HaraExtensionPackage> packages = new ConcurrentHashMap<>();

  HaraExtensionRegistry(ClassLoader classLoader) {
    this(classLoader, configuredRoots());
  }

  HaraExtensionRegistry(ClassLoader classLoader, List<Path> roots) {
    this.roots = Collections.unmodifiableList(new ArrayList<>(roots));
  }

  HaraExtensionPackage discover(String namespace) {
    return discover(namespace, java.util.List.of());
  }

  HaraExtensionPackage discover(String namespace, Path projectRoot) {
    return discover(
        namespace, projectRoot == null ? java.util.List.of() : java.util.List.of(projectRoot));
  }

  HaraExtensionPackage discover(String namespace, List<Path> projectRoots) {
    HaraExtensionPackage cached = packages.get(namespace);
    if (cached != null) return cached;
    try {
      ArrayList<HaraProject> candidates = new ArrayList<>();
      for (Path projectRoot : projectRoots) {
        addCandidates(candidates, projectRoot, namespace);
      }
      for (Path root : roots) addCandidates(candidates, root, namespace);
      if (candidates.isEmpty()) return null;
      if (candidates.size() > 1) {
        throw new HaraException(
            "extension/ambiguous: multiple packages export " + namespace + ": " + candidates);
      }
      HaraProject project = candidates.get(0);
      URL location = project.extensionRoot(namespace).resolve("project.edn").toUri().toURL();
      HaraExtensionManifest manifest =
          HaraExtensionManifest.parse(project.extensionManifestSource(namespace), project.descriptor().toString());
      HaraExtensionPackage extensionPackage = new HaraExtensionPackage(manifest, location);
      extensionPackage.validateDeclaredFiles();
      packages.put(namespace, extensionPackage);
      return extensionPackage;
    } catch (IOException | IllegalArgumentException error) {
      if (error instanceof HaraException) throw (HaraException) error;
      throw new HaraException(error.getMessage());
    }
  }

  /** Verifies an installed package index before exposing its extension root. */
  HaraExtensionPackage discoverWasmImport(String logical) {
    ArrayList<Path> candidates = new ArrayList<>();
    for (Path root : HaraPackageManifest.installedRoots()) {
      HaraPackageManifest manifest = HaraPackageManifest.read(root);
      if (manifest == null || manifest.wasmImport(logical) == null) continue;
      manifest.verifyImport(root, logical);
      candidates.add(root);
    }
    if (candidates.size() > 1) {
      throw new HaraException("package/ambiguous-wasm-import: " + logical);
    }
    if (candidates.isEmpty()) return null;
    HaraExtensionPackage extension = discover(logical, candidates);
    if (extension == null) {
      throw new HaraException("package/extension-missing: " + logical);
    }
    HaraPackageManifest manifest = HaraPackageManifest.read(candidates.get(0));
    HaraPackageManifest.WasmImport selected = manifest.wasmImport(logical);
    HaraExtensionManifest extensionManifest = extension.manifest();
    if (extensionManifest.identity() != null
        && !extensionManifest.identity().equals(manifest.identity())) {
      throw new HaraException("package/identity-mismatch: " + logical);
    }
    if (!selected.exports().stream().allMatch(extensionManifest.exports()::containsKey)) {
      throw new HaraException("package/manifest-mismatch: selected exports are not declared by extension");
    }
    if (!extensionManifest.exports().values().stream()
        .anyMatch(export -> selected.entryPoint().equals(export.wasmExport()))) {
      throw new HaraException("package/entry-point-mismatch: " + logical);
    }
    return extension;
  }

  /** Finds the one installed JVM flavor available to the current package context. */
  JvmFlavorPackage discoverJvmFlavor() {
    ArrayList<JvmFlavorPackage> candidates = new ArrayList<>();
    for (Path root : HaraPackageManifest.installedRoots()) {
      HaraPackageManifest manifest = HaraPackageManifest.read(root);
      if (manifest == null || manifest.jvmFlavor() == null) continue;
      manifest.verifyJvmFlavor(root);
      candidates.add(new JvmFlavorPackage(root, manifest));
    }
    if (candidates.size() > 1) {
      throw new HaraException("package/ambiguous-jvm-flavor: multiple installed packages provide :jvm");
    }
    return candidates.isEmpty() ? null : candidates.get(0);
  }

  private static void addCandidates(
      List<HaraProject> candidates, Path root, String namespace) throws IOException {
    Path normalizedRoot = root.toAbsolutePath().normalize();
    if (!Files.exists(normalizedRoot)) return;
    try (var paths = Files.walk(normalizedRoot, 8)) {
      for (Path descriptor : paths
          .filter(path -> Files.isRegularFile(path) && "project.edn".equals(path.getFileName().toString()))
          .toList()) {
        HaraProject project = HaraProject.read(descriptor);
        if (project.extensionDeclaration(namespace) != null
            && candidates.stream().noneMatch(value -> value.descriptor().equals(project.descriptor()))) {
          candidates.add(project);
        }
      }
    }
  }

  private static List<Path> configuredRoots() {
    ArrayList<Path> roots = new ArrayList<>();
    HaraProject project = HaraProject.discover(Path.of("."));
    if (project != null) roots.addAll(project.extensionRoots());
    String configured = System.getProperty("hara.extensions.path", "");
    if (configured.isBlank()) configured = System.getenv().getOrDefault("HARA_EXTENSION_PATH", "");
    if (configured.isBlank()) return roots;
    for (String value : configured.split(java.io.File.pathSeparator)) {
      if (!value.isBlank()) roots.add(Path.of(value).toAbsolutePath().normalize());
    }
    return roots;
  }
}
