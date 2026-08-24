package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertThrows;
import static org.junit.Assert.assertTrue;

import java.nio.file.Files;
import java.nio.file.Path;
import java.security.MessageDigest;
import java.util.HexFormat;
import org.graalvm.polyglot.Context;
import org.graalvm.polyglot.PolyglotException;
import org.junit.Test;

public class HaraWasmExtensionTest {
  private static final byte[] ADD_WASM = {
    0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, 0x01, 0x07, 0x01, 0x60, 0x02, 0x7f, 0x7f, 0x01,
    0x7f, 0x03, 0x02, 0x01, 0x00, 0x07, 0x07, 0x01, 0x03, 0x61, 0x64, 0x64, 0x00, 0x00, 0x0a, 0x09,
    0x01, 0x07, 0x00, 0x20, 0x00, 0x20, 0x01, 0x6a, 0x0b
  };
  private static final byte[] ENV_TIME_WASM = {
    0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00,
    0x01, 0x05, 0x01, 0x60, 0x00, 0x01, 0x7e,
    0x02, 0x14, 0x01, 0x03, 0x65, 0x6e, 0x76, 0x0c, 0x68, 0x61, 0x72, 0x61,
    0x5f, 0x74, 0x69, 0x6d, 0x65, 0x5f, 0x6d, 0x73, 0x00, 0x00,
    0x03, 0x02, 0x01, 0x00,
    0x07, 0x07, 0x01, 0x03, 0x6e, 0x6f, 0x77, 0x00, 0x01,
    0x0a, 0x06, 0x01, 0x04, 0x00, 0x10, 0x00, 0x0b
  };

  @Test
  public void coreWasmRejectsHtaEnvironmentImports() throws Exception {
    Path root = Files.createTempDirectory("hara-wasm-env-import-");
    Path extension = root.resolve("demo/time");
    Files.createDirectories(extension);
    HaraExtensionTestProject.write(
        extension,
        "{:namespace \"demo.time\" :version \"1.0.0\" :provider :wasm "
            + ":module \"time.wasm\" :abi :core.v1 "
            + ":exports {\"now\" {:args [] :returns :i64}} :capabilities []}");
    Files.write(extension.resolve("time.wasm"), ENV_TIME_WASM);
    Path descriptor = extension.resolve("project.edn");
    try {
      HaraProject project = HaraProject.read(descriptor);
      HaraExtensionManifest manifest =
          HaraExtensionManifest.parse(
              project.extensionManifestSource("demo.time"), descriptor.toString());
      HaraExtensionPackage extensionPackage =
          new HaraExtensionPackage(manifest, descriptor.toUri().toURL());
      HaraException error =
          assertThrows(HaraException.class, () -> new HaraWasmExtension(extensionPackage));
      assertTrue(error.getMessage().startsWith("extension/module-invalid"));
    } finally {
      deleteTree(root);
    }
  }

