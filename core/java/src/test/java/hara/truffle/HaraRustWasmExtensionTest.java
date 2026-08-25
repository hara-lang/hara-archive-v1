package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assume.assumeTrue;

import java.nio.file.Files;
import java.nio.file.Path;
import org.graalvm.polyglot.Context;
import org.junit.Test;

/** End-to-end check that the raw Rust target is loadable through Hara's WASM extension contract. */
public class HaraRustWasmExtensionTest {
  @Test
  public void rawRustCoreCompilesAndLoadsThroughTruffle() throws Exception {
    Path artifact =
        Path.of("rust/raw/target/wasm32-unknown-unknown/browser-release/hara-wasm-vm.wasm")
            .toAbsolutePath();
    assumeTrue(
        "build rust/raw before running this integration test: " + artifact,
        Files.isRegularFile(artifact));
    Path root = Files.createTempDirectory("hara-rust-wasm-extension-");
    Path extension = root.resolve("test/rust/core");
    Files.createDirectories(extension);
    HaraExtensionTestProject.write(
        extension,
        "{:namespace \"test.rust.core\" :version \"0.1.0\" :provider :wasm "
            + ":module \"hara.wasm\" :abi :core.v1 "
            + ":exports {\"add\" {:args [:i32 :i32] :returns :i32} "
            + "\"version\" {:args [] :returns :i32} "
            + "\"eval_i64\" {:args [:utf8] :returns :i64} "
            + "\"eval_error_code\" {:args [:utf8] :returns :i32}} :capabilities []}");
    Files.copy(artifact, extension.resolve("hara.wasm"));
    String previous = System.getProperty("hara.extensions.path");
    System.setProperty("hara.extensions.path", root.toString());
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      assertEquals(
          43,
          context
              .eval(
                  HaraLanguage.ID,
                  "(ns app (:require [test.rust.core :as rust :refer [add version]])) "
                      + "(+ (rust/add 20 22) (version))")
              .asLong());
      assertEquals(42, context.eval(HaraLanguage.ID, "(rust/eval_i64 \"(+ 20 22)\")").asLong());
      String iteratorSource = "(iter-next (Iter/iter-map (fn [x] (+ x 1)) [0]))";
      assertEquals(
          0,
          context
              .eval(HaraLanguage.ID, "(rust/eval_error_code " + quote(iteratorSource) + ")")
              .asLong());
      assertEquals(
          1,
          context.eval(HaraLanguage.ID, "(rust/eval_i64 " + quote(iteratorSource) + ")").asLong());
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

  private static String quote(String value) {
    return "\"" + value.replace("\\", "\\\\").replace("\"", "\\\"") + "\"";
  }
}
