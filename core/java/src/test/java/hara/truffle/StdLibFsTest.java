package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.LinkOption;
import java.nio.file.Path;
import org.graalvm.polyglot.Context;
import org.graalvm.polyglot.io.IOAccess;
import org.junit.Test;

public class StdLibFsTest {
  @Test
  public void nativeEffectsReturnPromisesAndStructuredCapabilityFailures() {
    try (Context context = Context.newBuilder(HaraLanguage.ID).build()) {
      assertTrue(context.eval(HaraLanguage.ID, "(promise? (File/read \"/missing\"))").asBoolean());
      assertEquals(
          ":native/capability-denied",
          context
              .eval(
                  HaraLanguage.ID,
                  "(try (deref (File/read \"/missing\"))"
                      + "     :unexpected"
                      + "     (catch Throwable error"
                      + "       (get (ex-data error) :ex/code)))")
              .toString());
    }
  }

  @Test
  public void portableFsIsDirectStyleAndSupportsRecursiveOperations() throws Exception {
    Path root = Files.createTempDirectory("hara-std-fs-");
    try (Context context = newContext(root)) {
      assertEquals(
          "[\"/alpha.bin\" \"/alpha.bin\" \"alpha.bin\" :file 2 true false]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(do (require 'std.fs)"
                      + "    (let [written (std.fs/write-bytes \"/alpha.bin\" (bytes 1 2))"
                      + "          entry (std.fs/stat \"/alpha.bin\")]"
                      + "      [written (:path entry) (:name entry) (:type entry)"
                      + "       (:size entry) (map? (:extensions entry))"
                      + "       (promise? written)]))")
              .toString());

      assertEquals(
          ":file/already-exists",
          context
              .eval(
                  HaraLanguage.ID,
                  "(try (std.fs/write-bytes \"/alpha.bin\" (bytes 9))"
                      + "     :unexpected"
                      + "     (catch Throwable error"
                      + "       (get (ex-data error) :ex/code)))")
              .toString());

      assertEquals(
          "/simple-delete",
          context
              .eval(
                  HaraLanguage.ID,
                  "(do (std.fs/write-bytes \"/simple-delete\" (bytes 1))"
                      + "    (std.fs/delete \"/simple-delete\"))")
              .toString());

      assertEquals(
          "[\"/native-missing-ok\" true true]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(do (require 'std.fs)"
                      + "    [(deref (File/delete \"/native-missing-ok\""
                      + "                         {:missing-ok? true}))"
                      + "     (:missing-ok?"
                      + "      (merge std.fs/delete-default-options"
                      + "             {:missing-ok? true}))"
                      + "     (:missing-ok? {:missing-ok? true})])")
              .toString());

      assertEquals(
          "/missing-ok",
          context
              .eval(
                  HaraLanguage.ID,
                  "(std.fs/delete \"/missing-ok\" {:missing-ok? true})")
              .toString());

      assertEquals(
          "[\"one\" \"two\" false false]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(do (std.fs/create-directory \"/src/sub\" {:parents? true})"
                      + "    (std.fs/write-bytes \"/src/one\""
                      + "                        (std.foundation.string/encode-utf8 \"one\"))"
                      + "    (std.fs/write-bytes \"/src/sub/two\""
                      + "                        (std.foundation.string/encode-utf8 \"two\"))"
                      + "    (let [mapping (std.fs/copy \"/src\" \"/dst\")"
                      + "          bytes (std.fs/read-bytes \"/dst/one\")]"
                      + "      [(std.foundation.string/decode-utf8 bytes)"
                      + "       (std.foundation.string/decode-utf8"
                      + "        (std.fs/read-bytes \"/dst/sub/two\"))"
                      + "       (promise? mapping)"
                      + "       (promise? bytes)]))")
              .toString());

      assertEquals(
          "[\"/src/one\" \"/src/sub/two\" \"/src/sub\" \"/src\"]",
          context
              .eval(
                  HaraLanguage.ID,
                  "(std.fs/delete \"/src\" {:recursive? true})")
              .toString());
      assertFalse(
          context
              .eval(HaraLanguage.ID, "(std.fs/exists? \"/src\")")
              .asBoolean());
    } finally {
      deleteTree(root);
    }
  }

  @Test
  public void entriesAreSortedAndTraversalDoesNotFollowSymlinks() throws Exception {
    Path root = Files.createTempDirectory("hara-std-fs-walk-");
    Path outside = Files.createTempDirectory("hara-std-fs-outside-");
    try {
      Files.writeString(outside.resolve("secret"), "secret");
      try {
        Files.createSymbolicLink(root.resolve("link"), outside);
      } catch (UnsupportedOperationException | IOException | SecurityException ignored) {
        // The ordering assertions remain valid on filesystems without symlink support.
      }
      try (Context context = newContext(root)) {
        assertEquals(
            "[\"/a\" \"/b\"]",
            context
                .eval(
                    HaraLanguage.ID,
                    "(do (require 'std.fs)"
                        + "    (std.fs/write-bytes \"/b\" (bytes 2))"
                        + "    (std.fs/write-bytes \"/a\" (bytes 1))"
                        + "    (vec (filter (fn [path]"
                        + "                   (or (= path \"/a\") (= path \"/b\")))"
                        + "                 (std.fs/list \"/\"))))")
                .toString());
        assertEquals(
            "[\"/a\" \"/b\"]",
            context
                .eval(
                    HaraLanguage.ID,
                    "(vec (map :path"
                        + "          (filter (fn [entry]"
                        + "                    (or (= (:path entry) \"/a\")"
                        + "                        (= (:path entry) \"/b\")))"
                        + "                  (std.fs.walk/walk \"/\"))))")
                .toString());
        if (Files.isSymbolicLink(root.resolve("link"))) {
          assertEquals(
              "[[:symlink \"/link\"]]",
              context
                  .eval(
                      HaraLanguage.ID,
                      "(vec (map (fn [entry] [(:type entry) (:path entry)])"
                          + "          (filter (fn [entry] (= (:path entry) \"/link\"))"
                          + "                  (std.fs.walk/walk \"/\"))))")
                  .toString());
        }
      }
    } finally {
      deleteTree(root);
      deleteTree(outside);
    }
  }

  private static Context newContext(Path root) {
    IOAccess io = IOAccess.newBuilder().fileSystem(new HaraMountedFileSystem(root)).build();
    return Context.newBuilder(HaraLanguage.ID).allowIO(io).build();
  }

  private static void deleteTree(Path root) throws IOException {
    if (root == null || !Files.exists(root, LinkOption.NOFOLLOW_LINKS)) return;
    try (var paths = Files.walk(root)) {
      for (Path path : paths.sorted(java.util.Comparator.reverseOrder()).toList()) {
        Files.deleteIfExists(path);
      }
    }
  }
}