  @Test
  public void descriptorAndWasmGenerateTheDeclaredAnswer42Namespace() throws Exception {
    Path root = Files.createTempDirectory("hara-wasm-extension-");
    Path extension = root.resolve("demo/000-answer-42");
    Files.createDirectories(extension);
    HaraExtensionTestProject.write(
        extension,
        "{:namespace \"demo.000-answer-42\" :version \"1.0.0\" :provider :wasm "
            + ":module \"answer-42.wasm\" :abi :core.v1 "
            + ":exports {\"add\" {:args [:i32 :i32] :returns :i32 :async true}} "
            + ":capabilities []}");
    Files.write(extension.resolve("answer-42.wasm"), ADD_WASM);
    String previous = System.getProperty("hara.extensions.path");
    System.setProperty("hara.extensions.path", root.toString());
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      assertEquals(
          45,
          context
              .eval(
                  HaraLanguage.ID,
                  "(ns app (:require [demo.000-answer-42 :as answer :refer [add]])) "
                      + "(+ (deref (answer/add 20 22)) (deref (add 1 2)))")
              .asLong());
      assertEquals(
          42,
          context
              .eval(
                  HaraLanguage.ID,
                  "(ns imported (:import demo.000-answer-42)) "
                      + "(deref (demo.000-answer-42/add 20 22))")
              .asLong());
      PolyglotException arity =
          assertThrows(
              PolyglotException.class,
              () -> context.eval(HaraLanguage.ID, "(deref (demo.000-answer-42/add 1))"));
      assertTrue(arity.getMessage().contains("expects 2 arguments"));
    } finally {
      if (previous == null) System.clearProperty("hara.extensions.path");
      else System.setProperty("hara.extensions.path", previous);
      Files.deleteIfExists(extension.resolve("answer-42.wasm"));
      Files.deleteIfExists(extension.resolve("project.edn"));
      Files.deleteIfExists(extension);
      Files.deleteIfExists(extension.getParent());
      Files.deleteIfExists(extension.getParent().getParent());
      Files.deleteIfExists(root);
    }
  }

  @Test
  public void answer42IsNotInstalledUntilItsDescriptorAndWasmAreBothPresent() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      PolyglotException error =
          assertThrows(
              PolyglotException.class,
              () ->
                  context.eval(
                      HaraLanguage.ID, "(ns app (:require [demo.000-answer-42 :as answer]))"));
      assertTrue(error.getMessage().contains("Cannot require missing namespace"));
    }
  }

  @Test
  public void coreWasmCannotBorrowHtaCapabilities() throws Exception {
    Path root = Files.createTempDirectory("hara-wasm-capability-boundary-");
    Path extension = root.resolve("demo/capability-boundary");
    Files.createDirectories(extension);
    HaraExtensionTestProject.write(
        extension,
        "{:namespace \"demo.capability-boundary\" :version \"1.0.0\" :provider :wasm "
            + ":module \"answer-42.wasm\" :abi :core.v1 "
            + ":exports {\"add\" {:args [:i32 :i32] :returns :i32 :async true}} "
            + ":capabilities [:filesystem]}");
    Files.write(extension.resolve("answer-42.wasm"), ADD_WASM);
    String previousPath = System.getProperty("hara.extensions.path");
    String previousCapabilities = System.getProperty("hara.hta.capabilities");
    System.setProperty("hara.extensions.path", root.toString());
    System.setProperty("hara.hta.capabilities", "filesystem");
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      PolyglotException error =
          assertThrows(
              PolyglotException.class,
              () ->
                  context.eval(
                      HaraLanguage.ID,
                      "(ns app (:require [demo.capability-boundary]))"));
      assertTrue(error.getMessage().contains("extension/capability-denied"));
    } finally {
      if (previousPath == null) System.clearProperty("hara.extensions.path");
      else System.setProperty("hara.extensions.path", previousPath);
      if (previousCapabilities == null) System.clearProperty("hara.hta.capabilities");
      else System.setProperty("hara.hta.capabilities", previousCapabilities);
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
  public void importRejectsHostClassesAndWasmFlavorIsNotAllowed() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      PolyglotException hostImport =
          assertThrows(
              PolyglotException.class,
              () -> context.eval(HaraLanguage.ID, "(ns host-import (:import [java.lang String]))"));
      assertTrue(hostImport.getMessage().contains("native/import-missing"));

      PolyglotException wasmFlavor =
          assertThrows(
              PolyglotException.class,
              () -> context.eval(HaraLanguage.ID, "(ns wasm-flavor (:flavor :wasm))"));
      assertTrue(wasmFlavor.getMessage().contains("not a host flavor"));
    }
  }

  @Test
  public void installedPackageImportIsVerifiedBeforeTheWasmNamespaceIsExposed() throws Exception {
    Path dist = Files.createTempDirectory("hara-package-dist-");
    Path root = dist.resolve("roots/sha256/demo");
    Files.createDirectories(root);
    HaraExtensionTestProject.write(
        root,
        "{:namespace \"demo.000-answer-42\" :version \"1.0.0\" :provider :wasm "
            + ":module \"answer-42.wasm\" :abi :core.v1 "
            + ":exports {\"add\" {:args [:i32 :i32] :returns :i32 :async true}} :capabilities []}");
    Files.write(root.resolve("answer-42.wasm"), ADD_WASM);
    String project = Files.readString(root.resolve("project.edn"));
    String packageManifest =
        "{:harp/format \"0.0.0-alpha\" :package {:identity \"test/demo.000-answer-42\" :version \"1.0.0\"} "
            + ":files {\"project.edn\" {:sha256 \""
            + digest(project.getBytes())
            + "\" :size "
            + project.getBytes().length
            + "} \"answer-42.wasm\" {:sha256 \""
            + digest(ADD_WASM)
            + "\" :size "
            + ADD_WASM.length
            + "}} :wasm-imports {:demo.000-answer-42 {:variant/artifact {:artifact/type :wasm "
            + ":artifact/path \"answer-42.wasm\" :artifact/sha256 \""
            + digest(ADD_WASM)
            + "\" :artifact/target \"wasm32-wasi-preview1\" :artifact/abi \"core.v1\" :artifact/entry-point \"add\"} "
            + ":variant/required-capabilities #{} :variant/host-calls #{} :variant/exports #{:add}}}}";
    Files.writeString(root.resolve("package.edn"), packageManifest);
    String previous = System.getProperty("hara.dist.home");
    System.setProperty("hara.dist.home", dist.toString());
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      assertEquals(
          42,
          context
              .eval(
                  HaraLanguage.ID,
                  "(ns packaged (:import demo.000-answer-42)) "
                      + "(deref (demo.000-answer-42/add 20 22))")
              .asLong());
    } finally {
      if (previous == null) System.clearProperty("hara.dist.home");
      else System.setProperty("hara.dist.home", previous);
      try (var paths = Files.walk(dist)) {
        paths.sorted(java.util.Comparator.reverseOrder()).forEach(path -> {
          try { Files.deleteIfExists(path); } catch (Exception ignored) { }
        });
      }
    }
  }

  private static String digest(byte[] bytes) throws Exception {
    return "sha256:" + HexFormat.of().formatHex(MessageDigest.getInstance("SHA-256").digest(bytes));
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
