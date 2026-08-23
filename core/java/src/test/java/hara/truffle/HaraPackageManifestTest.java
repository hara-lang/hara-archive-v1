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
}
