package hara.truffle;

import java.io.IOException;
import java.io.InputStream;
import java.net.URISyntaxException;
import java.net.URL;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.LinkedHashSet;

/** A validated extension descriptor and its colocated provider artifacts. */
final class HaraExtensionPackage {
  private static final int MAX_MODULE_BYTES = 64 * 1024 * 1024;
  private static final int MAX_BINDING_BYTES = 4 * 1024 * 1024;

  private final HaraExtensionManifest manifest;
  private final URL descriptor;

  HaraExtensionPackage(HaraExtensionManifest manifest, URL descriptor) {
    this.manifest = manifest;
    this.descriptor = descriptor;
  }

  HaraExtensionManifest manifest() {
    return manifest;
  }

  URL descriptor() {
    return descriptor;
  }

  URL resolve(String relative) {
    try {
      return new URL(descriptor, relative);
    } catch (IOException error) {
      throw new HaraException(
          "extension/asset-invalid: " + manifest.namespace() + "/" + relative);
    }
  }

  Path file(String relative) {
    URL url = resolve(relative);
    if (!"file".equals(url.getProtocol())) {
      throw new HaraException(
          "extension/target-unavailable: process modules must be installed as files: " + url);
    }
    try {
      Path packageRoot = Path.of(descriptor.toURI()).getParent().toRealPath();
      Path resolved = Path.of(url.toURI()).toRealPath();
      if (!resolved.startsWith(packageRoot)) {
        throw new HaraException("extension/path-denied: " + relative);
      }
      return resolved;
    } catch (IOException | URISyntaxException error) {
      throw new HaraException(
          "extension/asset-unavailable: "
              + manifest.namespace()
              + "/"
              + relative
              + " ("
              + error.getMessage()
              + ")");
    }
  }

  void validateDeclaredFiles() {
    LinkedHashSet<String> paths = new LinkedHashSet<>(manifest.assets());
    if (manifest.module() != null) paths.add(manifest.module());
    manifest.targets().values().forEach(target -> paths.add(target.provider()));
    for (String path : paths) {
      URL asset = resolve(path);
      try (InputStream ignored = asset.openStream()) {
        // Opening is sufficient here; provider-specific readers enforce size limits.
      } catch (IOException error) {
        throw new HaraException(
            "extension/asset-unavailable: " + manifest.namespace() + "/" + path);
      }
    }
  }

  byte[] moduleBytes() {
    if (manifest.module() == null) {
      throw new HaraException("extension/module-unavailable: " + manifest.namespace());
    }
    return readBytes(manifest.module(), MAX_MODULE_BYTES, "module");
  }

  byte[] wrappedLibraryBytes() {
    if (!"hta.v1".equals(manifest.abi())) return null;
    String library =
        manifest.assets().stream()
            .filter(path -> path.endsWith(".wasm") && !path.equals(manifest.module()))
            .findFirst()
            .orElse(null);
    return library == null ? null : readBytes(library, MAX_MODULE_BYTES, "library");
  }

  HaraWasmMemoryBinding memoryBinding() {
    if (!"memory.v1".equals(manifest.abi())) {
      throw new HaraException(
          "extension/abi-invalid: " + manifest.namespace() + " is not a memory.v1 package");
    }
    String source =
        new String(
            readBytes("bindings.edn", MAX_BINDING_BYTES, "binding plan"),
            StandardCharsets.UTF_8);
    HaraWasmMemoryBinding binding =
        HaraWasmMemoryBinding.parse(source, manifest.namespace() + "/bindings.edn");
    binding.verifyManifest(manifest);
    return binding;
  }

  private byte[] readBytes(String relative, int maximum, String subject) {
    URL asset = resolve(relative);
    try {
      byte[] bytes;
      try (InputStream input = asset.openStream()) {
        bytes = input.readNBytes(maximum + 1);
      }
      if (bytes.length > maximum) {
        throw new HaraException("extension/" + subject.replace(' ', '-') + "-too-large: " + asset);
      }
      return bytes;
    } catch (IOException error) {
      throw new HaraException(
          "extension/asset-unavailable: "
              + manifest.namespace()
              + "/"
              + relative
              + " ("
              + error.getMessage()
              + ")");
    }
  }
}
