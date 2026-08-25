package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertThrows;
import static org.junit.Assert.assertTrue;
import static org.junit.Assume.assumeTrue;

import java.io.InputStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.security.MessageDigest;
import org.graalvm.polyglot.Context;
import org.graalvm.polyglot.Value;
import org.junit.Test;

public class HaraHtaExtensionTest {
  private static final Path ARTIFACT =
      Path.of("rust/raw/target/wasm32-unknown-unknown/browser-release/hara-wasm-vm.wasm");
  private static final String ADAPTER_FIXTURE_DIGEST =
      "6742ab577c2f6852103effd650d97d88c7427fd0e7520466126f892fb4fb0dab";
  private static final String LIBRARY_FIXTURE_DIGEST =
      "cf96c3351ea2afd66dd2cee4480ea44fd2e76f8009ca1df96edb9dc149749edc";

  @Test
  public void htaTimeoutConfigurationMatchesTheRustBoundary() {
    String previous = System.getProperty("hara.hta.timeout.ms");
    try {
      System.setProperty("hara.hta.timeout.ms", "0");
      assertEquals(0L, HaraWasmExtension.htaTimeoutMillis());
      System.setProperty("hara.hta.timeout.ms", "17");
      assertEquals(17L, HaraWasmExtension.htaTimeoutMillis());
      System.setProperty("hara.hta.timeout.ms", "-1");
      assertEquals(120_000L, HaraWasmExtension.htaTimeoutMillis());
      System.setProperty("hara.hta.timeout.ms", "invalid");
      assertEquals(120_000L, HaraWasmExtension.htaTimeoutMillis());
    } finally {
      if (previous == null) System.clearProperty("hara.hta.timeout.ms");
      else System.setProperty("hara.hta.timeout.ms", previous);
    }
  }

  @Test
  public void htaActorEvaluatesAndSettlesTasks() throws Exception {
    withExtension(
        "",
        context -> {
          assertEquals(
              42,
              context
                  .eval(
                      HaraLanguage.ID,
                      "(ns app (:require [tool.runtime.wasm :as runtime])) "
                          + "(deref (runtime/eval \"(+ 20 22)\"))")
                  .asLong());
        });
  }

  @Test
  public void allowlistedHostCallSettlesWithSha256Bytes() throws Exception {
    withExtension(
        ":host-calls {\"crypto.hash.sha256\" [\"digest\"]}",
        context -> {
          Value digest =
              context.eval(
                  HaraLanguage.ID,
                  "(ns app (:require [tool.runtime.wasm :as runtime])) "
                      + "(deref (runtime/eval "
                      + "\"(std.native.Host/call \\\"crypto.hash.sha256\\\" \\\"digest\\\" [(bytes 97 98 99)])\"))");
          assertTrue(digest.hasArrayElements());
          assertEquals(32, digest.getArraySize());
          assertEquals((byte) 0xba, digest.getArrayElement(0).asByte());
          assertEquals((byte) 0xad, digest.getArrayElement(31).asByte());
        });
  }

  @Test
  public void pendingDerefResumesInsideNestedEvaluation() throws Exception {
    withExtension(
        ":host-calls {\"crypto.hash.sha256\" [\"digest\"]}",
        context ->
            assertEquals(
                42,
                context
                    .eval(
                        HaraLanguage.ID,
                        "(ns app (:require [tool.runtime.wasm :as runtime])) "
                            + "(deref (runtime/eval \"(+ 10 (count (deref (std.native.Host/call \\\"crypto.hash.sha256\\\" \\\"digest\\\" [(bytes 97 98 99)]))))\"))")
                    .asLong()));
  }

