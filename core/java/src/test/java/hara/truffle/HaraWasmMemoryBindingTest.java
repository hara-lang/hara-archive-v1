package hara.truffle;

import static org.junit.Assert.assertArrayEquals;
import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertThrows;
import static org.junit.Assert.assertTrue;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Comparator;
import org.junit.Test;

public class HaraWasmMemoryBindingTest {
  private static final byte[] MEMORY_MODULE =
      hex(
          "0061736d0100000001140460017f017f60017f0060027f7f017e6000017f"
              + "030504000102030504010101100606017f0141000b073605066d656d6f7279"
              + "020005616c6c6f630000046672656500010a6563686f5f627974657300020d"
              + "72656c656173655f636f756e7400030a230405004180080b0900230041016a"
              + "24000b0c002001ad4220862000ad840b040023000b");
  private static final byte[] MEMORY_ENV_MODULE =
      hex(
          "0061736d0100000001180560017f017f60017f0060027f7f017e6000017f6000017e"
              + "02140103656e760c686172615f74696d655f6d730004030504000102030504010101"
              + "100606017f0141000b073605066d656d6f7279020005616c6c6f6300010466726565"
              + "00020a6563686f5f627974657300030d72656c656173655f636f756e7400040a230405"
              + "004180080b0900230041016a24000b0c002001ad4220862000ad840b040023000b");

  @Test
  public void executesBytesAndUtf8ThroughTheCanonicalPlan() throws Exception {
    Path bytesRoot = packageRoot("bytes", "borrowed", "bytes", "caller", MEMORY_MODULE);
    try (HaraWasmExtension extension = extension(bytesRoot)) {
      byte[] input = {1, 2, 3, 4};
      assertArrayEquals(input, (byte[]) extension.invoke("echo", new Object[] {input}));
      assertEquals(1L, extension.invoke("release-count", new Object[] {}));
    } finally {
      deleteTree(bytesRoot);
    }

    Path stringRoot = packageRoot("string", "borrowed", "string", "caller", MEMORY_MODULE);
    try (HaraWasmExtension extension = extension(stringRoot)) {
      String input = "hara memory binding";
      assertEquals(input, extension.invoke("echo", new Object[] {input}));
      assertEquals(1L, extension.invoke("release-count", new Object[] {}));
    } finally {
      deleteTree(stringRoot);
    }
  }

  @Test
  public void transferredInputsAreReleasedOnlyWhenInvocationDoesNotComplete() throws Exception {
    Path successRoot =
        packageRoot("bytes", "transferred", "bytes", "callee", MEMORY_MODULE);
    try (HaraWasmExtension extension = extension(successRoot)) {
      byte[] input = {9, 8, 7};
      assertArrayEquals(input, (byte[]) extension.invoke("echo", new Object[] {input}));
      assertEquals(0L, extension.invoke("release-count", new Object[] {}));
    } finally {
      deleteTree(successRoot);
    }

    byte[] trapping = moduleWithBody(2, new byte[] {0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 11});
    Path failureRoot =
        packageRoot("bytes", "transferred", "bytes", "callee", trapping);
    try (HaraWasmExtension extension = extension(failureRoot)) {
      HaraException error =
          assertThrows(
              HaraException.class,
              () -> extension.invoke("echo", new Object[] {new byte[] {1, 2, 3}}));
      assertTrue(error.getMessage().startsWith("extension/invoke-failed"));
      assertEquals(1L, extension.invoke("release-count", new Object[] {}));
    } finally {
      deleteTree(failureRoot);
    }
  }

  @Test
  public void invalidUtf8StillReleasesCallerOwnedResults() throws Exception {
    Path root = packageRoot("bytes", "borrowed", "string", "caller", MEMORY_MODULE);
    try (HaraWasmExtension extension = extension(root)) {
      HaraException error =
          assertThrows(
              HaraException.class,
              () -> extension.invoke("echo", new Object[] {new byte[] {(byte) 0xff}}));
      assertTrue(error.getMessage().startsWith("extension/utf8-invalid"));
      assertEquals(1L, extension.invoke("release-count", new Object[] {}));
    } finally {
      deleteTree(root);
    }
  }

  @Test
  public void memoryWasmRejectsHtaEnvironmentImports() throws Exception {
    Path root = packageRoot("bytes", "borrowed", "bytes", "caller", MEMORY_ENV_MODULE);
    try {
      HaraException error =
          assertThrows(HaraException.class, () -> extension(root));
      assertTrue(error.getMessage().startsWith("extension/module-invalid"));
    } finally {
      deleteTree(root);
    }
  }

