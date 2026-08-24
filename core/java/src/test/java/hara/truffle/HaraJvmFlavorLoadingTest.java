package hara.truffle;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertThrows;
import static org.junit.Assert.assertTrue;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.security.MessageDigest;
import java.util.Comparator;
import java.util.HexFormat;
import java.util.jar.JarEntry;
import java.util.jar.JarOutputStream;
import javax.tools.JavaCompiler;
import javax.tools.ToolProvider;
import org.graalvm.polyglot.Context;
import org.graalvm.polyglot.HostAccess;
import org.graalvm.polyglot.PolyglotException;
import org.junit.Test;

public class HaraJvmFlavorLoadingTest {
  @Test
  public void resolvesDirectAndPackageVectorImports() {
    try (Context context = context()) {
      context.eval(
          HaraLanguage.ID,
          "(ns jvm-loading (:flavor :jvm java.lang.String [java.lang RuntimeException] [java.awt Point]))");
      assertEquals("42", context.eval(HaraLanguage.ID, "(String/valueOf 42)").asString());
      assertEquals(
          "3", context.eval(HaraLanguage.ID, "(. (new Point 3 4) x (toString))").asString());
      assertEquals(
          "java.lang.RuntimeException",
          context.eval(HaraLanguage.ID, "(hara.native.jvm.reflect/name RuntimeException)").asString());
    }
  }

  @Test
  public void unresolvedImportsRollBackPartialResolution() {
    try (Context context = context()) {
      context.eval(HaraLanguage.ID, "(ns jvm-loading (:flavor :jvm [java.lang String]))");
      PolyglotException error =
          assertThrows(
              PolyglotException.class,
              () ->
                  context.eval(
                      HaraLanguage.ID,
                      "(ns jvm-loading (:flavor :jvm [java.lang String MissingType]))"));
      assertTrue(error.getMessage().contains("JVM class not found: java.lang.MissingType"));
      assertEquals("42", context.eval(HaraLanguage.ID, "(String/valueOf 42)").asString());
    }
  }

  @Test
  public void reloadingAFlavorReplacesThePreviousImports() {
    try (Context context = context()) {
      context.eval(HaraLanguage.ID, "(ns jvm-loading (:flavor :jvm [java.util Date]))");
      context.eval(HaraLanguage.ID, "(ns jvm-loading (:flavor :jvm [java.awt Point]))");
      assertEquals(
          "java.awt.Point",
          context.eval(HaraLanguage.ID, "(hara.native.jvm.reflect/name Point)").asString());
      PolyglotException removed =
          assertThrows(
              PolyglotException.class,
              () -> context.eval(HaraLanguage.ID, "(hara.native.jvm.reflect/name Date)"));
      assertTrue(removed.getMessage().contains("Unbound symbol: Date"));
    }
  }

  @Test
  public void rejectsAmbiguousAndMalformedImportsBeforeChangingState() {
    try (Context context = context()) {
      context.eval(HaraLanguage.ID, "(ns jvm-loading (:flavor :jvm [java.lang String]))");
      PolyglotException collision =
          assertThrows(
              PolyglotException.class,
              () ->
                  context.eval(
                      HaraLanguage.ID,
                      "(ns jvm-loading (:flavor :jvm [java.util Date] [java.sql Date]))"));
      assertTrue(collision.getMessage().contains("Native import already exists: Date"));
      assertEquals("42", context.eval(HaraLanguage.ID, "(String/valueOf 42)").asString());

      PolyglotException malformed =
          assertThrows(
              PolyglotException.class,
              () -> context.eval(HaraLanguage.ID, "(ns malformed (:flavor :jvm []))"));
      assertTrue(
          malformed
              .getMessage()
              .contains(":flavor package vector requires a package and at least one class"));
    }
  }

  @Test
  public void resolvesTypesFromTheInstalledJvmFlavorLoader() throws Exception {
    Path root = Files.createTempDirectory("hara-jvm-flavor-root-");
    Path dist = Files.createTempDirectory("hara-jvm-flavor-dist-");
    String previousDist = System.getProperty("hara.dist.home");
    try {
      Path jar = buildTypeJar(root);
      byte[] bytes = Files.readAllBytes(jar);
      String digest =
          "sha256:"
              + HexFormat.of()
                  .formatHex(MessageDigest.getInstance("SHA-256").digest(bytes));
      Path packageRoot = dist.resolve("roots/sha256/fixture");
      Files.createDirectories(packageRoot.resolve("artifacts/jvm"));
      Files.copy(jar, packageRoot.resolve("artifacts/jvm/provider.jar"));
      Files.writeString(
          packageRoot.resolve("package.edn"),
          "{:harp/format \"0.0.0-alpha\" :package {:identity \"fixture/package\" "
              + ":version \"1.0.0\" :provenance {:repository \"https://example.test/fixture\" "
              + ":commit \"0123456789abcdef0123456789abcdef01234567\"}} "
              + ":files {\"artifacts/jvm/provider.jar\" {:sha256 \""
              + digest
              + "\" :size "
              + bytes.length
              + "}} :flavors {:jvm {:variant/artifact {:artifact/type :jar "
              + ":artifact/path \"artifacts/jvm/provider.jar\" :artifact/sha256 \""
              + digest
              + "\" :artifact/target \"java-21\" :artifact/abi \"hara.provider.jvm.v1\" "
              + ":artifact/entry-point \"fixture.PackageType\"} :variant/required-capabilities #{}}}}");
      System.setProperty("hara.dist.home", dist.toString());
      try (Context context = context()) {
        context.eval(
            HaraLanguage.ID,
            "(ns installed-jvm (:flavor :jvm [fixture PackageType]))");
        assertEquals(
            "package-loader",
            context.eval(HaraLanguage.ID, "(PackageType/value)").asString());
      }
    } finally {
      if (previousDist == null) System.clearProperty("hara.dist.home");
      else System.setProperty("hara.dist.home", previousDist);
      deleteTree(dist);
      deleteTree(root);
    }
  }

  private static Path buildTypeJar(Path root) throws Exception {
    Path source = root.resolve("PackageType.java");
    Path classes = root.resolve("classes");
    Files.createDirectories(classes);
    Files.writeString(
        source,
        "package fixture; public final class PackageType { "
            + "public static String value() { return \"package-loader\"; } }");
    JavaCompiler compiler = ToolProvider.getSystemJavaCompiler();
    assertTrue("JDK compiler is required for the package flavor fixture", compiler != null);
    assertEquals(
        0,
        compiler.run(
            null,
            null,
            null,
            "--release",
            "21",
            "-d",
            classes.toString(),
            source.toString()));
    Path jar = root.resolve("provider.jar");
    try (JarOutputStream output = new JarOutputStream(Files.newOutputStream(jar))) {
      Path classFile = classes.resolve("fixture/PackageType.class");
      output.putNextEntry(new JarEntry("fixture/PackageType.class"));
      output.write(Files.readAllBytes(classFile));
      output.closeEntry();
    }
    return jar;
  }

  private static void deleteTree(Path root) throws IOException {
    if (!Files.exists(root)) return;
    try (var paths = Files.walk(root)) {
      for (Path path : paths.sorted(Comparator.reverseOrder()).toList()) {
        Files.deleteIfExists(path);
      }
    }
  }

  private static Context context() {
    return Context.newBuilder(HaraLanguage.ID)
        .allowHostAccess(HostAccess.ALL)
        .allowHostClassLookup(name -> true)
        .build();
  }
}