  @Test
  public void generatedAdapterLinksTheWrappedLibraryThroughHaraLibraryImports() throws Exception {
    Path root = Files.createTempDirectory("hara-hta-adapter-");
    Path extension = root.resolve("math/async");
    Files.createDirectories(extension);
    HaraExtensionTestProject.write(
        extension,
        "{:namespace \"math.async\" :version \"1.0.0\" :provider :wasm "
            + ":module \"adapter.wasm\" :abi :hta.v1 "
            + ":exports {\"sum\" {:args [:i64 :i64] :returns :i64 :async true}} "
            + ":assets [\"library.wasm\"] :capabilities []}");
    Files.write(extension.resolve("adapter.wasm"), resource("adapter.wasm"));
    Files.write(extension.resolve("library.wasm"), resource("library.wasm"));
    String previous = System.getProperty("hara.extensions.path");
    System.setProperty("hara.extensions.path", root.toString());
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      assertEquals(
          42,
          context
              .eval(
                  HaraLanguage.ID,
                  "(ns app (:require [math.async :as math])) "
                      + "(deref (math/sum 19 23))")
              .asLong());
    } finally {
      if (previous == null) System.clearProperty("hara.extensions.path");
      else System.setProperty("hara.extensions.path", previous);
      try (var paths = Files.walk(root)) {
        paths
            .sorted(java.util.Comparator.reverseOrder())
            .forEach(
                path -> {
                  try {
                    Files.deleteIfExists(path);
                  } catch (Exception ignored) {
                  }
                });
      }
    }
  }

  @Test
  public void compositionFixturesMatchTheReviewedCrossHostDigests() throws Exception {
    assertEquals(ADAPTER_FIXTURE_DIGEST, sha256(resource("adapter.wasm")));
    assertEquals(LIBRARY_FIXTURE_DIGEST, sha256(resource("library.wasm")));
  }

  @Test
  public void malformedWrappedLibraryIsRejectedBeforeAdapterInstantiation() throws Exception {
    Path root = Files.createTempDirectory("hara-hta-malformed-");
    Path extension = root.resolve("math/async");
    Files.createDirectories(extension);
    String descriptor =
        "{:namespace \"math.async\" :version \"1.0.0\" :provider :wasm "
            + ":module \"adapter.wasm\" :abi :hta.v1 "
            + ":exports {\"sum\" {:args [:i64 :i64] :returns :i64 :async true}} "
            + ":assets [\"library.wasm\"] :capabilities []}";
    HaraExtensionTestProject.write(extension, descriptor);
    Files.write(extension.resolve("adapter.wasm"), resource("adapter.wasm"));
    Files.write(
        extension.resolve("library.wasm"), replaceUtf8(resource("library.wasm"), "add", "sub"));
    Path project = extension.resolve("project.edn");
    try {
      HaraExtensionManifest manifest =
          HaraExtensionManifest.parse(
              HaraProject.read(project).extensionManifestSource("math.async"),
              project.toString());
      HaraExtensionPackage extensionPackage =
          new HaraExtensionPackage(manifest, project.toUri().toURL());
      extensionPackage.validateDeclaredFiles();
      HaraException error =
          assertThrows(HaraException.class, () -> new HaraWasmExtension(extensionPackage));
      assertTrue(error.getMessage().startsWith("extension/module-invalid"));
    } finally {
      deleteTree(root);
    }
  }

  @Test
  public void rejectedPendingDerefFlowsThroughCatch() throws Exception {
    withExtension(
        "",
        context ->
            assertEquals(
                42,
                context
                    .eval(
                        HaraLanguage.ID,
                        "(ns app (:require [tool.runtime.wasm :as runtime])) "
                            + "(deref (runtime/eval \"(try (deref (std.native.Host/call \\\"denied\\\" \\\"call\\\" [])) (catch error 42))\"))")
                    .asLong()));
  }

  private static void withExtension(String hostCalls, CheckedConsumer operation) throws Exception {
    assumeTrue("build rust/raw before HTA tests: " + ARTIFACT, Files.isRegularFile(ARTIFACT));
    Path root = Files.createTempDirectory("hara-hta-extension-");
    Path extension = root.resolve("hara/runtime/wasm");
    Files.createDirectories(extension);
    HaraExtensionTestProject.write(
        extension,
        "{:namespace \"tool.runtime.wasm\" :version \"0.1.0\" :provider :wasm "
            + ":module \"hara.wasm\" :abi :hta.v1 "
            + ":exports {\"eval\" {:args [:value] :returns :value :async true}} "
            + ":capabilities [] "
            + hostCalls
            + "}");
    Files.copy(ARTIFACT, extension.resolve("hara.wasm"));
    String previous = System.getProperty("hara.extensions.path");
    System.setProperty("hara.extensions.path", root.toString());
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      operation.accept(context);
    } finally {
      if (previous == null) System.clearProperty("hara.extensions.path");
      else System.setProperty("hara.extensions.path", previous);
      Files.deleteIfExists(extension.resolve("hara.wasm"));
      Files.deleteIfExists(extension.resolve("project.edn"));
      Files.deleteIfExists(extension);
      Files.deleteIfExists(extension.getParent());
      Files.deleteIfExists(extension.getParent().getParent());
      Files.deleteIfExists(root);
    }
  }

  private interface CheckedConsumer {
    void accept(Context context) throws Exception;
  }

  private static byte[] resource(String name) throws Exception {
    try (InputStream input =
        HaraHtaExtensionTest.class.getResourceAsStream("/hta-adapter/" + name)) {
      assertTrue("missing HTA adapter fixture " + name, input != null);
      return input.readAllBytes();
    }
  }

  private static String sha256(byte[] bytes) throws Exception {
    byte[] digest = MessageDigest.getInstance("SHA-256").digest(bytes);
    StringBuilder result = new StringBuilder(digest.length * 2);
    for (byte value : digest) result.append(String.format("%02x", value));
    return result.toString();
  }

  private static byte[] replaceUtf8(byte[] bytes, String from, String to) {
    byte[] source = from.getBytes(java.nio.charset.StandardCharsets.UTF_8);
    byte[] replacement = to.getBytes(java.nio.charset.StandardCharsets.UTF_8);
    if (source.length != replacement.length) {
      throw new IllegalArgumentException("fixture lengths differ");
    }
    byte[] copy = bytes.clone();
    for (int index = 0; index <= copy.length - source.length; index++) {
      boolean match = true;
      for (int offset = 0; offset < source.length; offset++) {
        if (copy[index + offset] != source[offset]) {
          match = false;
          break;
        }
      }
      if (match) {
        System.arraycopy(replacement, 0, copy, index, replacement.length);
        return copy;
      }
    }
    throw new IllegalArgumentException("fixture marker missing: " + from);
  }

  private static void deleteTree(Path root) throws Exception {
    try (var paths = Files.walk(root)) {
      paths
          .sorted(java.util.Comparator.reverseOrder())
          .forEach(
              path -> {
                try {
                  Files.deleteIfExists(path);
                } catch (Exception ignored) {
                }
              });
    }
  }
}