  @Test
  public void rejectsManifestDriftAndMissingTransferredInputCleanup() {
    String plan = bindingPlan("bytes", "transferred", "bytes", "callee");
    HaraException missingRelease =
        assertThrows(
            HaraException.class,
            () ->
                HaraWasmMemoryBinding.parse(
                    plan.replace(" :release \"free\"", ""), "missing-release"));
    assertTrue(missingRelease.getMessage().contains("require :memory :release"));

    HaraWasmMemoryBinding parsed = HaraWasmMemoryBinding.parse(plan, "fixture");
    HaraExtensionManifest drifted =
        HaraExtensionManifest.parse(
            manifest("bytes", "bytes").replace("echo_bytes", "different_export"),
            "fixture");
    HaraException mismatch =
        assertThrows(HaraException.class, () -> parsed.verifyManifest(drifted));
    assertTrue(mismatch.getMessage().startsWith("extension/manifest-mismatch"));
  }

  private static Path packageRoot(
      String argumentType,
      String argumentOwnership,
      String resultType,
      String resultOwnership,
      byte[] module)
      throws Exception {
    Path root = Files.createTempDirectory("hara-memory-v1-");
    HaraExtensionTestProject.write(root, manifest(argumentType, resultType));
    Files.write(root.resolve("echo.wasm"), module);
    Files.writeString(
        root.resolve("bindings.edn"),
        bindingPlan(argumentType, argumentOwnership, resultType, resultOwnership));
    return root;
  }

  private static HaraWasmExtension extension(Path root) throws Exception {
    Path descriptor = root.resolve("project.edn");
    HaraProject project = HaraProject.read(descriptor);
    HaraExtensionManifest manifest =
        HaraExtensionManifest.parse(
            project.extensionManifestSource("codec.echo"), descriptor.toString());
    HaraExtensionPackage extensionPackage =
        new HaraExtensionPackage(manifest, descriptor.toUri().toURL());
    extensionPackage.validateDeclaredFiles();
    return new HaraWasmExtension(extensionPackage);
  }

  private static String manifest(String argumentType, String resultType) {
    return "{:namespace \"codec.echo\" :version \"0.1.0\" :provider :wasm "
        + ":module \"echo.wasm\" :abi :memory.v1 "
        + ":exports {\"echo\" {:wasm/export \"echo_bytes\" :args [:"
        + argumentType
        + "] :returns :"
        + resultType
        + " :async false} "
        + "\"release-count\" {:wasm/export \"release_count\" :args [] "
        + ":returns :i32 :async false}} "
        + ":capabilities [] :assets [\"bindings.edn\"]}";
  }

  private static String bindingPlan(
      String argumentType,
      String argumentOwnership,
      String resultType,
      String resultOwnership) {
    return "{:schema \"hara.wasm-memory-binding/0-alpha\" "
        + ":namespace codec.echo :module \"echo.wasm\" :target :memory.v1 "
        + ":memory {:export \"memory\" :allocate \"alloc\" :release \"free\"} "
        + ":functions ["
        + "{:hara/name echo :wasm/export \"echo_bytes\" "
        + ":arguments [{:name input :hara/type :"
        + argumentType
        + " :wasm/types [:i32 :i32] :lower [:pointer :length] :ownership :"
        + argumentOwnership
        + "}] "
        + ":returns {:hara/type :"
        + resultType
        + " :wasm/type :i64 :lift :packed-i64 :ownership :"
        + resultOwnership
        + "} "
        + ":wasm/arguments [:i32 :i32] :wasm/returns :i64} "
        + "{:hara/name release-count :wasm/export \"release_count\" "
        + ":arguments [] :returns {:hara/type :i32 :wasm/type :i32} "
        + ":wasm/arguments [] :wasm/returns :i32}]}";
  }

  private static byte[] moduleWithBody(int function, byte[] body) {
    byte[] module = MEMORY_MODULE.clone();
    int bodyStart;
    int bodySize;
    switch (function) {
      case 0 -> {
        bodyStart = 111;
        bodySize = 5;
      }
      case 1 -> {
        bodyStart = 117;
        bodySize = 9;
      }
      case 2 -> {
        bodyStart = 127;
        bodySize = 12;
      }
      case 3 -> {
        bodyStart = 140;
        bodySize = 4;
      }
      default -> throw new IllegalArgumentException("invalid fixture function");
    }
    if (body.length != bodySize) throw new IllegalArgumentException("invalid fixture body");
    module[bodyStart - 1] = (byte) body.length;
    System.arraycopy(body, 0, module, bodyStart, body.length);
    return module;
  }

  private static byte[] hex(String source) {
    byte[] bytes = new byte[source.length() / 2];
    for (int index = 0; index < bytes.length; index++) {
      bytes[index] =
          (byte) Integer.parseInt(source.substring(index * 2, index * 2 + 2), 16);
    }
    return bytes;
  }

  private static void deleteTree(Path root) throws Exception {
    if (root == null || !Files.exists(root)) return;
    try (var paths = Files.walk(root)) {
      for (Path path : paths.sorted(Comparator.reverseOrder()).toList()) {
        Files.deleteIfExists(path);
      }
    }
  }
}
