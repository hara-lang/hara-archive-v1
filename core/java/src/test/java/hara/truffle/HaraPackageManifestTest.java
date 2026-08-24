package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertThrows;

import java.nio.file.Files;
import java.nio.file.Path;
import org.junit.Test;

public class HaraPackageManifestTest {
  private static final String CORE_MANIFEST =
      "{:harp/format \"0.0.0-alpha\" "
          + ":package {:identity \"example/math\" :version \"1.0.0\"} "
          + ":files {\"artifacts/math.wasm\" {:sha256 \"sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad\" :size 3}} "
          + ":wasm-imports {:math {:variant/artifact {:artifact/type :wasm :artifact/path \"artifacts/math.wasm\" "
          + ":artifact/sha256 \"sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad\" "
          + ":artifact/target \"wasm32-wasi-preview1\" :artifact/abi \"core.v1\" :artifact/entry-point \"add\"} "
          + ":variant/required-capabilities #{} :variant/host-calls #{} :variant/exports #{:add}}}}";

  private static final String JVM_MANIFEST =
      "{:harp/format \"0.0.0-alpha\" "
          + ":package {:identity \"example/provider\" :version \"1.0.0\" "
          + ":provenance {:repository \"https://github.com/example/provider\" "
          + ":commit \"0123456789abcdef0123456789abcdef01234567\"}} "
          + ":files {\"artifacts/provider.jar\" {:sha256 \"sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad\" :size 3}} "
          + ":flavors {:jvm {:variant/artifact {:artifact/type :jar "
          + ":artifact/path \"artifacts/provider.jar\" "
          + ":artifact/sha256 \"sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad\" "
          + ":artifact/target \"java-21\" :artifact/abi \"hara.provider.jvm.v1\" "
          + ":artifact/entry-point \"example.Provider\"} :variant/required-capabilities #{} "
          + ":variant/dependencies {:maven {org.example/provider-runtime {:version \"1.0.0\"}}}}}}";

  @Test
  public void verifiesTheIndexedCoreWasmImportAndItsDigest() throws Exception {
    Path root = Files.createTempDirectory("hara-package-manifest-");
    try {
      Files.createDirectories(root.resolve("artifacts"));
      Files.write(root.resolve("artifacts/math.wasm"), new byte[] {'a', 'b', 'c'});
      HaraPackageManifest manifest = HaraPackageManifest.parse(CORE_MANIFEST, "test");
      assertEquals(root.resolve("artifacts/math.wasm"), manifest.verifyImport(root, "math"));
    } finally {
      Files.deleteIfExists(root.resolve("artifacts/math.wasm"));
      Files.deleteIfExists(root.resolve("artifacts"));
      Files.deleteIfExists(root);
    }
  }

  @Test
  public void directImportsRejectNonCoreAbisBeforeActivation() {
    String hta = CORE_MANIFEST.replace(":artifact/abi \"core.v1\"", ":artifact/abi \"hta.v1\"");
    HaraPackageManifest manifest = HaraPackageManifest.parse(hta, "test");
    HaraException error = assertThrows(HaraException.class, () -> manifest.verifyImport(Path.of("."), "math"));
    assertEquals(true, error.getMessage().contains("requires core.v1"));
  }

  @Test
  public void verifiesTheJvmFlavorArtifactAndRequiresProvenance() throws Exception {
    Path root = Files.createTempDirectory("hara-package-jvm-manifest-");
    try {
      Files.createDirectories(root.resolve("artifacts"));
      Files.write(root.resolve("artifacts/provider.jar"), new byte[] {'a', 'b', 'c'});
      HaraPackageManifest manifest = HaraPackageManifest.parse(JVM_MANIFEST, "test");
      assertEquals(
          root.resolve("artifacts/provider.jar"), manifest.verifyJvmFlavor(root));
      assertEquals(
          "1.0.0", manifest.jvmFlavor().mavenDependencies().get("org.example/provider-runtime"));

      String missingProvenance =
          JVM_MANIFEST.replace(
              ":provenance {:repository \"https://github.com/example/provider\" "
                  + ":commit \"0123456789abcdef0123456789abcdef01234567\"}",
              "");
      HaraException error =
          assertThrows(
              HaraException.class, () -> HaraPackageManifest.parse(missingProvenance, "test"));
      assertEquals(true, error.getMessage().contains("require :package :provenance"));
    } finally {
      Files.deleteIfExists(root.resolve("artifacts/provider.jar"));
      Files.deleteIfExists(root.resolve("artifacts"));
      Files.deleteIfExists(root);
    }
  }
}
